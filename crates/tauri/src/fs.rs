use crate::{app_state, logger};


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


pub fn read_file_from_resources(path: &str) -> Result<String, String> {
	logger::info(&format!("read_file_from_resources (path={})", path));

	let resource_path = app_state::get_resource_path();

	let full_path = std::path::Path::new(&resource_path).join(path);
	if !full_path.exists() {
		logger::error(&format!("read_file_from_resources -> File does not exist: {}", full_path.display()));
		return Err(format!("File does not exist: {}", full_path.display()));
	}

	let content = match std::fs::read_to_string(&full_path) {
		Ok(content) => {
			logger::info(&format!("read_file_from_resources -> File read successfully: {}", full_path.display()));
			content
		}
		Err(e) => {
			logger::error(&format!("read_file_from_resources -> Failed to read file: {}", e));
			return Err(format!("Failed to read file: {}", e));
		}
	};

	Ok(content)
}
