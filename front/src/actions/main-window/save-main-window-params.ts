import { getCurrentWindow } from '@tauri-apps/api/window';
import { saveWindowParams } from '^actions/generic-actions/window-actions/save-window-params.ts';
import { xlog } from '^lib/xlog/xlog.ts';


export async function saveMainWindowParams() {
	xlog.info('MainWindow::saveMainWindowParams');

	return saveWindowParams('main', getCurrentWindow());
}
