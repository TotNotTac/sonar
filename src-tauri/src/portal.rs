//! Sonar as the file chooser of other apps (Linux).
//!
//! Firefox, Chromium, Flatpak apps and most GTK/Qt apps ask the XDG Desktop
//! Portal for "Open" and "Save" dialogs, and the portal hands that to the
//! backend the desktop is configured to use. This implements such a backend
//! (`org.freedesktop.impl.portal.FileChooser`) on the session bus: each request
//! opens a picker window (the normal UI in picker mode) and answers with what
//! was chosen.
//!
//! The running Sonar owns the bus name; when Sonar isn't running, D-Bus starts
//! `sonar --portal`, which runs without a main window and stays around for a
//! while after the last picker closes. To use it, the portal config
//! (`~/.config/xdg-desktop-portal/portals.conf`) names it for the FileChooser:
//! `org.freedesktop.impl.portal.FileChooser=sonar`.
//!
//! Spec: <https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.FileChooser.html>

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use sonar_core::uipath;
use tokio::sync::oneshot;
use zbus::object_server::ObjectServer;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use crate::picker::{Choice, Filter, Picker, PickerAnswer, PickerRequest};

pub const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.sonar";
const OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";

/// Portal response codes.
const RESPONSE_OK: u32 = 0;
const RESPONSE_CANCELLED: u32 = 1;
const RESPONSE_FAILED: u32 = 2;

// ---------- Reading the request ----------

type Options = HashMap<String, OwnedValue>;

fn opt<T>(options: &Options, key: &str) -> Option<T>
where
    T: TryFrom<OwnedValue>,
{
    options.get(key)?.try_clone().ok()?.try_into().ok()
}

/// A byte-string path (`ay`, NUL-terminated).
fn path_opt(options: &Options, key: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let mut bytes: Vec<u8> = opt(options, key)?;
    while bytes.last() == Some(&0) {
        bytes.pop();
    }
    (!bytes.is_empty()).then(|| OsString::from_vec(bytes).into())
}

/// GTK-style mnemonics ("_Open") as plain text.
fn without_mnemonic(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                out.push('_');
                chars.next();
            }
            continue;
        }
        out.push(c);
    }
    out
}

pub fn parse_request(id: u64, save: bool, title: &str, options: &Options) -> PickerRequest {
    let filters: Vec<Filter> = opt::<Vec<(String, Vec<(u32, String)>)>>(options, "filters")
        .unwrap_or_default()
        .into_iter()
        .map(|(name, patterns)| Filter { name, patterns })
        .collect();
    let current = opt::<(String, Vec<(u32, String)>)>(options, "current_filter")
        .map(|(name, patterns)| Filter { name, patterns });
    let mut filters = filters;
    let current_filter = current.map(|c| match filters.iter().position(|f| *f == c) {
        Some(i) => i,
        None => {
            // A current filter that isn't in the list is offered too (the spec allows it).
            filters.push(c);
            filters.len() - 1
        }
    });
    let choices = opt::<Vec<(String, String, Vec<(String, String)>, String)>>(options, "choices")
        .unwrap_or_default()
        .into_iter()
        .map(|(id, label, options, selected)| Choice { id, label, options, selected })
        .collect();
    let current_file = save.then(|| path_opt(options, "current_file")).flatten();
    let folder = current_file
        .as_deref()
        .and_then(|f| f.parent().map(PathBuf::from))
        .or_else(|| path_opt(options, "current_folder"));
    let name = current_file
        .as_deref()
        .and_then(|f| f.file_name())
        .map(uipath::encode)
        .or_else(|| opt::<String>(options, "current_name"));
    PickerRequest {
        id,
        save,
        title: title.to_string(),
        accept_label: opt::<String>(options, "accept_label").map(|l| without_mnemonic(&l)),
        multiple: !save && opt(options, "multiple").unwrap_or(false),
        directory: !save && opt(options, "directory").unwrap_or(false),
        filters,
        current_filter,
        choices,
        folder,
        name,
    }
}

// ---------- The answer ----------

fn value<'a>(v: impl Into<Value<'a>>) -> OwnedValue {
    OwnedValue::try_from(v.into()).expect("plain data has no file descriptors")
}

pub fn results(request: &PickerRequest, answer: &PickerAnswer) -> HashMap<String, OwnedValue> {
    let uris: Vec<String> = answer.paths.iter().map(|p| sonar_core::clipboard::path_to_uri(p)).collect();
    let mut out = HashMap::from([("uris".to_string(), value(uris)), ("writable".to_string(), value(true))]);
    if !answer.choices.is_empty() {
        out.insert("choices".into(), value(answer.choices.clone()));
    }
    if let Some(f) = answer.filter.and_then(|i| request.filters.get(i)) {
        out.insert("current_filter".into(), value((f.name.clone(), f.patterns.clone())));
    }
    out
}

// ---------- The D-Bus objects ----------

pub struct FileChooser {
    picker: Arc<dyn Picker>,
    next_id: AtomicU64,
    /// When the last request finished (see [`crate::picker::Usage`]).
    last_used: Arc<Mutex<Option<Instant>>>,
}

