import { ReactiveTarget } from './internal/reactive-target.class.ts';
import type { EffectCleanupFn, EffectExecutorFn } from './types.ts';


export class Effect extends ReactiveTarget {
	public constructor(executor: EffectExecutorFn) {
		super('Effect', executor);
	}

	public notify() {
		if (this._queued) return this;
		this._queued = true;
		queueMicrotask(() => {
			this._cleanup?.();
			this._cleanup = this._executor();
			this._queued = false;
		});
		return this;
	}

	private _cleanup: EffectCleanupFn | void = undefined;
	private _queued = false;
}
