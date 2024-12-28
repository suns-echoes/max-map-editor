import type { Signal } from './signal.class.ts';


export function signalValue<T>(signal: Signal<T>, expectedValue: T): Promise<T>;
export function signalValue<T>(signal: Signal<T>, valueTester: (signalValue: T) => boolean): Promise<T>;
export function signalValue<T>(signal: Signal<T>, expectedValue: any): Promise<T> {
	const test = typeof expectedValue === 'function'
		? expectedValue
		: function (value: T) { return value === expectedValue };

	return new Promise<T>(function (resolve) {
		function observeValue(_: T, currentValue: T) {
			if (test(currentValue)) {
				signal.observers.delete(observeValue);
				resolve(currentValue);
			}
		}

		signal.observers.add(observeValue);
	});
}
