import { restoreMainWindow } from './restore-main-window.ts';
import '^components/x-main-window/x-main-window.ts';
import type { XMainWindow as XMainWindowT } from '^components/x-main-window/x-main-window.ts';


async function initWindowEvents() {
	await import('^events/window/window-close.event.ts');
	await import('^events/window/window-move.event.ts');
	await import('^events/window/window-resize.event.ts');
}


export async function showMainWindow() {
	await restoreMainWindow();
	initWindowEvents();

	const xMainWindow = document.createElement<XMainWindowT>('x-main-window');
	document.body.appendChild(xMainWindow);
}
