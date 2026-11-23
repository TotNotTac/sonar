//! "Show in Finder" for apps, when Sonar is the file viewer
//! (`defaults write -g NSFileViewer -string dev.sonar.filemanager`).
//!
//! AppKit delivers those requests to the file viewer as Finder's own
//! "reveal" Apple Event (class `misc`, id `mvis`), which Tauri doesn't
//! handle. Each item is shown selected in its parent folder, like Finder.

use std::path::PathBuf;
use std::sync::OnceLock;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager};
use tauri::AppHandle;

static APP: OnceLock<AppHandle> = OnceLock::new();

const fn four_cc(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SonarRevealHandler"]
    struct RevealHandler;

    unsafe impl NSObjectProtocol for RevealHandler {}

    impl RevealHandler {
        #[unsafe(method(handleReveal:withReplyEvent:))]
        fn handle_reveal(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            if let Some(app) = APP.get() {
                crate::open::handle_reveal(app, file_paths(event));
            }
        }
    }
);

/// The files in the event's direct object (a single item or a list).
fn file_paths(event: &NSAppleEventDescriptor) -> Vec<PathBuf> {
    let Some(direct) = event.paramDescriptorForKeyword(four_cc(b"----")) else { return Vec::new() };
    let one = |d: &NSAppleEventDescriptor| -> Option<PathBuf> {
        let url = d.fileURLValue().or_else(|| d.coerceToDescriptorType(four_cc(b"furl"))?.fileURLValue())?;
        Some(PathBuf::from(url.path()?.to_string()))
    };
    if direct.descriptorType() == four_cc(b"list") {
        (1..=direct.numberOfItems()).filter_map(|i| direct.descriptorAtIndex(i)).filter_map(|d| one(&d)).collect()
    } else {
        one(&direct).into_iter().collect()
    }
}

/// Register for reveal events. Call once, on the main thread.
pub fn install(app: &AppHandle) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let _ = APP.set(app.clone());
    let handler: Retained<RevealHandler> = unsafe { msg_send![RevealHandler::alloc(mtm), init] };
    let target: &AnyObject = &handler;
    // SAFETY: the selector matches the method defined above, and the handler
    // lives for the rest of the process (leaked below).
    unsafe {
        NSAppleEventManager::sharedAppleEventManager().setEventHandler_andSelector_forEventClass_andEventID(
            target,
            sel!(handleReveal:withReplyEvent:),
            four_cc(b"misc"),
            four_cc(b"mvis"),
        );
    }
    std::mem::forget(handler);
}
