import { AsyncSignal } from '^utils/reactive/async-signal.ts';
import { Signal } from '^utils/reactive/signal.class.ts';


export const AppSignals = {
	windowCloseRequested: AsyncSignal.empty(),
	windowResizeSignal: new Signal({
		innerWidth: window.innerWidth,
		innerHeight: window.innerHeight,
	}),
}
