mod commands;
mod menu;
mod native;
mod open;
mod ops;
mod picker;
#[cfg(target_os = "linux")]
mod portal;
mod protocol;
#[cfg(target_os = "macos")]
mod reveal_macos;

use tauri::{AppHandle, Manager, RunEvent, WindowEvent};

/// `sonar --portal`: started by D-Bus to show a file chooser (see portal.rs).
/// No main window then, until Sonar is opened normally.
const PORTAL_ARG: &str = "--portal";

pub fn run() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let portal_only = args.iter().any(|a| a == PORTAL_ARG);
    // The portal service is started by D-Bus; it must not hand its arguments
    // to a running main instance and exit, or the chooser never appears.
    // Portal instances bypass the single-instance lock (and lose the bus-name
    // race gracefully if a main instance already serves the portal).
    let mut builder = tauri::Builder::default();
    if !portal_only {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            show_main_window(app);
            open::handle(app, sonar_core::paths::paths_from_args(argv.get(1..).unwrap_or_default(), cwd.as_ref()));
        }));
    }
    // Window size, position and maximized state, saved on close and
    // restored at start (the main window's; pickers open centred).
    builder
        .plugin(tauri_plugin_window_state::Builder::default().with_filter(|label| label == "main").build())
        .manage(commands::ViewState::default())
        .manage(commands::SearchState::default())
        .manage(commands::SizeJobs::default())
        .manage(open::OpenQueue::default())
        .manage(ops::OpsState::default())
        .manage(picker::Pending::default())
        .manage(picker::Usage::new(portal_only))
        .register_asynchronous_uri_scheme_protocol(protocol::SCHEME, protocol::handle)
        .on_window_event(|window, event| {
            native::forward_drag_drop(window, event);
            if let WindowEvent::Destroyed = event {
                picker::window_destroyed(window.app_handle(), window.label());
                if window.label() == "main" {
                    open::main_window_closed(window.app_handle());
                }
            }
        })
        .setup(move |app| {
            commands::watch_places(app.handle().clone());
            #[cfg(target_os = "macos")]
            {
                menu::install(app.handle());
                reveal_macos::install(app.handle());
            }
            #[cfg(target_os = "linux")]
            portal::start(app.handle(), portal_only);
            if !portal_only {
                show_main_window(app.handle());
                // `sonar ~/Downloads` (and desktop launchers' `Exec=sonar %U`).
                let cwd = std::env::current_dir().unwrap_or_else(|_| "/".into());
                open::handle(app.handle(), sonar_core::paths::paths_from_args(&args, &cwd));
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_places,
            commands::open_dir,
            commands::close_pane,
            commands::start_search,
            commands::cancel_search,
            commands::get_properties,
            commands::apps_for,
            commands::open_with,
            commands::set_default_app,
            commands::start_size,
            commands::cancel_size,
            commands::mount_device,
            menu::has_native_menu,
            menu::set_menu_state,
            commands::eject_volume,
            commands::open_path,
            commands::resolve_path,
            commands::set_archive_password,
            commands::home_dir,
            open::take_open_requests,
            ops::rename_entry,
            ops::create_folder,
            ops::trash_entries,
            ops::delete_entries,
            ops::undo,
            ops::undo_label,
            ops::start_transfer,
            ops::start_extract,
            ops::start_compress,
            ops::cancel_job,
            ops::resolve_conflict,
            native::clipboard_write_files,
            native::clipboard_read_files,
            native::clipboard_write_text,
            native::clipboard_clear_cut,
            native::start_drag_out,
            picker::picker_request,
            picker::picker_respond,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Sonar")
        .run(|app, event| match event {
            // The last window closed. Serving file choosers, stay a while: the
            // next one opens much faster (picker::Usage ends it when idle).
            RunEvent::ExitRequested { code: None, api, .. } if app.state::<picker::Usage>().keep_running() => {
                api.prevent_exit();
            }
            // macOS: folders opened with Sonar from Finder, `open -a Sonar`, or
            // as the default folder handler arrive as open events.
            #[cfg(target_os = "macos")]
            RunEvent::Opened { urls } => {
                show_main_window(app);
                open::handle(app, urls.iter().filter_map(|u| u.to_file_path().ok()).collect());
            }
            _ => {}
        });
}

/// Bring up the main window, creating it (from tauri.conf.json) if needed.
fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let Some(config) = app.config().app.windows.iter().find(|w| w.label == "main") else { return };
    if let Err(e) = tauri::WebviewWindowBuilder::from_config(app, config).and_then(|b| b.build()) {
        eprintln!("Sonar: could not open the main window: {e}");
    }
}
