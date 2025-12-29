pub fn get_yyyymmdd_hhmmss() -> String {
	let now = chrono::Local::now();
	return now.format("%Y-%m-%d %H:%M:%S").to_string();
}
