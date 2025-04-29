import assert from 'node:assert';
import { describe, it, mock } from 'node:test';

import { SyncExpr } from '../sync-expr.class.ts';


describe('SyncExpr', () => {
	it('should initialize with the given executor() and initial value', () => {
		const executor = mock.fn((value: number) => value + 10);
		const syncExpr = new SyncExpr(executor, 10);

		assert.strictEqual(syncExpr.value, 10);
		assert.strictEqual(syncExpr['_executor'], executor);
	});

	it('should call the executor() when notify() is called', () => {
		const executor = mock.fn((value: number) => value + 10);
		const syncExpr = new SyncExpr(executor, 10);

		syncExpr.notify([]);

		assert.strictEqual(executor.mock.calls.length, 1);
		assert.strictEqual(executor.mock.calls[0].arguments[0], 10);
		assert.strictEqual(executor.mock.calls[0].result, 20);
		assert.strictEqual(syncExpr.value, 20);
	});

	it('should call dispatch() after updating the value in notify()', () => {
		const executor = mock.fn((value: number) => value + 10);
		const mockDispatch = mock.fn();
		const syncExpr = new SyncExpr(executor, 10);
		syncExpr.dispatch = mockDispatch as any;

		syncExpr.notify([]);

		assert.strictEqual(mockDispatch.mock.calls.length, 1);
	});
});
