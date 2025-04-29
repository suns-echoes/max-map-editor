import { ReactiveMiddleware } from './internal/reactive-middleware.class.ts';


export class SyncExpr<T> extends ReactiveMiddleware {
	value: T;

	constructor(executor: ExprExecutor<T>, initValue?: T) {
		super('SyncExpr', executor);
		this.value = initValue!;
	}

	notify(_: Promise<void>[], trace: string | false = false) {
		if (this._trace) console.log(this._debug + '\n' + trace);
		this.value = this._executor(this.value);
		this.dispatch(_);
		return this;
	}
}


export type ExprExecutor<T> = (value: T) => T;
