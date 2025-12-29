import { getCurrentWindow } from '^tauri-apps/api/window.ts';
import { PhysicalPosition, PhysicalSize } from '^tauri-apps/api/dpi.ts';

import { RustAPI } from '^src/bff/rust-api';
import { printDebugInfo } from '^lib/debug/debug.ts';
import { SettingsFile } from '^storage/perma-storage/settings-file.ts';


export async function restoreMainWindow() {
	await printDebugInfo('restoreMainWindow');

	const settings = SettingsFile.getAll();

	if (settings.debug.showDevTools) {
		RustAPI.openDevTools().catch(console.error);
	}

	const currentWindow = getCurrentWindow();

	currentWindow.setSize(
		new PhysicalSize(settings.window.width ?? 1280, settings.window.height ?? 920),
	);

	currentWindow.setSize(
		new PhysicalSize(settings.window.width ?? 1280, settings.window.height ?? 920),
	);


	currentWindow.setPosition(
		new PhysicalPosition(settings.window.x, settings.window.y),
	);

	if (settings.window.maximized) {
		currentWindow.maximize();
	}
}
