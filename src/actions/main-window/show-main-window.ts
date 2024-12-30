import { restoreMainWindow } from './restore-main-window.ts';
import { printDebugInfo } from '^utils/debug/debug.ts';
import { initWindowEvents } from './init-window-events.ts';

import type { XMainWindow } from '^components/x-main-window/x-main-window.ts';
import '^components/x-main-window/x-main-window.ts';


export async function showMainWindow() {
	await printDebugInfo('showMainWindow');

	await restoreMainWindow();
	await initWindowEvents();

	const xMainWindow = document.createElement<XMainWindow>('x-main-window');
	document.body.appendChild(xMainWindow);
}
