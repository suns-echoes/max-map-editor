use tauri::{AppHandle, Manager, Runtime, path::BaseDirectory};
use xlog::*;

use crate::{app_settings, app_state};

pub fn app_setup(app: &tauri::App) -> Result<(), Box<dyn std::error::Error + 'static>> {
	xlog_info!("Application setup");

	let app_local_data_path = app
		.path()
		.resolve("", BaseDirectory::AppLocalData)?;

	xlog_init(&app_local_data_path, "dmesg.log");

	app_state::set_app_local_data_path(app_local_data_path);

	let resource_path = app
		.path()
		.resolve("", BaseDirectory::Resource)?
		.join("resources");

	app_state::set_resource_path(resource_path);

	app_settings::read_from_file().map_err(|e| {
		xlog_error!("Failed to read application settings");
		xlog_error!("{}", e);
		e
	})?;

	xlog_info!("Application started");

	xlog_info!(">> app local data path: {}", app_state::get_app_local_data_path_as_str());
	xlog_info!(">> resource path: {}", app_state::get_resource_path_as_str());
	xlog_info!(">> max path: {}", app_state::get_max_path_as_str());
	Ok(())
}

pub fn app_single_instance<R: Runtime>(app: &AppHandle<R>, args: Vec<String>, cwd: String) {
	xlog_warn!("Another instance attempted to start.");
	xlog_info!("Args: {:?}, CWD: {:?}", args, cwd);

	let window = app.get_webview_window("main").unwrap();

	if window.is_minimized().unwrap() {
		window.unminimize().unwrap();
	}

	window.show().unwrap();
	window.set_focus().unwrap();
}
