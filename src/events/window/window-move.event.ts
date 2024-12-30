import { TauriEvent } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { isTauri } from '^tauri/is-tauri.ts';

import { Signal } from '^utils/reactive/signal.class.ts';


export const windowMoveSignal = Signal.empty();


if (isTauri) {
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
