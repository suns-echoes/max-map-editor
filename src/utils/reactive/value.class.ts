import { Effect } from './effect.class.ts';
import { ReactiveSource } from './internal/reactive-source.class.ts';


export class Value<T> extends ReactiveSource {
	static toPromise<T>(value: Value<T>, predicate?: (value: T) => boolean): Promise<T> {
		return new Promise(function (resolve) {
			const testFn = predicate ?? function (value: T) {
				return value !== null && value !== undefined;
			}
			const effect = new Effect(function () {
				if (testFn(value.value)) {
					resolve(value.value);
					value.targets.delete(effect);
				}
			}).watch([value]);
		});
	}

	value: T;

	constructor(value: T) {
		super('Value');
		this.value = value;
	}

	destroy() {
		this.value = null!;
		if (Object.hasOwn(this, 'set'))
			this.set = null!;
		super.destroy();
	}

	set(value: T) {
		this.value = value;
		this.dispatch();
		return this;
	}

	apply(fn: (value: T) => T) {
		this.set(fn(this.value));
		return this;
	}

	updater(fn: (value: T) => T) {
		this.set = function customSet(value: T) {
			this.value = fn(value);
			this.dispatch();
			return this;
		};
		return this;
	}
}
