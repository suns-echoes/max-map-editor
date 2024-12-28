use tauri::Manager;

#[tauri::command]
pub fn open_devtools(app_handle: tauri::AppHandle) {
	#[cfg(debug_assertions)] {
		app_handle
			.get_webview_window("main")
			.unwrap()
			.open_devtools();
	}
	#[cfg(not(debug_assertions))] {
		return ();
	}
}

#[tauri::command]
pub fn close_devtools(app_handle: tauri::AppHandle) {
	#[cfg(debug_assertions)] {
		app_handle
			.get_webview_window("main")
			.unwrap()
			.close_devtools();
	}
	#[cfg(not(debug_assertions))] {
		return ();
	}
}

#[tauri::command]
pub fn is_devtools_open(app_handle: tauri::AppHandle) -> bool {
	#[cfg(debug_assertions)] {
		return app_handle
			.get_webview_window("main")
			.unwrap()
			.is_devtools_open();
	}
	#[cfg(not(debug_assertions))] {
		return false;
	}
}
