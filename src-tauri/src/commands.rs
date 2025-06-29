use crate::app_state;
use crate::fs;
use crate::logger;
use crate::settings_json;

use crate::image_to_wrl;

const ERROR_PATH_DOES_NOT_EXIST: &str = "ERROR_PATH_DOES_NOT_EXIST";
const ERROR_INVALID_MAX_PATH: &str = "ERROR_INVALID_MAX_PATH";


#[tauri::command]
pub fn validate_max_dir(path: String) -> Result<bool, String> {
	logger::info(&format!("commands::validate_max_dir path: {}", path));

	let path_exists = std::path::Path::new(&path).exists();

	if !path_exists {
		return Err(ERROR_PATH_DOES_NOT_EXIST.to_string());
	}

	let is_max_path_valid = fs::dir_contains_file(path, String::from("MAXRUN.EXE"));

	if !is_max_path_valid {
		return Err(ERROR_INVALID_MAX_PATH.to_string());
	}

	Ok(is_max_path_valid)
}


#[tauri::command]
pub fn reload_max_path() -> Result<bool, String> {
	logger::info(&format!("commands::reload_max_path"));

	let max_path = match settings_json::read_max_path_from_settings() {
		Ok(path) => path,
		Err(e) => {
			logger::error(&format!("commands::reload_max_path Failed to read M.A.X. path from settings file: {}", e));
			"".to_string()
		}
	};

	if max_path == "" {
		logger::info(&format!("commands::reload_max_path -> path is empty"));
		return Ok(false);
	}

	app_state::set_max_path(max_path.clone());

	logger::info(&format!("commands::reload_max_path -> path: {}", max_path));
	Ok(true)
}


#[tauri::command]
pub fn image_to_wrl(path: String) -> Result<(Vec<u8>, Vec<u8>), String> {
	logger::info(&format!("commands::image_to_wrl path: {}", path));

	match image_to_wrl::image_to_wrl(path.clone()) {
		Ok(image_and_palette) => {
			logger::info(&format!("commands::image_to_wrl -> Successfully converted image to WRL: {}", path));
			Ok(image_and_palette)
		},
		Err(e) => {
			logger::error(&format!("commands::image_to_wrl -> Failed to convert image to WRL: {}", e));
			Err(format!("Failed to convert image to WRL: {}", e))
		}
	}
}
