mod commands;
mod menu;
mod native;
mod ops;
mod protocol;

pub fn run() {
    tauri::Builder::default()
        .manage(commands::ViewState::default())
        .manage(ops::OpsState::default())
        .register_asynchronous_uri_scheme_protocol(protocol::SCHEME, protocol::handle)
        .setup(|app| {
            commands::watch_places(app.handle().clone());
            #[cfg(target_os = "macos")]
            menu::install(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_places,
            commands::open_dir,
            commands::close_pane,
            commands::mount_device,
            menu::has_native_menu,
            menu::set_menu_state,
            commands::eject_volume,
            commands::open_path,
            commands::resolve_path,
            commands::home_dir,
            ops::rename_entry,
            ops::create_folder,
            ops::trash_entries,
            ops::delete_entries,
            ops::undo,
            ops::undo_label,
            ops::start_transfer,
            ops::cancel_job,
            ops::resolve_conflict,
            native::clipboard_write_files,
            native::clipboard_read_files,
            native::clipboard_write_text,
            native::clipboard_clear_cut,
            native::start_drag_out,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Sonar");
}
