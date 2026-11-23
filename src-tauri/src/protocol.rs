//! `sonar://` URL scheme: serves OS file icons and image thumbnails to the
//! webview, so `<img src>` can load them lazily and the browser caches them.
//!
//! - `sonar://localhost/icon?key=<icon key>&size=<px>`
//! - `sonar://localhost/thumb?path=<absolute path>&size=<px>`

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use sonar_core::platform::{self, Icon};
use tauri::http::{header, Request, Response, StatusCode};
use tauri::{Runtime, UriSchemeContext, UriSchemeResponder};

pub const SCHEME: &str = "sonar";

pub fn handle<R: Runtime>(_ctx: UriSchemeContext<'_, R>, request: Request<Vec<u8>>, responder: UriSchemeResponder) {
    // Icon rendering and image decoding are slow; keep them off the UI thread.
    tauri::async_runtime::spawn_blocking(move || responder.respond(respond(&request)));
}

fn respond(request: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let uri = request.uri();
    let q = query(uri.query().unwrap_or(""));
    let size = q.get("size").and_then(|s| s.parse().ok()).unwrap_or(32u32).clamp(8, 512);
    match uri.path() {
        "/icon" => match q.get("key").and_then(|k| icon(k, size)) {
            Some(icon) => ok(icon.content_type, icon.bytes.clone(), "max-age=86400"),
            None => not_found(),
        },
        "/thumb" => {
            match q.get("path").map(|p| sonar_core::thumbnail::thumbnail(sonar_core::uipath::decode(p).as_ref(), size))
            {
                // Thumbnails change when the file does; let the browser revalidate.
                Some(Ok(png)) => ok("image/png", png, "no-cache"),
                _ => not_found(),
            }
        }
        _ => not_found(),
    }
}

/// Icons are shared by many files of the same type: render each key/size once.
fn icon(key: &str, size: u32) -> Option<Arc<Icon>> {
    type IconCache = Mutex<HashMap<(String, u32), Option<Arc<Icon>>>>;
    static CACHE: OnceLock<IconCache> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let k = (key.to_string(), size);
    if let Some(hit) = cache.lock().unwrap_or_else(|p| p.into_inner()).get(&k) {
        return hit.clone();
    }
    let rendered = platform::icon(key, size).map(Arc::new);
    cache.lock().unwrap_or_else(|p| p.into_inner()).insert(k, rendered.clone());
    rendered
}

fn ok(content_type: &str, body: Vec<u8>, cache: &str) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, cache)
        .body(Cow::Owned(body))
        .expect("static response parts are valid")
}

fn not_found() -> Response<Cow<'static, [u8]>> {
    Response::builder().status(StatusCode::NOT_FOUND).body(Cow::Borrowed(&[][..])).expect("valid response")
}

/// Parse `a=1&b=%2Fx` into decoded pairs (`encodeURIComponent` on the JS side).
fn query(q: &str) -> HashMap<String, String> {
    q.split('&').filter_map(|pair| pair.split_once('=')).map(|(k, v)| (percent_decode(k), percent_decode(v))).collect()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |j: usize| (b[j] as char).to_digit(16);
        match b[i] {
            b'%' if i + 2 < b.len() => match (hex(i + 1), hex(i + 2)) {
                (Some(h), Some(l)) => {
                    out.push((h * 16 + l) as u8);
                    i += 3;
                }
                _ => {
                    out.push(b'%');
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_queries() {
        let q = query("path=%2Fhome%2Fme%2Fmy%20photo%20%C3%BC.png&size=128&bad=%zz%");
        assert_eq!(q["path"], "/home/me/my photo ü.png");
        assert_eq!(q["size"], "128");
        assert_eq!(q["bad"], "%zz%");
    }

    #[test]
    fn routes() {
        let req = |uri: &str| Request::builder().uri(uri).body(Vec::new()).unwrap();
        assert_eq!(respond(&req("sonar://localhost/nope")).status(), StatusCode::NOT_FOUND);
        assert_eq!(
            respond(&req("sonar://localhost/thumb?path=%2Fdoes%2Fnot%2Fexist.png")).status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(respond(&req("sonar://localhost/icon?key=bogus-key")).status(), StatusCode::NOT_FOUND);
        #[cfg(target_os = "macos")]
        {
            let res = respond(&req("sonar://localhost/icon?key=uti%3Apublic.png&size=16"));
            assert_eq!(res.status(), StatusCode::OK);
            assert_eq!(res.headers()[header::CONTENT_TYPE], "image/png");
            assert_eq!(res.headers()[header::CACHE_CONTROL], "max-age=86400");
        }
    }
}
