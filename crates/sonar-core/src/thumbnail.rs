//! Image thumbnails, cached per the freedesktop.org Thumbnail Managing
//! Standard: `<cache>/thumbnails/{normal,large,x-large}/<md5(uri)>.png`,
//! with `Thumb::URI` and `Thumb::MTime` text chunks to detect stale entries.
//!
//! On Linux the cache is the shared `~/.cache/thumbnails`, so thumbnails made
//! by Dolphin or Nautilus are reused and vice versa. On macOS Sonar keeps its
//! own cache under `~/Library/Caches/sonar/thumbnails` in the same format.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use md5::{Digest, Md5};

use crate::clipboard::path_to_uri;
use crate::error::{CoreError, Result};

/// Larger files are not worth decoding for a thumbnail.
const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];

/// Whether [`thumbnail`] can handle this file (by name).
pub fn supported(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(stem, ext)| !stem.is_empty() && EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// Freedesktop size buckets.
fn bucket(size: u32) -> (&'static str, u32) {
    match size {
        0..=128 => ("normal", 128),
        129..=256 => ("large", 256),
        _ => ("x-large", 512),
    }
}

pub fn cache_root() -> Option<PathBuf> {
    let base = dirs::cache_dir()?;
    Some(if cfg!(target_os = "linux") { base.join("thumbnails") } else { base.join("sonar").join("thumbnails") })
}

/// A PNG thumbnail of the image at `path`, at least `size` pixels on its
/// longer side (rounded up to the next cache bucket). Served from the cache
/// when it is still fresh.
pub fn thumbnail(path: &Path, size: u32) -> Result<Vec<u8>> {
    thumbnail_in(cache_root().as_deref(), path, size)
}

fn thumbnail_in(cache: Option<&Path>, path: &Path, size: u32) -> Result<Vec<u8>> {
    let meta = fs::metadata(path).map_err(|e| CoreError::from_io(path, e))?;
    if !meta.is_file() || meta.len() > MAX_SOURCE_BYTES {
        return Err(CoreError::Other(format!("No thumbnail for {}", path.display())));
    }
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    let absolute = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let uri = path_to_uri(&absolute);
    let (bucket_dir, px) = bucket(size);
    let cached = cache.map(|c| c.join(bucket_dir).join(format!("{}.png", md5_hex(&uri))));

    if let Some(file) = &cached
        && let Ok(bytes) = fs::read(file)
        && is_fresh(&bytes, &uri, mtime)
    {
        return Ok(bytes);
    }

    let img = image::ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| CoreError::from_io(path, e))?
        .decode()
        .map_err(|e| CoreError::Other(format!("Could not read image {}: {e}", path.display())))?;
    // Never upscale: small images are their own thumbnail.
    let thumb = if img.width() > px || img.height() > px { img.thumbnail(px, px) } else { img };
    let bytes = encode_png(&thumb.to_rgba8(), &uri, mtime)?;

    if let Some(file) = cached {
        let _ = write_cache(&file, &bytes); // a read-only cache is not an error
    }
    Ok(bytes)
}

/// Lower-case hex MD5, as the thumbnail spec names cache files.
fn md5_hex(s: &str) -> String {
    Md5::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn encode_png(rgba: &image::RgbaImage, uri: &str, mtime: u64) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(Cursor::new(&mut out), rgba.width(), rgba.height());
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let err = |e: png::EncodingError| CoreError::Other(format!("Could not encode thumbnail: {e}"));
    enc.add_text_chunk("Thumb::URI".into(), uri.into()).map_err(err)?;
    enc.add_text_chunk("Thumb::MTime".into(), mtime.to_string()).map_err(err)?;
    enc.add_text_chunk("Software".into(), "Sonar".into()).map_err(err)?;
    let mut writer = enc.write_header().map_err(err)?;
    writer.write_image_data(rgba.as_raw()).map_err(err)?;
    writer.finish().map_err(err)?;
    Ok(out)
}

/// A cached thumbnail is valid if it was made for this URI and the source's
/// modification time hasn't changed since.
fn is_fresh(png_bytes: &[u8], uri: &str, mtime: u64) -> bool {
    let Ok(reader) = png::Decoder::new(Cursor::new(png_bytes)).read_info() else { return false };
    let text =
        |key: &str| reader.info().uncompressed_latin1_text.iter().find(|t| t.keyword == key).map(|t| t.text.clone());
    text("Thumb::URI").as_deref() == Some(uri)
        && text("Thumb::MTime").and_then(|m| m.parse::<u64>().ok()) == Some(mtime)
}

fn write_cache(file: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = file.parent().expect("cache files live in a bucket dir");
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.tmp", std::process::id()));
    fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // The spec asks for private thumbnails: they reveal what the user has.
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&tmp, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_png(path: &Path, w: u32, h: u32) {
        image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255])).save(path).unwrap();
    }

    #[test]
    fn supported_names() {
        assert!(supported("a.PNG") && supported("x.jpeg"));
        assert!(!supported("a.txt") && !supported(".png") && !supported("png"));
    }

    #[test]
    fn makes_caches_and_refreshes() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("cache");
        let img = dir.path().join("big photo.png");
        write_png(&img, 1000, 500);

        let bytes = thumbnail_in(Some(&cache), &img, 100).unwrap();
        let t = image::load_from_memory(&bytes).unwrap();
        assert_eq!((t.width(), t.height()), (128, 64), "fits the 128px bucket, keeps aspect ratio");

        // Cached under the freedesktop name, with the metadata other apps check.
        let uri = path_to_uri(&img.canonicalize().unwrap());
        let file = cache.join("normal").join(format!("{}.png", md5_hex(&uri)));
        let cached = fs::read(&file).unwrap();
        assert_eq!(cached, bytes);
        let mtime = fs::metadata(&img).unwrap().modified().unwrap().duration_since(UNIX_EPOCH).unwrap().as_secs();
        assert!(is_fresh(&cached, &uri, mtime));
        assert!(!is_fresh(&cached, &uri, mtime + 1));

        // A stale cache entry is replaced.
        fs::write(&file, encode_png(&image::RgbaImage::new(1, 1), &uri, 1).unwrap()).unwrap();
        let again = thumbnail_in(Some(&cache), &img, 100).unwrap();
        assert_eq!(image::load_from_memory(&again).unwrap().width(), 128);

        // Larger buckets, and small images are not upscaled.
        assert_eq!(image::load_from_memory(&thumbnail_in(Some(&cache), &img, 200).unwrap()).unwrap().width(), 256);
        let small = dir.path().join("small.png");
        write_png(&small, 40, 30);
        assert_eq!(image::load_from_memory(&thumbnail_in(None, &small, 256).unwrap()).unwrap().width(), 40);
    }

    #[test]
    fn errors() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("not-really.png");
        fs::write(&fake, "hello").unwrap();
        assert!(thumbnail_in(None, &fake, 128).is_err());
        assert!(thumbnail_in(None, &dir.path().join("missing.png"), 128).is_err());
    }
}
