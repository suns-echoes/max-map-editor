import { AppEvents } from '^events/app-events.ts';
import { debounce } from '^lib/flow-control/debounce.ts';


export function initWindowResizeEvent() {
	window.addEventListener('resize', debounce(function () {
		AppEvents.windowResizeSignal.dispatch();
	}, 50));
}
