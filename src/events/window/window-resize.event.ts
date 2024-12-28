import { AppSignals } from '^src/state/app-signals.ts';
import { debounce } from '^utils/flow-control/debounce.ts';


window.addEventListener('resize', debounce(function () {
	AppSignals.windowResizeSignal.set({
		innerWidth: window.innerWidth,
		innerHeight: window.innerHeight,
	});
}, 50));
