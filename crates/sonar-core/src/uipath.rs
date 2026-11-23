//! Paths and names as text for the UI, without losing any bytes.
//!
//! Linux file names are bytes, not necessarily UTF-8 (Latin-1 names from old
//! drives, ZIPs, ...), but the UI talks JSON, which only carries Unicode. So
//! paths cross that boundary encoded: valid UTF-8 as itself, and every other
//! byte as one character from the last 256 code points of Unicode (private
//! use, U+10FF00 + byte). Characters from that range in real names are escaped
//! the same way (byte by byte), so decoding always gives back the original.
//! The UI shows escaped bytes as U+FFFD and otherwise treats paths as opaque
//! strings (joining a folder path and an entry name still works).
//!
//! On macOS names are always UTF-8 and nothing changes.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

const BASE: u32 = 0x10FF00;

fn escaped(byte: u8) -> char {
    char::from_u32(BASE + u32::from(byte)).expect("U+10FF00..=U+10FFFF are valid chars")
}

/// `s` as text for the UI.
pub fn encode(s: &OsStr) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let bytes = s.as_bytes();
        if let Ok(text) = std::str::from_utf8(bytes)
            && !text.chars().any(|c| u32::from(c) >= BASE)
        {
            return text.to_string();
        }
        let mut out = String::with_capacity(bytes.len());
        for chunk in bytes.utf8_chunks() {
            for c in chunk.valid().chars() {
                if u32::from(c) >= BASE {
                    out.extend(c.encode_utf8(&mut [0; 4]).bytes().map(escaped));
                } else {
                    out.push(c);
                }
            }
            out.extend(chunk.invalid().iter().copied().map(escaped));
        }
        out
    }
    #[cfg(not(unix))]
    {
        s.to_string_lossy().into_owned()
    }
}

/// The original name or path from text made by [`encode`] (or typed by the user).
pub fn decode(s: &str) -> OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        if !s.chars().any(|c| u32::from(c) >= BASE) {
            return s.into();
        }
        let mut bytes = Vec::with_capacity(s.len());
        for c in s.chars() {
            match u32::from(c).checked_sub(BASE) {
                Some(b) => bytes.push(b as u8),
                None => bytes.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
            }
        }
        OsString::from_vec(bytes)
    }
    #[cfg(not(unix))]
    {
        s.into()
    }
}

/// A file name for the UI: the last component of `path`, encoded.
pub fn file_name(path: &Path) -> String {
    path.file_name().map(encode).unwrap_or_default()
}

/// A path received from the UI, for command parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiPath(pub PathBuf);

impl std::ops::Deref for UiPath {
    type Target = PathBuf;
    fn deref(&self) -> &PathBuf {
        &self.0
    }
}

impl From<PathBuf> for UiPath {
    fn from(p: PathBuf) -> UiPath {
        UiPath(p)
    }
}

impl From<UiPath> for PathBuf {
    fn from(p: UiPath) -> PathBuf {
        p.0
    }
}

impl<'de> Deserialize<'de> for UiPath {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(UiPath(decode(&String::deserialize(d)?).into()))
    }
}

impl Serialize for UiPath {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&encode(self.0.as_os_str()))
    }
}

/// Paths from the UI, as plain `PathBuf`s.
pub fn paths(list: Vec<UiPath>) -> Vec<PathBuf> {
    list.into_iter().map(|p| p.0).collect()
}

/// `#[serde(with = "uipath::path")]` for a `PathBuf` field.
pub mod path {
    use super::*;

    pub fn serialize<S: Serializer>(p: &Path, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&encode(p.as_os_str()))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<PathBuf, D::Error> {
        Ok(decode(&String::deserialize(d)?).into())
    }
}

/// `#[serde(with = "uipath::opt")]` for an `Option<PathBuf>` field.
pub mod opt {
    use super::*;

    pub fn serialize<S: Serializer>(p: &Option<PathBuf>, s: S) -> Result<S::Ok, S::Error> {
        p.as_ref().map(|p| encode(p.as_os_str())).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<PathBuf>, D::Error> {
        Ok(Option::<String>::deserialize(d)?.map(|s| decode(&s).into()))
    }
}

/// `#[serde(with = "uipath::vec")]` for a `Vec<PathBuf>` field.
pub mod vec {
    use super::*;

    pub fn serialize<S: Serializer>(list: &[PathBuf], s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(list.iter().map(|p| encode(p.as_os_str())))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<PathBuf>, D::Error> {
        Ok(Vec::<String>::deserialize(d)?.into_iter().map(|s| decode(&s).into()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_is_unchanged() {
        for s in ["/home/me/notes.txt", "émoji 🎉.png", "", "line\nbreak", "\u{10FEFF}"] {
            assert_eq!(encode(OsStr::new(s)), s);
            assert_eq!(decode(s), OsString::from(s));
        }
    }

    #[cfg(unix)]
    #[test]
    fn any_bytes_round_trip() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let cases: Vec<Vec<u8>> = vec![
            b"latin1-\xe9t\xe9.txt".to_vec(),
            vec![0xff, 0xfe, b'/', 0x80],
            "real \u{10FF41} private-use".as_bytes().to_vec(),
            "\u{10FFFF}".as_bytes().to_vec(),
            // An escape character's own bytes, invalid on their own.
            vec![0xf4, 0x8f, 0xbc],
            (0..=255).collect(),
        ];
        for bytes in cases {
            let os = OsString::from_vec(bytes.clone());
            let text = encode(&os);
            assert_eq!(decode(&text).into_vec(), bytes, "{text:?}");
            // Through JSON, as the UI sends it back.
            let json = serde_json::to_string(&text).unwrap();
            let back: UiPath = serde_json::from_str(&json).unwrap();
            assert_eq!(back.0.as_os_str().as_bytes(), &bytes[..]);
        }
        assert_eq!(encode(OsStr::from_bytes(b"a\xe9")), "a\u{10FFE9}");
    }

    #[cfg(unix)]
    #[test]
    fn serde_helpers() {
        use std::os::unix::ffi::OsStringExt;
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct S {
            #[serde(with = "super::path")]
            p: PathBuf,
            #[serde(with = "super::opt")]
            o: Option<PathBuf>,
            #[serde(with = "super::vec")]
            v: Vec<PathBuf>,
        }
        let bad = PathBuf::from(OsString::from_vec(b"/tmp/\xe9".to_vec()));
        let s = S { p: bad.clone(), o: Some(bad.clone()), v: vec![bad.clone(), "/ok".into()] };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<S>(&json).unwrap(), s);
        let none = S { p: "/".into(), o: None, v: vec![] };
        assert_eq!(serde_json::from_str::<S>(&serde_json::to_string(&none).unwrap()).unwrap(), none);
    }
}
