import { Value } from '^reactive/value.ts';


export const AppEvents = {
	windowCloseSignal: new Value<void>(undefined),
	windowResizeSignal: new Value<void>(undefined),
}
