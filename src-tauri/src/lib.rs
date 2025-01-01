mod app_state;
mod commands;
mod devtools;
// mod hash;
mod logger;
mod fs;
mod settings_json;
// mod zip;

use tauri::Manager;


#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
	tauri::Builder::default()
		.plugin(tauri_plugin_dialog::init())
		.plugin(tauri_plugin_fs::init())
		.plugin(tauri_plugin_shell::init())
		.invoke_handler(tauri::generate_handler![
			devtools::open_devtools,
			devtools::close_devtools,
			devtools::is_devtools_open,
			commands::validate_max_dir,
			commands::reload_max_path,
			// hash::hash_md5,
			// zip::get_zip_file_list,
			// zip::load_zip_file_content
		])
		.setup(|app| {
			eprintln!(">> Init application");

			// TODO: Update the M.A.X. path in app state (load from settings.json) - show setup dialog if not set or file is missing/broken
			// TODO: Write file access system that allows file operations only inside M.A.X. directory OR projects directory (add projects directory to settings.json)

			let app_handle = app.handle();

			app_state::set_app_local_data_path(format!(
				"{}/{}",
				app_handle
					.path()
					.local_data_dir()
					.expect("failed to get app data path")
					.to_string_lossy()
					.to_string(),
				app_handle.config().identifier
			));

			eprintln!(">> local data path: {}", app_state::get_app_local_data_path());
			eprintln!(">> max path: {}", app_state::get_max_path());

			// settings_json::read_max_path_from_settings();

			logger::create_file();

			Ok(())
		})
		.run(tauri::generate_context!())
		.expect("error while running tauri application");
}
