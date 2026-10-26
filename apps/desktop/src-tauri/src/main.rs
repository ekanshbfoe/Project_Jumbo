// Release builds run without a console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bridge;
mod commands;
mod credentials;
mod recent;

use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::AppState::default())
        .manage(bridge::BridgeState::default())
        .setup(|app| {
            // `jumbo-desktop <project folder>` (e.g. "Open with") opens that project at startup.
            if let Some(path) = std::env::args_os().nth(1) {
                let state = app.state::<commands::AppState>();
                if let Err(e) = commands::open_at_startup(app.handle(), &state, path.into()) {
                    eprintln!("could not open {e}");
                }
            }
            Ok(())
        })
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
            commands::chapter_preview,
            bridge::bridge_settings,
            bridge::save_bridge_settings,
            bridge::credential_status,
            bridge::import_kaggle_json,
            bridge::set_kaggle_token,
            bridge::set_github_token,
            bridge::session_view,
            bridge::start_session,
            bridge::run_bridge_test,
            bridge::stop_session,
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| panic!("failed to start Jumbo: {e}"));
}
