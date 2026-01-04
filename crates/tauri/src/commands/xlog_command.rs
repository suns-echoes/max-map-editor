use xlog::*;

#[tauri::command]
pub async fn xlog_command(level: String, message: String) -> Result<(), String> {
	match level.as_str() {
		"SUCCESS" => {
			xlog_success!("{}", &message);
			Ok(())
		}
		"INFO" => {
			xlog_info!("{}", &message);
			Ok(())
		}
		"WARN" => {
			xlog_warn!("{}", &message);
			Ok(())
		}
		"ERROR" => {
			xlog_error!("{}", &message);
			Ok(())
		}
		_ => Err(format!("Invalid log level: {}", level)),
	}
}
