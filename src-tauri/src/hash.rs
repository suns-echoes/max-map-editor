use md5;

#[tauri::command]
pub fn hash_md5(request: tauri::ipc::Request) -> String {
	let tauri::ipc::InvokeBody::Raw(data) = request.body() else {
		return "".to_string();
	};

	let hash = md5::compute(data);
	format!("{:x}", hash)
}
