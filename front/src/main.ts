import { appDataDir, appLocalDataDir, resourceDir } from '@tauri-apps/api/path';

import { getAppVersion } from '^lib/info/info.ts';
import { xlog } from '^lib/xlog/xlog.ts';
import { initErrorBoundary, disposeErrorBoundary } from '^lib/errors/errors.ts';
import { hmrDispose, hmrAccept } from '^lib/hmr/hmr.ts';

import { SettingsFile } from '^storage/perma-storage/settings-file.ts';

import { saveMainWindowParams } from '^actions/main-window/save-main-window-params.ts';
import { showSetupScreen } from '^actions/main-window/setup-screen/show-setup-screen.ts';
import { showMainWindow } from '^actions/main-window/show-main-window.ts';


import './styles/global.style.css';
import './styles/index.style.css';


// Initialize global error boundary first
initErrorBoundary({
	alwaysLog: true,
});

// HMR support: cleanup error boundary when module is hot-replaced
hmrDispose(import.meta, disposeErrorBoundary);
hmrAccept(import.meta);


async function main() {
	xlog.info('M.A.X. Map Editor');
	xlog.info('version:', getAppVersion());
	xlog.info('$APPDATA:', await appDataDir());
	xlog.info('$APPLOCALDATA:', await appLocalDataDir());
	xlog.info('$RESOURCE:', await resourceDir());

	// TODO: Refactor `SettingsFile` logic, and place it in BE code, then expose the API
	await SettingsFile.sync();
	xlog.info('Settings:', SettingsFile.getAll());

	// SETUP

	// TODO: Refactor `SettingsFile` logic, and place it in BE code, then expose the API
	// TODO: is setup needed should check if MAX path is valid on every app startup
	if (SettingsFile.get('setup') || !SettingsFile.get('max')?.path) {
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
