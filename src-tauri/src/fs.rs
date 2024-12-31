use std::io::Write;

//
// TODO: Consider using state struct https://v2.tauri.app/develop/state-management/
//
static mut APP_DATA_PATH: String = String::new();
static mut MAX_PATH: String = String::new();
static mut MAX_DIR_VALID: bool = false;

pub fn update_app_data_path(path: String) {
	unsafe {
		APP_DATA_PATH = path;
	}
}

pub fn read_max_path_from_settings() {
	if !std::path::Path::new(&format!("{}/settings.json", unsafe {
		APP_DATA_PATH.clone()
	}))
	.exists() {
		return;
	}
	let app_data_path = unsafe { APP_DATA_PATH.clone() };
	let settings_path = format!("{}/settings.json", app_data_path);
	let settings_content =
		std::fs::read_to_string(&settings_path).expect("failed to read settings file");
	let settings_json: serde_json::Value =
		serde_json::from_str(&settings_content).expect("failed to parse settings file");
	let max_path = settings_json["max"]["path"]
		.as_str()
		.expect("failed to get max.path property")
		.to_string();
	unsafe {
		MAX_PATH = max_path;
	}
}

fn is_max_dir_valid(path: String) -> bool {
	let entries = match std::fs::read_dir(&path) {
		Ok(entries) => entries,
		Err(_) => return false,
	};

	let mut files_in_dir = Vec::new();
	for entry in entries {
		if let Ok(entry) = entry {
			if let Some(file_name) = entry.file_name().to_str() {
				files_in_dir.push(file_name.to_uppercase());
			}
		}
	}

	let files = [
		"CRATER_1.WRL",
		"CRATER_2.WRL",
		"CRATER_3.WRL",
		"CRATER_4.WRL",
		"CRATER_5.WRL",
		"CRATER_6.WRL",
		"DESERT_1.WRL",
		"DESERT_2.WRL",
		"DESERT_3.WRL",
		"DESERT_4.WRL",
		"DESERT_5.WRL",
		"DESERT_6.WRL",
		"GREEN_1.WRL",
		"GREEN_2.WRL",
		"GREEN_3.WRL",
		"GREEN_4.WRL",
		"GREEN_5.WRL",
		"GREEN_6.WRL",
		"SNOW_1.WRL",
		"SNOW_2.WRL",
		"SNOW_3.WRL",
		"SNOW_4.WRL",
		"SNOW_5.WRL",
		"SNOW_6.WRL",
	];

	for file in files.iter() {
		if !files_in_dir.contains(&file.to_string()) {
			unsafe {
				MAX_DIR_VALID = false;
			}
			return false;
		}
	}

	unsafe {
		MAX_DIR_VALID = true;
	}
	return true;
}

fn starts_in_max_path(path: String) -> bool {
	let path = std::path::Path::new(path.as_str());

	unsafe {
		if !MAX_DIR_VALID {
			return false;
		}

		return path.starts_with(MAX_PATH.clone());
	}
}

#[tauri::command]
pub fn update_max_path() -> bool {
	read_max_path_from_settings();
	unsafe { is_max_dir_valid(MAX_PATH.clone()) }
}

#[tauri::command]
pub fn check_max_dir(path: String) -> bool {
	is_max_dir_valid(path)
}

#[tauri::command]
pub fn file_exists(path: String) -> bool {
	std::path::Path::new(&path).exists()
}

#[tauri::command]
pub fn read_wrl_file(path: String) -> tauri::ipc::Response {
	unsafe {
		if !MAX_DIR_VALID && !is_max_dir_valid(MAX_PATH.clone()) {
			return tauri::ipc::Response::new("Error: Invalid M.A.X. path".as_bytes().to_vec());
		}
	}

	if !path.ends_with(".WRL") {
		return tauri::ipc::Response::new("Error: Can read only WRL files".as_bytes().to_vec());
	}

	if !starts_in_max_path(path.clone()) {
		return tauri::ipc::Response::new(
			"Error: Can read files from M.A.X. directory only"
				.as_bytes()
				.to_vec(),
		);
	}

	let data = std::fs::read(path).unwrap();
	return tauri::ipc::Response::new(data);
}

#[tauri::command]
pub fn write_wrl_file(request: tauri::ipc::Request) -> String {
	let tauri::ipc::InvokeBody::Raw(data) = request.body() else {
		return "Error: Expected uint8 array".to_string();
	};
	let Some(header_path) = request.headers().get("path") else {
		return "Error: Expected path".to_string();
	};
	let path = header_path
		.to_str()
		.map(|s| s.to_string())
		.expect("path is not a valid string");

	unsafe {
		if !MAX_DIR_VALID && !is_max_dir_valid(MAX_PATH.clone()) {
			return "Error: Invalid M.A.X. path".to_string();
		}
	}

	if !path.ends_with(".WRL") {
		return "Error: Can write only WRL files".to_string();
	}

	if !starts_in_max_path(path.clone()) {
		return "Error: Can write files to M.A.X. directory only".to_string();
	}

	let mut f = std::fs::File::create(&path).expect("no file found");
	f.write_all(&data).expect("buffer overflow");

	"OK".to_string()
}
