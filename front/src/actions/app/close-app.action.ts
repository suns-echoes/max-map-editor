import { getAllWindows } from '@tauri-apps/api/window';
import { xlog } from '^lib/xlog/xlog.ts';


export async function CloseAppAction() {
	try {
		const windows = await getAllWindows();

		if (windows.length > 0) {
			for (const window of windows) {
				try {
					await window.close();
				} catch (error) {
					xlog.error(`Error closing window ${window.label}:`, error);
					return;
				}
			}
		}
	} catch(error) {
		xlog.error('Error getting windows:', error);
	}
}
