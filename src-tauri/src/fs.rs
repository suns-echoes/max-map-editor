use crate::logger;


pub fn file_exists(path: &str) -> bool {
	logger::info(&format!("file_exists (path={})", path));

	return std::path::Path::new(path).exists();
}


pub fn dir_contains_file(dir: String, file: String) -> bool {
	logger::info(&format!("dir_contains_file (dir={}, file={})", dir, file));

	let entries = match std::fs::read_dir(&dir) {
		Ok(entries) => entries,
		Err(e) => {
			logger::error(&format!("dir_contains_file (dir={}, file={}) -> Failed to read directory", dir, file));
			logger::error(&format!("dir_contains_file Error: {}", e));
			return false;
		}
	};

	for entry in entries {
		if let Ok(entry) = entry {
			if let Some(file_name) = entry.file_name().to_str() {
				if file_name == file {
					return true;
				}
			}
		}
	}

	return false;
}
