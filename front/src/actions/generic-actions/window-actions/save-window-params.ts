import { getCurrentWindow } from '@tauri-apps/api/window';
import { SettingsFile } from '^storage/perma-storage/settings-file.ts';
import { printDebugInfo } from '^lib/debug/debug.ts';
import { RustAPI } from '^src/bff/rust-api';


type TWindow = ReturnType<typeof getCurrentWindow>;


export async function saveWindowParams(name: string, window: TWindow) {
	await printDebugInfo('saveMainWindowParams');

	const outerPosition = await window.outerPosition();
	const innerSize = await window.innerSize();
	const maximized = await window.isMaximized()

	const rect = maximized ? undefined : {
		x: outerPosition.x,
		y: outerPosition.y,
		width: innerSize.width,
		height: innerSize.height,
	};

	return SettingsFile.set({
		debug: {
			showDevTools: await RustAPI.isDevToolsOpen(),
		},
		window: {
			[name]: {
				...rect,
				maximized,
			},
		},
	}).sync();
}
