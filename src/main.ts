import { isTauri } from '@tauri-apps/api/core';
import { appDataDir, appLocalDataDir, resourceDir } from '^tauri-apps/api/path';
import { saveMainWindowParams } from '^actions/main-window/save-main-window-params.ts';
import { showSetupScreen } from '^actions/setup-screen/show-setup-screen.ts';
import { showMainWindow } from '^actions/main-window/show-main-window.ts';
import { SettingsFile } from '^storage/perma-storage/settings-file.ts';
import { printDebugInfo } from '^utils/debug/debug.ts';

import globalStyle from './styles/global.style';
import style from './styles/index.style';


document.head.appendChild(globalStyle);
document.head.appendChild(style);


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
