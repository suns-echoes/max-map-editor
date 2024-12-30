import { AppEvents } from '^events/app-events.ts';
import { debounce } from '^utils/flow-control/debounce.ts';


window.addEventListener('resize', debounce(function () {
	AppEvents.windowResizeSignal.set({
		innerWidth: window.innerWidth,
		innerHeight: window.innerHeight,
	});
}, 50));
