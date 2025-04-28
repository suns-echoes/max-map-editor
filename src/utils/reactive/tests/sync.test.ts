import assert from 'node:assert';
import { describe, it, mock } from 'node:test';
import { Reactive } from '../reactive.class.ts';
import { AsyncEffect } from '../async-effect.class.ts';
import { AsyncExpr } from '../async-expr.class.ts';
import { Effect } from '../effect.class.ts';
import { Expr } from '../expr.class.ts';
import { Value } from '../value.class.ts';


function sleep(ms: number) {
	return new Promise<void>((resolve) => setTimeout(resolve, ms));
}


describe('Reactive sync', () => {
	it('should return a promise', () => {
		const result = Reactive.sync();
		assert.strictEqual(result instanceof Promise, true);
	});

	it('should resolve after flat microtasks', async () => {
		const value = new Value(0);

		const exprExec = mock.fn();
		new Expr(exprExec).watch([value]);

		const effectExec = mock.fn();
		new Effect(effectExec).watch([value]);

		assert.strictEqual(exprExec.mock.calls.length, 0);
		assert.strictEqual(effectExec.mock.calls.length, 0);

		await value.set(1).sync();

		assert.strictEqual(exprExec.mock.calls.length, 1);
		assert.strictEqual(effectExec.mock.calls.length, 1);
	});

	it('should resolve after nested microtasks', async () => {
		const value = new Value(0);

		const exprExec = mock.fn();
		const expr = new Expr(exprExec).watch([value]);

		const effectExec = mock.fn();
		new Effect(effectExec).watch([expr]);

		assert.strictEqual(exprExec.mock.calls.length, 0);
		assert.strictEqual(effectExec.mock.calls.length, 0);

		await value.set(1).sync();

		assert.strictEqual(exprExec.mock.calls.length, 1);
		assert.strictEqual(effectExec.mock.calls.length, 1);
	});

	it('should resolve after async tasks', async () => {
		const value = new Value(0);

		const asyncExprExec = mock.fn(async () => await sleep(0));
		const expr = new AsyncExpr(asyncExprExec).watch([value]);

		const asyncEffectExec = mock.fn(async () => await sleep(0));
		new AsyncEffect(asyncEffectExec).watch([expr]);

		assert.strictEqual(asyncExprExec.mock.calls.length, 0);
		assert.strictEqual(asyncEffectExec.mock.calls.length, 0);

		await value.set(1).sync();

		assert.strictEqual(asyncExprExec.mock.calls.length, 1);
		assert.strictEqual(asyncEffectExec.mock.calls.length, 1);
	});
});
