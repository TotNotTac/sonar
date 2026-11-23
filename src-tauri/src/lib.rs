mod commands;
mod native;
mod ops;

pub fn run() {
    tauri::Builder::default()
        .manage(commands::ViewState::default())
        .manage(ops::OpsState::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_places,
            commands::open_dir,
            commands::close_pane,
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
