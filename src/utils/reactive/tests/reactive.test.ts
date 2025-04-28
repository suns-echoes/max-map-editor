import { describe, it, mock } from 'node:test';
import assert from 'node:assert';

import { Effect } from '../effect.class.ts';
import { Expr } from '../expr.class.ts';
import { Value } from '../value.class.ts';
import { Reactive } from '../reactive.class.ts';
import { SyncExpr } from '../sync-expr.class.ts';
import { SyncEffect } from '../sync-effect.class.ts';


describe('Reactive', () => {
    describe('Value', () => {
		it('should notify effect', async () => {
			const value = new Value(10);

			const effectExec = mock.fn();
			new Effect(effectExec).watch([value]);

			value.set(20);

			assert.strictEqual(effectExec.mock.callCount(), 0);

			await Reactive.sync();

			assert.strictEqual(effectExec.mock.callCount(), 1);
		});

		it('should notify sync effect', async () => {
			const value = new Value(10);

			const syncEffectExec = mock.fn();
			new SyncEffect(syncEffectExec).watch([value]);

			value.set(20);

			assert.strictEqual(syncEffectExec.mock.callCount(), 1);
		});

		it('should notify expr', async () => {
			const value = new Value(10);

			const exprExec = mock.fn(() => value.value + 10);
			const expr = new Expr(exprExec, 0).watch([value]);

			value.set(20);

			assert.strictEqual(exprExec.mock.callCount(), 0);
			assert.strictEqual(expr.value, 0);

			await Reactive.sync();

			assert.strictEqual(exprExec.mock.callCount(), 1);
			assert.strictEqual(expr.value, 30);
		});

		it('should notify sync expr', async () => {
			const value = new Value(10);

			const exprExec = mock.fn((x) => x + value.value);
			const syncExpr = new SyncExpr(exprExec, 0).watch([value]);

			value.set(20);

			assert.strictEqual(exprExec.mock.callCount(), 1);
			assert.deepStrictEqual(exprExec.mock.calls[0].arguments, [0]);
			assert.strictEqual(exprExec.mock.calls[0].result, 20);
			assert.strictEqual(syncExpr.value, 20);
		});

		it('should notify all observers', async () => {
			let effectExec, syncEffectExec, exprExec, syncExprExec;

			const value = new Value(10);

			new Effect(effectExec = mock.fn()).watch([value]);
			new SyncEffect(syncEffectExec = mock.fn()).watch([value]);

			const expr = new Expr(exprExec = mock.fn((x: number) => x + value.value), 0).watch([value]);
			const syncExpr = new SyncExpr(syncExprExec = mock.fn((x: number) => x + value.value), 0).watch([value]);

			value.set(20);

			assert.strictEqual(effectExec.mock.callCount(), 0);
			assert.strictEqual(syncEffectExec.mock.callCount(), 1);
			assert.strictEqual(exprExec.mock.callCount(), 0);
			assert.strictEqual(expr.value, 0);
			assert.strictEqual(syncExprExec.mock.callCount(), 1);
			assert.strictEqual(syncExpr.value, 20);

			await Reactive.sync();

			assert.strictEqual(effectExec.mock.callCount(), 1);
			assert.strictEqual(syncEffectExec.mock.callCount(), 1);
			assert.strictEqual(exprExec.mock.callCount(), 1);
			assert.strictEqual(expr.value, 20);
			assert.strictEqual(syncExprExec.mock.callCount(), 1);
			assert.strictEqual(syncExpr.value, 20);
		});
	});

	describe('Expr', () => {
		it('should notify effect', async () => {
			const expr = new Expr(() => 0, 0);

			const effectExec = mock.fn();
			new Effect(effectExec).watch([expr]);

			expr.dispatch();

			assert.strictEqual(effectExec.mock.callCount(), 0);

			await Reactive.sync();

			assert.strictEqual(effectExec.mock.callCount(), 1);
		});

		it('should notify sync effect', async () => {
			const expr = new Expr(() => 0, 0);

			const syncEffectExec = mock.fn();
			new SyncEffect(syncEffectExec).watch([expr]);

			expr.dispatch();

			assert.strictEqual(syncEffectExec.mock.callCount(), 1);
		});

		it('should notify expr', async () => {
			const expr = new Expr(() => 0, 0);

			const exprExec = mock.fn(() => expr.value + 10);
			const expr2 = new Expr(exprExec, 0).watch([expr]);

			expr.dispatch();

			assert.strictEqual(exprExec.mock.callCount(), 0);
			assert.strictEqual(expr2.value, 0);

			await Reactive.sync();

			assert.strictEqual(exprExec.mock.callCount(), 1);
			assert.strictEqual(expr2.value, 10);
		});

		it('should notify sync expr', async () => {
			const expr = new Expr(() => 0, 0);

			const syncExprExec = mock.fn((x) => x + 10);
			const syncExpr = new SyncExpr(syncExprExec, 0).watch([expr]);

			expr.dispatch();

			assert.strictEqual(syncExprExec.mock.callCount(), 1);
			assert.deepStrictEqual(syncExprExec.mock.calls[0].arguments, [0]);
			assert.strictEqual(syncExprExec.mock.calls[0].result, 10);
			assert.strictEqual(syncExpr.value, 10);
		});

		it('should notify all observers', async () => {
			let effectExec, syncEffectExec, exprExec, syncExprExec;

			const expr = new Expr(() => 0, 0);

			new Effect(effectExec = mock.fn()).watch([expr]);
			new SyncEffect(syncEffectExec = mock.fn()).watch([expr]);

			const expr2 = new Expr(exprExec = mock.fn((x: number) => x + 10), 0).watch([expr]);
			const syncExpr = new SyncExpr(syncExprExec = mock.fn((x: number) => x + 10), 0).watch([expr]);

			expr.dispatch();

			assert.strictEqual(effectExec.mock.callCount(), 0);
			assert.strictEqual(syncEffectExec.mock.callCount(), 1);
			assert.strictEqual(exprExec.mock.callCount(), 0);
			assert.strictEqual(expr2.value, 0);
			assert.strictEqual(syncExprExec.mock.callCount(), 1);
			assert.strictEqual(syncExpr.value, 10);

			await Reactive.sync();

			assert.strictEqual(effectExec.mock.callCount(), 1);
			assert.strictEqual(syncEffectExec.mock.callCount(), 1);
			assert.strictEqual(exprExec.mock.callCount(), 1);
			assert.strictEqual(expr2.value, 10);
			assert.strictEqual(syncExprExec.mock.callCount(), 1);
			assert.strictEqual(syncExpr.value, 10);
		});
	});

	describe('SyncExpr', () => {
		it('should notify effect', async () => {
			const expr = new SyncExpr(() => 0, 0);

			const effectExec = mock.fn();
			new Effect(effectExec).watch([expr]);

			expr.dispatch();

			assert.strictEqual(effectExec.mock.callCount(), 0);

			await Reactive.sync();

			assert.strictEqual(effectExec.mock.callCount(), 1);
		});

		it('should notify sync effect', async () => {
			const expr = new SyncExpr(() => 0, 0);

			const syncEffectExec = mock.fn();
			new SyncEffect(syncEffectExec).watch([expr]);

			expr.dispatch();

			assert.strictEqual(syncEffectExec.mock.callCount(), 1);
		});

		it('should notify expr', async () => {
			const expr = new SyncExpr(() => 0, 0);

			const exprExec = mock.fn(() => expr.value + 10);
			const expr2 = new Expr(exprExec, 0).watch([expr]);

			expr.dispatch();

			assert.strictEqual(exprExec.mock.callCount(), 0);
			assert.strictEqual(expr2.value, 0);

			await Reactive.sync();

			assert.strictEqual(exprExec.mock.callCount(), 1);
			assert.strictEqual(expr2.value, 10);
		});

		it('should notify sync expr', async () => {
			const expr = new SyncExpr(() => 0, 0);

			const syncExprExec = mock.fn((x) => x + 10);
			const syncExpr = new SyncExpr(syncExprExec, 0).watch([expr]);

			expr.dispatch();

			assert.strictEqual(syncExprExec.mock.callCount(), 1);
			assert.deepStrictEqual(syncExprExec.mock.calls[0].arguments, [0]);
			assert.strictEqual(syncExprExec.mock.calls[0].result, 10);
			assert.strictEqual(syncExpr.value, 10);
		});

		it('should notify all observers', async () => {
			let effectExec, syncEffectExec, exprExec, syncExprExec;

			const expr = new SyncExpr(() => 0, 0);

			new Effect(effectExec = mock.fn()).watch([expr]);
			new SyncEffect(syncEffectExec = mock.fn()).watch([expr]);

			const expr2 = new Expr(exprExec = mock.fn((x: number) => x + 10), 0).watch([expr]);
			const syncExpr = new SyncExpr(syncExprExec = mock.fn((x: number) => x + 10), 0).watch([expr]);

			expr.dispatch();

			assert.strictEqual(effectExec.mock.callCount(), 0);
			assert.strictEqual(syncEffectExec.mock.callCount(), 1);
			assert.strictEqual(exprExec.mock.callCount(), 0);
			assert.strictEqual(expr2.value, 0);
			assert.strictEqual(syncExprExec.mock.callCount(), 1);
			assert.strictEqual(syncExpr.value, 10);

			await Reactive.sync();

			assert.strictEqual(effectExec.mock.callCount(), 1);
			assert.strictEqual(syncEffectExec.mock.callCount(), 1);
			assert.strictEqual(exprExec.mock.callCount(), 1);
			assert.strictEqual(expr2.value, 10);
			assert.strictEqual(syncExprExec.mock.callCount(), 1);
			assert.strictEqual(syncExpr.value, 10);
		});
	});

	describe('Effect', () => {
		it('should be notified by many values', async () => {
			const value1 = new Value(10);
			const value2 = new Value(20);

			const effectExec = mock.fn();
			new Effect(effectExec).watch([value1, value2]);

			value1.set(30);
			value2.set(40);

			assert.strictEqual(effectExec.mock.callCount(), 0);

			await Reactive.sync();

			assert.strictEqual(effectExec.mock.callCount(), 1);
		});

		it('should be notified by many exprs', async () => {
			const expr1 = new Expr(() => 10, 0);
			const expr2 = new Expr(() => 20, 0);

			const effectExec = mock.fn();
			new Effect(effectExec).watch([expr1, expr2]);

			expr1.dispatch();
			expr2.dispatch();

			assert.strictEqual(effectExec.mock.callCount(), 0);

			await Reactive.sync();

			assert.strictEqual(effectExec.mock.callCount(), 1);
		});

		it('should be notified by many sync exprs', async () => {
			const expr1 = new SyncExpr(() => 10, 0);
			const expr2 = new SyncExpr(() => 20, 0);

			const effectExec = mock.fn();
			new Effect(effectExec).watch([expr1, expr2]);

			expr1.dispatch();
			expr2.dispatch();

			assert.strictEqual(effectExec.mock.callCount(), 0);

			await Reactive.sync();

			assert.strictEqual(effectExec.mock.callCount(), 1);
		});

		it('should be notified by all observed sources', async () => {
			const value = new Value(10);
			const expr = new Expr(() => 10, 0);
			const syncExpr = new SyncExpr(() => 20, 0);

			const effectExec = mock.fn();
			new Effect(effectExec).watch([value, expr, syncExpr]);

			value.set(20);
			expr.dispatch();
			syncExpr.dispatch();

			assert.strictEqual(effectExec.mock.callCount(), 0);

			await Reactive.sync();

			assert.strictEqual(effectExec.mock.callCount(), 1);
		});
	});

	describe('SyncEffect', () => {
		it('should be notified by many values', async () => {
			const value1 = new Value(10);
			const value2 = new Value(20);

			const effectExec = mock.fn();
			new SyncEffect(effectExec).watch([value1, value2]);

			value1.set(30);

			assert.strictEqual(effectExec.mock.callCount(), 1);

			value2.set(40);

			assert.strictEqual(effectExec.mock.callCount(), 2);
		});

		it('should be notified by many exprs', async () => {
			const expr1 = new Expr(() => 10, 0);
			const expr2 = new Expr(() => 20, 0);

			const effectExec = mock.fn();
			new SyncEffect(effectExec).watch([expr1, expr2]);

			expr1.dispatch();

			assert.strictEqual(effectExec.mock.callCount(), 1);

			expr2.dispatch();

			assert.strictEqual(effectExec.mock.callCount(), 2);
		});

		it('should be notified by many sync exprs', async () => {
			const expr1 = new SyncExpr(() => 10, 0);
			const expr2 = new SyncExpr(() => 20, 0);

			const effectExec = mock.fn();
			new SyncEffect(effectExec).watch([expr1, expr2]);

			expr1.dispatch();

			assert.strictEqual(effectExec.mock.callCount(), 1);

			expr2.dispatch();

			assert.strictEqual(effectExec.mock.callCount(), 2);
		});

		it('should be notified by all observed sources', async () => {
			const value = new Value(10);
			const expr = new Expr(() => 10, 0);
			const syncExpr = new SyncExpr(() => 20, 0);

			const effectExec = mock.fn();
			new SyncEffect(effectExec).watch([value, expr, syncExpr]);

			value.set(20);

			assert.strictEqual(effectExec.mock.callCount(), 1);

			expr.dispatch();

			assert.strictEqual(effectExec.mock.callCount(), 2);

			syncExpr.dispatch();

			assert.strictEqual(effectExec.mock.callCount(), 3);
		});
	});
});
