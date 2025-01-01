use crate::app_state;
use crate::fs;
use crate::logger;


pub fn read() -> Result<serde_json::Value, String> {
	if !fs::file_exists(&format!("{}/settings.json", app_state::get_app_local_data_path())) {
		logger::error("settings_json::read -> Error: settings.json not found");
		return Err("Error: settings.json not found".to_string());
	}

	let settings_content = match std::fs::read_to_string("settings.json") {
		Ok(content) => content,
		Err(e) => {
			logger::error(&format!("settings_json::read -> Error: failed to read settings.json: {}", e));
			return Err("Error: failed to read settings.json".to_string());
		}
	};

	let json: serde_json::Value = match serde_json::from_str(&settings_content) {
		Ok(json) => json,
		Err(e) => {
			logger::error(&format!("settings_json::read -> Error: failed to parse settings.json: {}", e));
			return Err("Error: failed to parse settings.json".to_string());
		}
	};

	Ok(json)
}


pub fn read_max_path_from_settings() -> Result<String, String> {
	let settings = match read() {
		Ok(settings) => settings,
		Err(e) => return Err(e)
	};

	let max_path = match settings["max"]["path"].as_str() {
		Some(path) => path.to_string(),
		None => {
			logger::error("failed to get max.path property");
			return Err("failed to get max.path property".to_string());
		}
	};

	Ok(max_path)
}
