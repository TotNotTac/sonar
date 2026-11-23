mod commands;
mod menu;
mod native;
mod open;
mod ops;
mod protocol;
#[cfg(target_os = "macos")]
mod reveal_macos;

pub fn run() {
    tauri::Builder::default()
        // Must be first: a second `sonar <folder>` hands its arguments to the
        // running instance and exits.
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            open::handle(app, sonar_core::paths::paths_from_args(argv.get(1..).unwrap_or_default(), cwd.as_ref()));
        }))
        .manage(commands::ViewState::default())
        .manage(commands::SearchState::default())
        .manage(open::OpenQueue::default())
        .manage(ops::OpsState::default())
        .register_asynchronous_uri_scheme_protocol(protocol::SCHEME, protocol::handle)
        .setup(|app| {
            commands::watch_places(app.handle().clone());
            #[cfg(target_os = "macos")]
            {
                menu::install(app.handle());
                reveal_macos::install(app.handle());
            }
            // `sonar ~/Downloads` (and desktop launchers' `Exec=sonar %U`).
            let args: Vec<String> = std::env::args().skip(1).collect();
            let cwd = std::env::current_dir().unwrap_or_else(|_| "/".into());
            open::handle(app.handle(), sonar_core::paths::paths_from_args(&args, &cwd));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_places,
            commands::open_dir,
            commands::close_pane,
            commands::start_search,
            commands::cancel_search,
            commands::mount_device,
            menu::has_native_menu,
            menu::set_menu_state,
            commands::eject_volume,
            commands::open_path,
            commands::resolve_path,
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
        ])
        .build(tauri::generate_context!())
        .expect("error while building Sonar")
        .run(|_app, _event| {
            // macOS: folders opened with Sonar from Finder, `open -a Sonar`, or
            // as the default folder handler arrive as open events.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = _event {
                open::handle(_app, urls.iter().filter_map(|u| u.to_file_path().ok()).collect());
            }
        });
}
