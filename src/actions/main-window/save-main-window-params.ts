import { getCurrentWindow } from '@tauri-apps/api/window';

import { SettingsFile } from '^storage/perma-storage/settings-file.ts';
import { printDebugInfo } from '^utils/debug/debug.ts';
import { RustAPI } from '^utils/rust-api.ts';


export async function saveMainWindowParams() {
	await printDebugInfo('saveMainWindowParams');

	const currentWindow = getCurrentWindow();
	const outerPosition = await currentWindow.outerPosition();
	const innerSize = await currentWindow.innerSize();
	const maximized = await currentWindow.isMaximized()

	const rect = maximized ? undefined : {
		x: outerPosition.x,
		y: outerPosition.y,
		width: innerSize.width,
		height: innerSize.height,
	};

	SettingsFile.set({
		debug: {
			showDevTools: await RustAPI.isDevToolsOpen(),
		},
		window: {
			...rect,
			maximized,
		},
	}).sync();
}
