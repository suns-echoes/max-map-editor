import { restoreMainWindow } from './restore-main-window.ts';
import { xlog } from '^lib/xlog/xlog.ts';
import { initWindowEvents } from './init-window-events.ts';
import { MainWindow } from '../../ui/main-window/main-window.component.ts';


export async function showMainWindow() {
	xlog.info('App::showMainWindow');

	await restoreMainWindow();
	await initWindowEvents();

	document.body.appendChild(MainWindow().element);
}
