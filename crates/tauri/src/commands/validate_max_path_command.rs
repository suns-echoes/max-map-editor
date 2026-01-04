#[tauri::command]
pub fn validate_max_path_command(path_str: String) -> Result<(), String> {
	let path = std::path::PathBuf::from(path_str);

	if !path.exists() {
		return Err(format!("Path does not exist: {}", path.display()));
	}

	if !path.is_dir() {
		return Err(format!("Path is not a directory: {}", path.display()));
	}

	if !check_for_max_res_file(&path) {
		return Err(format!(
			"'MAX.RES' file not found in directory: {}",
			path.display()
		));
	}

	Ok(())
}

fn check_for_max_res_file(dir_path: &std::path::Path) -> bool {
	if let Ok(entries) = std::fs::read_dir(dir_path) {
		for entry in entries {
			if let Ok(entry) = entry {
				if let Some(file_name) = entry.file_name().to_str() {
					if file_name.eq_ignore_ascii_case("MAX.RES") {
						return true;
					}
				}
			}
		}
	}
	false
}
