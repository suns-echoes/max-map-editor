import type { ReactiveD } from './internal/reactive.types.ts';
import type { EffectExecutorFn } from './types.ts';
import { Effect } from './effect.class.ts';
import { Expr } from './expr.class.ts';
import { Signal } from './signal.class.ts';
import { Value } from './value.class.ts';
import { SyncEffect } from './sync-effect.class.ts';
import { SyncExpr } from './sync-expr.class.ts';


export class Scope implements ReactiveD.Scope {
	reactiveObjects = new Set<ReactiveD.Object>();

	destroy() {
		for (const item of this.reactiveObjects)
			item.destroy();
		this.reactiveObjects.clear();
		this.destroyed = true;
	}

	add(item: any) {
		this.reactiveObjects.add(item);
		return this;
	}

	delete(item: any) {
		this.reactiveObjects.delete(item);
		return this;
	}

	Signal() {
		return new Signal().scope(this);
	}

	Value<T>(value: T) {
		return new Value<T>(value).scope(this);
	}

	Expr<T>(executor: (value: T) => T, initValue?: T) {
		return new Expr<T>(executor, initValue).scope(this);
	}

	SyncExpr<T>(executor: (value: T) => T, initValue?: T) {
		return new SyncExpr<T>(executor, initValue).scope(this);
	}

	Effect(executor: EffectExecutorFn) {
		return new Effect(executor).scope(this);
	}

	SyncEffect(executor: EffectExecutorFn) {
		return new SyncEffect(executor).scope(this);
	}

	public destroyed = false;
}
