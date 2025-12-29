import { getAllWindows } from '@tauri-apps/api/window';


export async function CloseAppAction() {
	try {
		const windows = await getAllWindows();

		if (windows.length > 0) {
			for (const window of windows) {
				try {
					await window.close();
				} catch (error) {
					console.error(`Error closing window ${window.label}:`, error);
					return;
				}
			}
		}
	} catch(error) {
		console.error('Error getting windows:', error);
	}
}
