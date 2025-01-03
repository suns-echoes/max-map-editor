/**
 * The signal class is a simple observable pattern implementation.
 * It allows to observe changes in the value and call attached observer functions.
 * The signal can be used to create custom signals with custom equality checks.
 */
export class AsyncSignal<T> {
	/**
	 * Creates the new signal that have no value.
	 * It can be used to dispatch events that have no value.
	 * @returns {AsyncSignal<void>}
	 */
	static empty(): AsyncSignal<undefined> {
		return new AsyncSignal(undefined, { equal: false });
	};

	/**
	 * Creates the new signal that compares the value of the source signal with the reference value.
	 * It will call observers with `true` if the values are equal and `false` otherwise.
	 */
	static comparator(source: AsyncSignal<any>, referenceValue: any): AsyncSignalComparator {
		const comparatorSignal = new AsyncSignal(source.equal(source.value, referenceValue)) as AsyncSignalComparator;
		comparatorSignal.referenceValue = referenceValue;
		source.observers.add(async function () {
			comparatorSignal.set(source.equal(source.value, comparatorSignal.referenceValue));
		});
		return comparatorSignal;
	};

	/**
	 * Creates the new signal with the initial value and optional custom equality check.
	 */
	constructor(initialValue: T, options?: AsyncSignalOptions) {
		this.value = typeof initialValue === 'function' ? initialValue() : initialValue;

		if (options?.equal === false)
			this.equal = function () { return false; };
		else if (typeof options?.equal === 'function')
			this.equal = options.equal;
	}

	/**
	 * Destroys the signal by removing all observers and clearing the value.
	 */
	destroy() {
		this.observers.clear();
		this.value = undefined!;
	}

	/**
	 * The signals value.
	 * *Note: assigning new value directly to this property will not call observers.*
	 */
	value: T = undefined!;

	/**
	 * The set of observer functions that will be called on value change.
	*/
	observers = new Set<AsyncSignalObserver<T>>();

	/**
	 * Calls all attached observer functions with the current value.
	 */
	async dispatch() {
		for (const observer of this.observers.values())
			await observer(this.value, this.value);
	}

	/**
	 * Sets the new value and calls all attached observer functions.
	 */
	async set(newValue: T) {
		if (this.equal(this.value, newValue))
			return;

		const prevValue = this.value;
		this.value = newValue;

		for (const observer of this.observers.values())
			await observer(prevValue, newValue);
	}

	/**
	 * The equality check function that is used to compare the previous and new values.
	 * It also can be set to `false` in which case the signal will call all observers even is new value is equal to previous value.
	 */
	equal = Object.is;
}


interface AsyncSignalOptions {
	equal?: boolean | ((a: any, b: any) => boolean);
}

type AsyncSignalObserver<T> = (prevValue: T, newValue: T) => Promise<void>;

interface AsyncSignalComparator extends AsyncSignal<boolean> {
	referenceValue: any;
}
