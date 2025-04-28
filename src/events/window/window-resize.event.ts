import { AppEvents } from '^events/app-events.ts';
import { debounce } from '^utils/flow-control/debounce.ts';


export function initWindowResizeEvent() {
	window.addEventListener('resize', debounce(function () {
		AppEvents.windowSize.set({
			innerWidth: window.innerWidth,
			innerHeight: window.innerHeight,
		});
	}, 50));
}
