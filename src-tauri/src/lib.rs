mod devtools;
mod fs;
mod hash;
mod zip;

use tauri::Manager;

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
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
			fs::update_max_path,
			fs::check_max_dir,
			fs::file_exists,
			fs::read_wrl_file,
			fs::write_wrl_file,
			hash::hash_md5,
			zip::get_zip_file_list,
			zip::load_zip_file_content
		])
		.setup(|app| {
			let app_handle = app.handle();
			fs::update_app_data_path(format!(
				"{}/{}",
				app_handle
					.path()
					.data_dir()
					.expect("failed to get app data path")
					.to_string_lossy()
					.to_string(),
				app_handle.config().identifier
			));
			fs::read_max_path_from_settings();
			Ok(())
		})
		.run(tauri::generate_context!())
		.expect("error while running tauri application");
}
