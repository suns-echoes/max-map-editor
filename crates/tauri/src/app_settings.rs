use xini::XINI;
use xlog::*;

use crate::{app_state};

pub fn read_from_file() -> Result<(), String> {
	let settings_path = app_state::get_app_local_data_path_to("settings.ini");

	if !settings_path.exists() {
		xlog_info!("Settings file does not exist; recreating with default values at: {}", settings_path.display());

		let default_settings_path = app_state::get_resource_path_to("settings.ini");
		let default_settings = XINI::load_from_file(&default_settings_path)
			.map_err(|e| format!("Failed to read default settings file at {}: {}", default_settings_path.display(), e))?;

		default_settings.save_to_file(&settings_path)
			.map_err(|e| format!("Failed to create settings file: {}", e))?;
	}

	let settings = XINI::load_from_file(&settings_path)
		.map_err(|e| format!("Failed to read settings file: {}", e))?;

	app_state::set_max_path(
		&settings.get_string_or("paths", "max_path", ""),
	);

	Ok(())
}

pub fn write_to_file() -> Result<(), String> {
	let settings_path = app_state::get_app_local_data_path_to("settings.ini");

	let mut settings = XINI::new();

	settings.set_string("paths", "max_path",
		&app_state::get_max_path_as_str(),
	);

	settings.save_to_file(&settings_path)
		.map_err(|e| format!("Failed to write settings file: {}", e))?;

	Ok(())
}
