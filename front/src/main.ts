import { isTauri } from '@tauri-apps/api/core';
import { appDataDir, appLocalDataDir, resourceDir } from '^tauri-apps/api/path';
import { saveMainWindowParams } from '^actions/main-window/save-main-window-params.ts';
import { showSetupScreen } from '^actions/main-window/setup-screen/show-setup-screen';
import { showMainWindow } from '^actions/main-window/show-main-window.ts';
import { SettingsFile } from '^storage/perma-storage/settings-file.ts';
import { getAppVersion } from '^lib/info/info.ts';
import { xlog } from '^lib/xlog/xlog.ts';
import { initErrorBoundary } from '^lib/errors/errors.ts';

import './styles/global.style.css';
import './styles/index.style.css';


// Initialize global error boundary first
initErrorBoundary({
	alwaysLog: true,
});


async function main() {
	xlog.info('M.A.X. Map Editor');
	xlog.info('version:', getAppVersion());

	xlog.info('$APPDATA:', await appDataDir());
	xlog.info('$APPLOCALDATA:', await appLocalDataDir());
	xlog.info('$RESOURCE:', await resourceDir());


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
	xlog.error('Fatal error during application startup:', String(e));
}
