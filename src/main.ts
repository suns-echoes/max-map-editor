import { isTauri } from '@tauri-apps/api/core';
import { appDataDir, appLocalDataDir, resourceDir } from '^tauri-apps/api/path';
import { saveMainWindowParams } from '^actions/main-window/save-main-window-params.ts';
import { showSetupScreen } from '^actions/main-window/setup-screen/show-setup-screen';
import { showMainWindow } from '^actions/main-window/show-main-window.ts';
import { SettingsFile } from '^storage/perma-storage/settings-file.ts';
import { printDebugInfo } from '^utils/debug/debug.ts';

import './styles/global.style.css';
import './styles/index.style.css';


await printDebugInfo('M.A.X. Game Map Editor');
await printDebugInfo('version: ' + __ENV__.build_version);

await printDebugInfo('$APPDATA: ' + await appDataDir());
await printDebugInfo('$APPLOCALDATA: ' + await appLocalDataDir());
await printDebugInfo('$RESOURCE: ' + await resourceDir());


await SettingsFile.sync();
console.info('Settings:', SettingsFile.getAll());

//
// SETUP
//
if (isTauri() && (SettingsFile.get('setup') || !SettingsFile.get('max')?.path)) {
	await showSetupScreen();
	await saveMainWindowParams();
}

//
// MAIN WINDOW
//
await showMainWindow();
