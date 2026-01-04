import { isTauri } from '@tauri-apps/api/core';
import { appDataDir, appLocalDataDir, resourceDir } from '^tauri-apps/api/path';
import { saveMainWindowParams } from '^actions/main-window/save-main-window-params.ts';
import { showSetupScreen } from '^actions/main-window/setup-screen/show-setup-screen';
import { showMainWindow } from '^actions/main-window/show-main-window.ts';
import { SettingsFile } from '^storage/perma-storage/settings-file.ts';
import { printDebugInfo } from '^lib/debug/debug.ts';
import { getAppVersion } from '^lib/info/info.ts';

import './styles/global.style.css';
import './styles/index.style.css';
import { xlog } from '^lib/xlog/xlog.ts';


async function main() {
	await printDebugInfo('M.A.X. Map Editor');
	await printDebugInfo('version: ' + getAppVersion());

	await printDebugInfo('$APPDATA: ' + await appDataDir());
	await printDebugInfo('$APPLOCALDATA: ' + await appLocalDataDir());
	await printDebugInfo('$RESOURCE: ' + await resourceDir());


	await SettingsFile.sync();
	console.info('Settings:', SettingsFile.getAll());


	// SETUP

	if (isTauri() && (SettingsFile.get('setup') || !SettingsFile.get('max')?.path)) {
		await showSetupScreen();
		await saveMainWindowParams();
	}


	// MAIN WINDOW

	await showMainWindow();
}

try {
	xlog.info('Front starting up...');

	await main();
} catch(e) {
	xlog.error('Fatal error during application startup:', e);
}
