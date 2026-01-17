import { isTauri } from '@tauri-apps/api/core';
import { TauriEvent } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Value } from '^reactive/value.ts';


export const windowMoveSignal = new Value<void>(undefined);

export async function initWindowMoveEvent() {
	if (isTauri()) {
		let timeout: any = null;

		getCurrentWindow().listen(TauriEvent.WINDOW_MOVED, function () {
			if (timeout) {
				clearTimeout(timeout);
			}

			timeout = setTimeout(function () {
				windowMoveSignal.set(undefined);
			}, 250);
		});
	}
}
