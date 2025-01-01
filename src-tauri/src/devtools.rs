#[cfg(debug_assertions)]
use tauri::Manager;


#[tauri::command]
pub fn open_devtools(_app_handle: tauri::AppHandle) {
	#[cfg(debug_assertions)] {
		_app_handle
			.get_webview_window("main")
			.unwrap()
			.open_devtools();
	}
	#[cfg(not(debug_assertions))] {
		return ();
	}
}


#[tauri::command]
pub fn close_devtools(_app_handle: tauri::AppHandle) {
	#[cfg(debug_assertions)] {
		_app_handle
			.get_webview_window("main")
			.unwrap()
			.close_devtools();
	}
	#[cfg(not(debug_assertions))] {
		return ();
	}
}


#[tauri::command]
pub fn is_devtools_open(_app_handle: tauri::AppHandle) -> bool {
	#[cfg(debug_assertions)] {
		return _app_handle
			.get_webview_window("main")
			.unwrap()
			.is_devtools_open();
	}
	#[cfg(not(debug_assertions))] {
		return false;
	}
}
