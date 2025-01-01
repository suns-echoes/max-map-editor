import { printDebugInfo } from '^utils/debug/debug.ts';
import { initWindowCloseEvent } from '^events/window/window-close.event.ts';
import { initWindowMoveEvent } from '^events/window/window-move.event.ts';
import { initWindowResizeEvent } from '^events/window/window-resize.event.ts';


export async function initWindowEvents() {
	await printDebugInfo('initWindowEvents');
	initWindowCloseEvent();
	initWindowMoveEvent();
	initWindowResizeEvent();
}
