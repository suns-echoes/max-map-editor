import { isTauri } from '@tauri-apps/api/core';
import { TauriEvent } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Signal } from '^lib/reactive/signal.class.ts';


export const windowMoveSignal = new Signal();

export async function initWindowMoveEvent() {
	if (isTauri()) {
		let timeout: any = null;

		getCurrentWindow().listen(TauriEvent.WINDOW_MOVED, function () {
			if (timeout) {
				clearTimeout(timeout);
			}

			timeout = setTimeout(function () {
				windowMoveSignal.dispatch();
			}, 250);
		});
	}
}
