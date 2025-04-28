import { ReactiveTarget } from './internal/reactive-target.class.ts';
import type { EffectCleanupFn, EffectExecutorFn } from './types.ts';


export class SyncEffect extends ReactiveTarget {
	constructor(executor: EffectExecutorFn) {
		super('SyncEffect', executor);
	}

	notify(trace: string | false = false) {
		if (this._trace) console.log(this._debug + '\n' + trace);
		this._cleanup?.();
		this._cleanup = this._executor();
		return this;
	}

	private _cleanup: EffectCleanupFn | void = undefined;
}
