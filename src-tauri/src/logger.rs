use std::fs::OpenOptions;
use std::io::Write;


pub fn create_file() {
	std::fs::write("tauri.log", "").expect("failed to create log file");
}

pub fn info(message: &str) {
	let mut file = OpenOptions::new()
		.append(true)
		.open("tauri.log")
		.expect("failed to open log file");
	writeln!(file, "INFO: {}", message).expect("failed to write to log file");
}

pub fn error(message: &str) {
	let mut file = OpenOptions::new()
		.append(true)
		.open("tauri.log")
		.expect("failed to open log file");
	writeln!(file, "ERROR: {}", message).expect("failed to write to log file");
}
