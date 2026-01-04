use xlog::*;

use crate::image_to_wrl;

#[tauri::command]
pub fn image_to_wrl_command(path: String) -> Result<(Vec<u8>, Vec<u8>), String> {
    xlog_info!("commands::image_to_wrl_command path: {}", path);

    match image_to_wrl::image_to_wrl(path.clone()) {
        Ok(image_and_palette) => {
            xlog_info!(
                "commands::image_to_wrl_command -> Successfully converted image to WRL: {}",
                path
            );
            Ok(image_and_palette)
        }
        Err(e) => {
            xlog_error!(
                "commands::image_to_wrl_command -> Failed to convert image to WRL: {}",
                e
            );
            Err(format!("Failed to convert image to WRL: {}", e))
        }
    }
}
