import type { ExprExecutor } from './types.ts';
import { ReactiveMiddleware } from './internal/reactive-middleware.class.ts';


export class Expr<T> extends ReactiveMiddleware {
	public value: T;

	public constructor(executor: ExprExecutor<T>, initValue?: T) {
		super('Expr', executor);
		this.value = initValue!;
	}

	public notify(): this {
		if (this._queued) return this;
		this._queued = true;
		queueMicrotask(() => {
			this.value = this._executor(this.value);
			this.dispatch();
			this._queued = false;
		});
		return this;
	}

	private _queued = false;
}
