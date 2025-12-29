import { Effect } from './effect.class.ts';
import { ReactiveSource } from './internal/reactive-source.class.ts';


export class Signal extends ReactiveSource {
	static toPromise(signal: Signal): Promise<void> {
		return new Promise(function (resolve) {
			const effect = new Effect(function onSignal() {
				effect.destroy();
				resolve();
			}).watch([signal]);
		});
	}

	constructor() {
		super('Signal');
	}
}
