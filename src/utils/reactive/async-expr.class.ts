import type { ExprExecutor } from './types.ts';
import { ReactiveMiddleware } from './internal/reactive-middleware.class.ts';


export class AsyncExpr<T> extends ReactiveMiddleware {
	public value: T;

	public constructor(executor: ExprExecutor<T>, initValue?: T) {
		super('AsyncExpr', executor);
		this.value = initValue!;
	}

	public notify(asyncJobs: Promise<void>[]): this {
		if (this._queued) return this;
		this._queued = true;
		const { promise, resolve, reject } = Promise.withResolvers<void>();
		asyncJobs.push(promise);
		queueMicrotask(async () => {
			const localAsyncJobs: Promise<void>[] = [];
			try {
				this.value = this._executor(this.value);
				this.dispatch(localAsyncJobs);
				this._queued = false;
				await Promise.all(localAsyncJobs);
				resolve();
			} catch (error) {
				this._queued = false;
				// TODO: Return promises to the caller for better error handling
				for (let i = 0; i < localAsyncJobs.length; i++)
					asyncJobs.push(localAsyncJobs[i]);
				reject(error);
			}
		});
		return this;
	}

	private _queued = false;
}
