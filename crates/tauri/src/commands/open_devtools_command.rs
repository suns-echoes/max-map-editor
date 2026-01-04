use tauri::{Manager, Runtime};

#[tauri::command]
pub fn open_devtools_command<R: Runtime>(app: tauri::AppHandle<R>) -> Result<bool, String> {
    #[cfg(debug_assertions)]
    {
        let window = app.get_webview_window("main").unwrap();
        window.open_devtools();
    }

    Ok(true)
}
