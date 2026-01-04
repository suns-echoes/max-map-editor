mod app_setup;
mod app_state;
use app_setup::*;
mod app_settings;

mod image_to_wrl;

mod commands;
use commands::*;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(app_single_instance))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            image_to_wrl_command,
            open_devtools_command,
			validate_max_path_command,
			xlog_command,
        ])
        .setup(|app| app_setup(app))
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
