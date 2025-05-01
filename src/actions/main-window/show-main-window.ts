import { restoreMainWindow } from './restore-main-window.ts';
import { printDebugInfo } from '^utils/debug/debug.ts';
import { initWindowEvents } from './init-window-events.ts';
import { MainWindow } from '^src/ui/main-window/main-window.component.ts';


export async function showMainWindow() {
	await printDebugInfo('App::showMainWindow');

	await restoreMainWindow();
	await initWindowEvents();

	document.body.appendChild(MainWindow().element);
}
