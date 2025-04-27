import { Signal } from './signal.class.ts';


/**
 * Creates the reactive effect that will call the callback function when any of the trigger signals value change.
 *
 * @param callback The function that will be called when any of the triggers change.
 * @param triggers The array of signals that will trigger the callback function.
 * @param [options] The optional effect options.
 * @param [options.immediate] The optional effect options. Set to `true` to call the callback immediately after the effect is created. Default is `false`.
 * @param [options.batchUpdates] Optional effect options. Set to `true` to call the callback only once in the microtask queue. Default is `false`.
 * @param [options.executor] Optional effect options. Set to a custom executor function to override the default behavior.
 * @returns The destroy function that will remove the effect and call the cleanup function if returned by callback.
 */
export function effect(triggers: Array<Signal<any>>, callback: AnyFunction, options?: EffectOptions) {
	const { immediate = false, batchUpdates = false, executor } = options || {};

	function destroy() {
		for (let i = 0; i < triggers.length; i++)
			triggers[i].observers.delete(effectExecutor);

		cleanup?.();
	}

	const effectExecutor = executor?.(triggers, callback, destroy, batchUpdates) ?? createExecutor(callback, destroy, batchUpdates);
	let cleanup: (() => void) | undefined = undefined;

	for (let i = 0; i < triggers.length; i++)
		triggers[i].observers.add(effectExecutor);

	if (immediate)
		effectExecutor();

	return destroy;
}


function createExecutor(callback: AnyFunction, destroy: () => void, batchUpdates: boolean) {
	let cleanup: (() => void) | undefined = undefined;
	if (batchUpdates) {
		let isPending = false;
		return function () {
			if (isPending) return;
			isPending = true;
			queueMicrotask(function () {
				isPending = false;
				cleanup?.();
				cleanup = callback(destroy);
			});
		};
	} else {
		return function () {
			cleanup?.();
			cleanup = callback(destroy);
		};
	}
}


effect.once = function (triggers: Signal<any>[], callback: AnyFunction, options?: EffectOptions) {
	effect(triggers, function (destroy) {
		destroy();
		return callback();
	}, options);
};


/**
 * Creates the reactive effect that will call the callback function when all of the trigger signals have non-null values.
 *
 * @param callback The function that will be called when any of the triggers change.
 * @param triggers The array of signals that will trigger the callback function.
 * @param [options] The optional effect options.
 * @param [options.immediate] The optional effect options. Set to `true` to call the callback immediately after the effect is created. Default is `false`.
 * @param [options.batchUpdates] Optional effect options. Set to `true` to call the callback only once in the microtask queue. Default is `false`.
 * @param [options.executor] Optional effect options. Set to a custom executor function to override the default behavior.
 * @returns The destroy function that will remove the effect and call the cleanup function if returned by callback.
 */
effect.onNonNullValues = function <const T extends Signal<any>[]>(triggers: T, callback: (values: SignalNonNullValues<T>, destroy: () => void) => ((() => void) | void), options?: EffectOptions) {
	return effect(triggers, callback, {
		...options,
		executor: createOnNonNullValuesExecutor,
	});
}

function createOnNonNullValuesExecutor(triggers: Signal<any>[], callback: AnyFunction, destroy: () => void, batchUpdates: boolean) {
	let cleanup: (() => void) | undefined = undefined;
	if (batchUpdates) {
		let isPending = false;
		return function () {
			if (isPending) return;
			isPending = true;
			queueMicrotask(function () {
				const values = new Array(triggers.length);
				for (let i = 0; i < triggers.length; i++) {
					const signal = triggers[i];
					if (signal.value === null || signal.value === undefined) {
						return;
					}
					values[i] = signal.value;
				}
				isPending = false;
				cleanup?.();
				cleanup = callback(values, destroy);
			});
		}
	} else {
		return function () {
			const values = new Array(triggers.length);
			for (let i = 0; i < triggers.length; i++) {
				const signal = triggers[i];
				if (signal.value === null || signal.value === undefined) {
					return;
				}
				values[i] = signal.value;
			}
			cleanup?.();
			cleanup = callback(values, destroy);
		}
	}
}


/*
	const { immediate = false, batchUpdates = false } = options || {};

	let executor: () => void = undefined!;
	let cleanup: (() => void) | void = undefined;
	let pending = false;

	function destroy() {
		for (let i = 0; i < triggers.length; i++)
			triggers[i].observers.delete(executor);

		cleanup?.();
	}

	executor = batchUpdates
		? function () {
			if (pending) return;
			pending = true;
			queueMicrotask(function () {
				const values = new Array(triggers.length) as SignalNonNullValues<T>;
				for (let i = 0; i < triggers.length; i++) {
					const signal = triggers[i];
					if (signal.value === null || signal.value === undefined) {
						return;
					}
					values[i] = signal.value;
				}
				pending = false;
				cleanup?.();
				cleanup = callback(values, destroy);
			});
		}
		: function () {
			const values = new Array(triggers.length) as SignalNonNullValues<T>;
			for (let i = 0; i < triggers.length; i++) {
				const signal = triggers[i];
				if (signal.value === null || signal.value === undefined) {
					return;
				}
				values[i] = signal.value;
			}
			cleanup?.();
			cleanup = callback(values, destroy);
		};

	for (let i = 0; i < triggers.length; i++)
		triggers[i].observers.add(executor);

	if (immediate)
		executor();

	return destroy;
}
*/

type SignalNonNullValues<T> = {
	[K in keyof T]: T[K] extends Signal<infer U> ? U extends null ? never : U : never;
};

interface EffectOptions {
	immediate?: boolean,
	batchUpdates?: boolean,
	executor?: (triggers: Signal<any>[], callback: AnyFunction, destroy: () => void, batchUpdates: boolean) => void,
}
