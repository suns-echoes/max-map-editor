use crate::app_state;
use crate::stdio;
use crate::time;

use std::fs::OpenOptions;
use std::io::Write;


pub fn create_file() {
	let logger_file_path = format!("{}/tauri.log", app_state::get_app_local_data_path());
	let message = format!("{}", time::get_yyyymmdd_hhmmss());
	stdio::info(&format!("Creating log file: {}", logger_file_path));

	let mut file = OpenOptions::new()
		.create(true)
		.write(true)
		.truncate(true)
		.open(&logger_file_path)
		.unwrap();

	match writeln!(file, "{}", message) {
		Ok(_) => (),
		Err(e) => stdio::error(&format!("logger::create_file Error: {}", e))
	}
}

pub fn info(message: &str) {
	stdio::info(&format!("logger::info: {}", message));
	append_log(message);
}

pub fn warn(message: &str) {
	stdio::warn(&format!("logger::warn: {}", message));
	append_log(&format!("WARNING: {}", message));
}

pub fn error(message: &str) {
	stdio::info(&format!("logger::error: {}", message));
	append_log(&format!("ERROR: {}", message));
}


fn append_log(message: &str) {
	let logger_file_path = format!("{}/tauri.log", app_state::get_app_local_data_path());
	let mut file = OpenOptions::new().write(true).append(true).open(&logger_file_path).unwrap();

	match writeln!(file, "{}", message) {
		Ok(_) => (),
		Err(e) => stdio::error(&format!("logger::append_log Error: {}", e))
	}
}
