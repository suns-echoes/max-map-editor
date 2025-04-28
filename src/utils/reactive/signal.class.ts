import { ReactiveSource } from './internal/reactive-source.class.ts';


export class Signal extends ReactiveSource {
	constructor() {
		super('Signal');
	}
}
