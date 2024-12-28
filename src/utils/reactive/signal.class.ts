/**
 * The signal class is a simple observable pattern implementation.
 * It allows to observe changes in the value and call attached observer functions.
 * The signal can be used to create custom signals with custom equality checks.
 */
export class Signal<T> {
	/**
	 * Creates the new signal that have no value.
	 * It can be used to dispatch events that have no value.
	 * @returns {Signal<void>}
	 */
	static empty(): Signal<undefined> {
		return new Signal(undefined, { equal: false });
	};

	/**
	 * Creates the new signal that compares the value of the source signal with the reference value.
	 * It will call observers with `true` if the values are equal and `false` otherwise.
	 */
	static comparator(source: Signal<any>, referenceValue: any): SignalComparator {
		const comparatorSignal = new Signal(source.equal(source.value, referenceValue)) as SignalComparator;
		comparatorSignal.referenceValue = referenceValue;
		source.observers.add(function () {
			comparatorSignal.set(source.equal(source.value, comparatorSignal.referenceValue));
		});
		return comparatorSignal;
	};

	/**
	 * Creates the new signal with the initial value and optional custom equality check.
	 */
	constructor(initialValue: T, options?: SignalOptions) {
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
	observers = new Set<SignalObserver<T>>();

	/**
	 * Calls all attached observer functions with the current value.
	 */
	dispatch() {
		for (const observer of this.observers.values())
			observer(this.value, this.value);
	}

	/**
	 * Sets the new value and calls all attached observer functions.
	 */
	set(newValue: T) {
		if (this.equal(this.value, newValue))
			return;

		const prevValue = this.value;
		this.value = newValue;

		for (const observer of this.observers.values())
			observer(prevValue, newValue);
	}

	/**
	 * The equality check function that is used to compare the previous and new values.
	 * It also can be set to `false` in which case the signal will call all observers even is new value is equal to previous value.
	 */
	equal = Object.is;
}


interface SignalOptions {
	equal?: boolean | ((a: any, b: any) => boolean);
}

type SignalObserver<T> = (prevValue: T, newValue: T) => void;

interface SignalComparator extends Signal<boolean> {
	referenceValue: any;
}
