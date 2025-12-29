import assert from 'node:assert';
import { describe, it, mock } from 'node:test';

import { Expr } from '../expr.class.ts';
import { Reactive } from '../reactive.class.ts';


describe('Expr', () => {
	it('should initialize with the given executor() and initial value', () => {
		const executor = mock.fn((value: number) => value + 10);
		const expr = new Expr(executor, 10);

		assert.strictEqual(expr.value, 10);
		assert.strictEqual(expr['_executor'], executor);
	});

	it('should call the executor() when notify() is called', async () => {
		const executor = mock.fn((value: number) => value + 10);
		const expr = new Expr(executor, 10);

		expr.notify([]);
		await Reactive.sync();

		assert.strictEqual(executor.mock.calls.length, 1);
		assert.strictEqual(executor.mock.calls[0].arguments[0], 10);
		assert.strictEqual(executor.mock.calls[0].result, 20);
		assert.strictEqual(expr.value, 20);
	});

	it('should not call the executor() if already queued', async () => {
		const executor = mock.fn((value: number) => value + 10);
		const expr = new Expr(executor, 10);

		expr.notify([]);
		expr.notify([]);
		await Reactive.sync();

		assert.strictEqual(executor.mock.calls.length, 1);
		assert.strictEqual(expr.value, 20);
	});

	it('should call dispatch() after updating the value in ', async () => {
		const executor = mock.fn((value: number) => value + 10);
		const mockDispatch = mock.fn();
		const expr = new Expr(executor, 20);
		expr.dispatch = mockDispatch as any;

		expr.notify([]);
		await Reactive.sync();

		assert.strictEqual(mockDispatch.mock.calls.length, 1);
	});

	it('should update once the value when notify() is called multiple times', async () => {
		const executor = mock.fn((value: number) => value + 10);
		const expr = new Expr(executor, 10);

		expr.notify([]);
		expr.notify([]);
		await Reactive.sync();

		assert.strictEqual(expr.value, 20);
		assert.strictEqual(executor.mock.calls.length, 1);
	});
});
