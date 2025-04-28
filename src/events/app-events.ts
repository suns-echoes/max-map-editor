import { Signal } from '^utils/reactive/signal.class.ts';
import { Value } from '^utils/reactive/value.class.ts';


export const AppEvents = {
	windowCloseSignal: new Signal(),
	windowSize: new Value({
		innerWidth: window.innerWidth,
		innerHeight: window.innerHeight,
	}),
}
