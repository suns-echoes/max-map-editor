import { TauriEvent } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';

import { saveMainWindowParams } from '^actions/main-window/save-main-window-params';
import { AppSignals } from '^src/state/app-signals.ts';


getCurrentWindow().listen(TauriEvent.WINDOW_CLOSE_REQUESTED, async function () {
	await saveMainWindowParams().catch(console.error);

	await AppSignals.windowCloseRequested.dispatch();

	getCurrentWindow().destroy();
});