impl FileChooser {
    pub fn new(picker: Arc<dyn Picker>, last_used: Arc<Mutex<Option<Instant>>>) -> Self {
        Self { picker, next_id: AtomicU64::new(1), last_used }
    }

    async fn ask(
        &self,
        server: &ObjectServer,
        handle: OwnedObjectPath,
        save: bool,
        title: &str,
        options: Options,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = parse_request(id, save, title, &options);
        let (reply, answer) = oneshot::channel();
        let (closed_tx, closed) = oneshot::channel();
        // The app can withdraw the request through the Request object at `handle`.
        let exported = server.at(&handle, Request { close: Mutex::new(Some(closed_tx)) }).await.unwrap_or(false);
        self.picker.show(request.clone(), reply);
        let outcome = tokio::select! {
            answer = answer => match answer {
                Ok(a) if a.ok => (RESPONSE_OK, results(&request, &a)),
                Ok(_) | Err(_) => (RESPONSE_CANCELLED, HashMap::new()),
            },
            _ = closed => {
                self.picker.close(id);
                (RESPONSE_CANCELLED, HashMap::new())
            }
        };
        if exported {
            let _ = server.remove::<Request, _>(&handle).await;
        }
        *self.last_used.lock().unwrap_or_else(|p| p.into_inner()) = Some(Instant::now());
        outcome
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FileChooser {
    async fn open_file(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        handle: OwnedObjectPath,
        _app_id: &str,
        _parent_window: &str,
        title: &str,
        options: Options,
    ) -> (u32, HashMap<String, OwnedValue>) {
        self.ask(server, handle, false, title, options).await
    }

    async fn save_file(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        handle: OwnedObjectPath,
        _app_id: &str,
        _parent_window: &str,
        title: &str,
        options: Options,
    ) -> (u32, HashMap<String, OwnedValue>) {
        self.ask(server, handle, true, title, options).await
    }

    /// Saving several files at once (into a folder) isn't offered.
    async fn save_files(
        &self,
        _handle: OwnedObjectPath,
        _app_id: &str,
        _parent_window: &str,
        _title: &str,
        _options: Options,
    ) -> (u32, HashMap<String, OwnedValue>) {
        (RESPONSE_FAILED, HashMap::new())
    }
}

/// `org.freedesktop.impl.portal.Request` at a request's handle path.
struct Request {
    close: Mutex<Option<oneshot::Sender<()>>>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl Request {
    fn close(&self) {
        if let Some(tx) = self.close.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = tx.send(());
        }
    }
}

/// Serve the portal on the session bus under [`BUS_NAME`]. The connection
/// lives as long as the returned value.
pub async fn serve(chooser: FileChooser) -> zbus::Result<zbus::Connection> {
    zbus::connection::Builder::session()?.name(BUS_NAME)?.serve_at(OBJECT_PATH, chooser)?.build().await
}

/// Keeps the bus connection (and so the name) for the life of the app.
struct Connection(#[allow(dead_code)] zbus::Connection);

/// Serve the portal for this app, and end a Sonar that only serves it once it
/// has no windows and has been idle for a while.
pub fn start(app: &tauri::AppHandle, portal_only: bool) {
    use tauri::Manager;
    let usage = app.state::<crate::picker::Usage>().shared();
    let chooser = FileChooser::new(Arc::new(crate::picker::TauriPicker(app.clone())), usage);
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        match serve(chooser).await {
            Ok(conn) => {
                handle.manage(Connection(conn));
            }
            Err(e) => {
                eprintln!("Sonar: file chooser portal unavailable: {e}");
                // A windowless portal instance has nothing to do but wait for the
                // name; if the main instance owns it, exit instead of idling.
                if portal_only {
                    handle.exit(2);
                }
            }
        }
    });
    let handle = app.clone();
    std::thread::Builder::new()
        .name("sonar-portal-idle".into())
        .spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(30));
                let usage = handle.state::<crate::picker::Usage>();
                if handle.webview_windows().is_empty() && !usage.keep_running() {
                    handle.exit(0);
                }
            }
        })
        .expect("failed to spawn the portal idle watcher");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;
    use zbus::zvariant::ObjectPath;

    fn options(pairs: Vec<(&str, Value<'_>)>) -> Options {
        pairs.into_iter().map(|(k, v)| (k.to_string(), OwnedValue::try_from(v).unwrap())).collect()
    }

    fn cstr(path: &[u8]) -> Value<'static> {
        let mut v = path.to_vec();
        v.push(0);
        Value::from(v)
    }

    #[test]
    fn parses_open_options() {
        let png = (1u32, "image/png".to_string());
        let opts = options(vec![
            ("accept_label", Value::from("_Upload")),
            ("multiple", Value::from(true)),
            (
                "filters",
                Value::from(vec![
                    ("Images".to_string(), vec![png.clone(), (0u32, "*.jpg".to_string())]),
                    ("All files".to_string(), vec![(0u32, "*".to_string())]),
                ]),
            ),
            ("current_filter", Value::from(("Images".to_string(), vec![png, (0u32, "*.jpg".to_string())]))),
            (
                "choices",
                Value::from(vec![(
                    "encoding".to_string(),
                    "Encoding".to_string(),
                    vec![("utf8".to_string(), "UTF-8".to_string())],
                    "utf8".to_string(),
                )]),
            ),
            ("current_folder", cstr(b"/home/me/caf\xe9")),
        ]);
        let r = parse_request(7, false, "Upload photos", &opts);
        assert_eq!(r.accept_label.as_deref(), Some("Upload"));
        assert!(r.multiple && !r.directory && !r.save);
        assert_eq!(r.filters.len(), 2);
        assert_eq!(r.current_filter, Some(0));
        assert_eq!(r.choices[0].id, "encoding");
        assert_eq!(r.folder.as_deref().unwrap().as_os_str().as_bytes(), b"/home/me/caf\xe9");
    }

    #[test]
    fn parses_save_options() {
        let opts = options(vec![("current_file", cstr(b"/home/me/Documents/report.pdf"))]);
        let r = parse_request(1, true, "", &opts);
        assert_eq!(r.folder, Some(PathBuf::from("/home/me/Documents")));
        assert_eq!(r.name.as_deref(), Some("report.pdf"));
        let opts = options(vec![("current_name", Value::from("Untitled.txt")), ("multiple", Value::from(true))]);
        let r = parse_request(2, true, "", &opts);
        assert_eq!(r.name.as_deref(), Some("Untitled.txt"));
        assert!(!r.multiple, "saving is always one file");
        assert_eq!(without_mnemonic("Save __as_ _x"), "Save _as x");
    }

    #[test]
    fn builds_results() {
        let opts =
            options(vec![("filters", Value::from(vec![("Text".to_string(), vec![(0u32, "*.txt".to_string())])]))]);
        let request = parse_request(1, false, "", &opts);
        let answer = PickerAnswer {
            ok: true,
            paths: vec![PathBuf::from("/tmp/a b.txt")],
            choices: vec![("x".into(), "true".into())],
            filter: Some(0),
        };
        let r = results(&request, &answer);
        let uris: Vec<String> = r["uris"].try_clone().unwrap().try_into().unwrap();
        assert_eq!(uris, ["file:///tmp/a%20b.txt"]);
        let (name, _): (String, Vec<(u32, String)>) = r["current_filter"].try_clone().unwrap().try_into().unwrap();
        assert_eq!(name, "Text");
        assert!(r.contains_key("choices"));
    }

    /// Answers every request with a fixed file; records what it was asked.
    struct FakePicker(Mutex<Vec<PickerRequest>>);

    impl Picker for FakePicker {
        fn show(&self, request: PickerRequest, reply: oneshot::Sender<PickerAnswer>) {
            let cancel = request.title == "cancel me";
            self.0.lock().unwrap().push(request);
            let _ =
                reply.send(PickerAnswer { ok: !cancel, paths: vec!["/tmp/picked.txt".into()], ..Default::default() });
        }
        fn close(&self, _id: u64) {}
    }

    /// The whole D-Bus round trip, over a private connection pair.
    #[test]
    fn answers_over_dbus() {
        zbus::block_on(async {
            let (a, b) = std::os::unix::net::UnixStream::pair().unwrap();
            let guid = zbus::Guid::generate();
            let fake = Arc::new(FakePicker(Mutex::default()));
            let server = zbus::connection::Builder::async_io_unix_stream(a)
                .server(guid)
                .unwrap()
                .p2p()
                .serve_at(OBJECT_PATH, FileChooser::new(fake.clone(), Arc::default()))
                .unwrap()
                .build();
            let client = zbus::connection::Builder::async_io_unix_stream(b).p2p().build();
            let (_server, client) = tokio::join!(server, client);
            let client = client.unwrap();
            let call = |method: &'static str, title: &'static str| {
                let client = client.clone();
                async move {
                    let handle = ObjectPath::try_from("/org/freedesktop/portal/desktop/request/1_1/t").unwrap();
                    let opts: HashMap<&str, Value<'_>> = HashMap::from([("multiple", Value::from(true))]);
                    let reply = client
                        .call_method(
                            None::<&str>,
                            OBJECT_PATH,
                            Some("org.freedesktop.impl.portal.FileChooser"),
                            method,
                            &(handle, "org.mozilla.firefox", "x11:1", title, opts),
                        )
                        .await
                        .unwrap();
                    reply.body().deserialize::<(u32, HashMap<String, OwnedValue>)>().unwrap()
                }
            };
            let (code, results) = call("OpenFile", "Pick").await;
            assert_eq!(code, RESPONSE_OK);
            let uris: Vec<String> = results["uris"].try_clone().unwrap().try_into().unwrap();
            assert_eq!(uris, ["file:///tmp/picked.txt"]);
            let (code, results) = call("OpenFile", "cancel me").await;
            assert_eq!((code, results.len()), (RESPONSE_CANCELLED, 0));
            let (code, _) = call("SaveFile", "Save").await;
            assert_eq!(code, RESPONSE_OK);
            let asked = fake.0.lock().unwrap();
            assert!(asked[0].multiple && !asked[0].save);
            assert!(asked[2].save && !asked[2].multiple);
        });
    }
}
