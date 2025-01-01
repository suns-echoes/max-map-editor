import { getCurrentWindow, PhysicalPosition, PhysicalSize } from '^tauri-apps/api/window.ts';

import { SettingsFile } from '^storage/perma-storage/settings-file.ts';
import { RustAPI } from '^utils/rust-api.ts';
import { printDebugInfo } from '^utils/debug/debug.ts';


export async function restoreMainWindow() {
	await printDebugInfo('restoreMainWindow');

	const settings = SettingsFile.getAll();

	if (settings.debug.showDevTools) {
		RustAPI.openDevTools().catch(console.error);
	}

	const currentWindow = getCurrentWindow();

	currentWindow.setSize(
		new PhysicalSize(settings.window.width ?? 1280, settings.window.height ?? 920)
	);

	currentWindow.setSize(
		new PhysicalSize(settings.window.width ?? 1280, settings.window.height ?? 920)
	);


	currentWindow.setPosition(
		new PhysicalPosition(settings.window.x, settings.window.y)
	);

	if (settings.window.maximized) {
		currentWindow.maximize();
	}
}
