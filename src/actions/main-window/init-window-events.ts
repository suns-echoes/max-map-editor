import { printDebugInfo } from '^utils/debug/debug.ts';


export async function initWindowEvents() {
	await printDebugInfo('initWindowEvents');

	await import('^events/window/window-close.event.ts');
	await import('^events/window/window-move.event.ts');
	await import('^events/window/window-resize.event.ts');
}
