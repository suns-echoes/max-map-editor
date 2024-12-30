import { TauriEvent } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';

import { saveMainWindowParams } from '^actions/main-window/save-main-window-params';
import { AppEvents } from '^events/app-events.ts';
import { isTauri } from '^tauri/is-tauri.ts';


if (isTauri) {
	getCurrentWindow().listen(TauriEvent.WINDOW_CLOSE_REQUESTED, async function () {
		await saveMainWindowParams().catch(console.error);

		await AppEvents.windowCloseRequested.dispatch();

		getCurrentWindow().destroy();
	});
}
