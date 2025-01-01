use std::fs::File;
use std::io::Read;
use zip::ZipArchive;


#[tauri::command]
pub fn get_zip_file_list(path: String) -> Result<Vec<(String, u64)>, String> {
	let file = File::open(&path).map_err(|e| e.to_string())?;
	let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;

	let mut file_info = Vec::new();

	for i in 0..archive.len() {
		let file = archive.by_index(i).map_err(|e| e.to_string())?;
		file_info.push((file.name().to_string(), file.size()));
	}

	Ok(file_info)
}


// #[tauri::command]
// pub fn get_zip_file_list(path: String) -> Result<Vec<String>, String> {
// 	let file = File::open(&path).map_err(|e| e.to_string())?;
// 	let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;

// 	let mut file_names = Vec::new();

// 	for i in 0..archive.len() {
// 		let file = archive.by_index(i).map_err(|e| e.to_string())?;
// 		file_names.push(file.name().to_string());
// 	}

// 	Ok(file_names)
// }


#[tauri::command]
pub fn load_zip_file_content(path: String) -> Result<Vec<String>, String> {
	let file = File::open(&path).map_err(|e| e.to_string())?;
	let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;

	let mut file_contents = Vec::new();

	for i in 0..archive.len() {
		let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
		let mut contents = String::new();
		file.read_to_string(&mut contents)
			.map_err(|e| e.to_string())?;
		file_contents.push(contents);
	}

	Ok(file_contents)
}
