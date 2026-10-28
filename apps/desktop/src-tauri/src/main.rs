// Release builds run without a console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bridge;
mod commands;
mod credentials;
mod gemini;
mod memory;
mod recent;
mod render;
mod script;
mod vision;
mod voice;

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
            commands::set_project_method,
            bridge::bridge_settings,
            bridge::save_bridge_settings,
            bridge::credential_status,
            bridge::import_kaggle_json,
            bridge::set_kaggle_token,
            bridge::set_github_token,
            bridge::session_view,
            bridge::start_session,
            bridge::run_bridge_test,
            bridge::pool_status,
            vision::analyze_chapter,
            vision::chapter_panels,
            script::write_script,
            script::chapter_script,
            script::approve_script,
            script::rewind_chapter,
            gemini::gemini_pack,
            gemini::save_gemini_script,
            gemini::export_numbered_pdf,
            voice::voice_library,
            voice::import_voice,
            voice::record_voice,
            voice::chapter_audio,
            render::save_episode,
            render::render_chapter,
            render::chapter_render,
            bridge::stop_session,
            bridge::resume_session,
        ])
        .build(tauri::generate_context!())
        .unwrap_or_else(|e| panic!("failed to start Jumbo: {e}"))
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                // Quitting must not leave a Kaggle session running with nobody connected to it.
                tauri::async_runtime::block_on(bridge::stop_on_exit(app));
            }
        });
}
