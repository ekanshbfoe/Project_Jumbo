// Release builds run without a console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod recent;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::create_project,
            commands::open_project,
            commands::close_project,
            commands::current_project,
            commands::recent_projects,
            commands::forget_recent_project,
            commands::list_chapters,
            commands::import_chapters,
            commands::book_overview,
            commands::verify_book,
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| panic!("failed to start Jumbo: {e}"));
}
