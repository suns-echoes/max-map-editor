import { getCurrentWindow } from '@tauri-apps/api/window';
import { saveWindowParams } from '^actions/generic-actions/window-actions/save-window-params.ts';
import { printDebugInfo } from '^utils/debug/debug.ts';


export async function saveMainWindowParams() {
	await printDebugInfo('MainWindow::saveMainWindowParams');

	return saveWindowParams('main', getCurrentWindow());
}
