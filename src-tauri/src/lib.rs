// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
mod commands;
mod error;
mod library;

use tauri::Manager;

use commands::AppState;

/// Where imported databases and their index live, under the OS's per-app data
/// directory (`~/.local/share/com.vm.grepm/imports` on Linux). Created on
/// first launch; the user is never asked to choose it.
const IMPORTS_DIR: &str = "imports";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let imports_dir = app.path().app_data_dir()?.join(IMPORTS_DIR);
            std::fs::create_dir_all(&imports_dir)?;
            app.manage(AppState::new(imports_dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_imports,
            commands::active_import,
            commands::open_import,
            commands::start_import,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
