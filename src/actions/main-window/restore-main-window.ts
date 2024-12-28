import { getCurrentWindow, PhysicalPosition, PhysicalSize } from '@tauri-apps/api/window';

import { SettingsFile } from '^storage/perma-storage/settings-file.ts';
import { RustAPI } from '^utils/rust-api.ts';


export async function restoreMainWindow() {
	const settings = SettingsFile.getAll();

	if (settings.debug.showDevTools) {
		RustAPI.openDevTools().catch(console.error);
	}

	const currentWindow = getCurrentWindow();

	currentWindow.setSize(
		new PhysicalSize(settings.window.width ?? 1280, settings.window.height ?? 920).toLogical(await currentWindow.scaleFactor())
	);

	currentWindow.setPosition(
		new PhysicalPosition(settings.window.x, settings.window.y).toLogical(await currentWindow.scaleFactor())
	);

	if (settings.window.maximized) {
		currentWindow.maximize();
	}

	console.log('Window restored');
}
