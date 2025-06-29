import type { AsyncEffectExecutorFn } from './types.ts';
import { ReactiveTarget } from './internal/reactive-target.class.ts';


export class AsyncEffect extends ReactiveTarget {
	public constructor(executor: AsyncEffectExecutorFn) {
		super('AsyncEffect', executor);
	}

	public notify(asyncJobs: Promise<void>[]) {
		if (this._queued) return this;
		this._queued = true;
		const { promise, resolve, reject } = Promise.withResolvers<void>();
		asyncJobs.push(promise);
		queueMicrotask(async () => {
			try {
				await this._executor();
				this._queued = false;
				resolve();
			} catch (error) {
				this._queued = false;
				reject(error);
			}
		});
		return this;
	}

	private _queued = false;
}
