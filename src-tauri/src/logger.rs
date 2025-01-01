use crate::app_state;

use std::fs::OpenOptions;
use std::io::Write;


pub fn create_file() {
	let logger_file_path = format!("{}/tauri.log", app_state::get_app_local_data_path());
	eprintln!("\x1b[32m>> logger::create_file: {}\x1b[0m", logger_file_path);

	match OpenOptions::new()
		.create(true)
		.write(true)
		.truncate(true)
		.open(&logger_file_path) {
			Ok(_) => (),
			Err(e) => eprintln!("\x1b[31m>> logger::create_file Error: {}\x1b[0m", e)
		}
}

pub fn info(message: &str) {
	// match OpenOptions::new()
	// 	.append(true)
	// 	.open(&format!("{}/tauri.log", app_state::get_app_local_data_path())) {
	// 		Ok(_) => (),
	// 		Err(e) => eprintln!("\x1b[31m>> logger::info Error: {}\x1b[0m", e)
	// 	}
}

pub fn error(message: &str) {
	// match OpenOptions::new()
	// 	.append(true)
	// 	.open(&format!("{}/tauri.log", app_state::get_app_local_data_path())) {
	// 		Ok(_) => (),
	// 		Err(e) => eprintln!("\x1b[31m>> logger::info Error: {}\x1b[0m", e)
	// 	}
}
