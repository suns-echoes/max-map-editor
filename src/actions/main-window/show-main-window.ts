import { restoreMainWindow } from './restore-main-window.ts';
import { printDebugInfo } from '^utils/debug/debug.ts';
import { initWindowEvents } from './init-window-events.ts';

import type { XMainWindow as XMainWindowT } from '^components/x-main-window/x-main-window.ts';
import '^components/x-main-window/x-main-window.ts';


export async function showMainWindow() {
	await printDebugInfo('showMainWindow');

	await restoreMainWindow();
	await initWindowEvents();

	const xMainWindow = document.createElement<XMainWindowT>('x-main-window');
	document.body.appendChild(xMainWindow);
}
