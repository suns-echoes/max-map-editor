import assert from 'node:assert';
import { describe, it, mock } from 'node:test';

import { SyncEffect } from '../sync-effect.class.ts';


describe('SyncEffect', () => {
	it('should initialize with the given executor function', () => {
		const executor = mock.fn();
		const syncEffect = new SyncEffect(executor);

		assert.strictEqual(syncEffect['_executor'], executor);
		assert.strictEqual(syncEffect['_cleanup'], undefined);
	});

	it('should execute the executor() function when notify() is called', () => {
		const executor = mock.fn();
		const syncEffect = new SyncEffect(executor);

		syncEffect.notify();

		assert.strictEqual(executor.mock.calls.length, 1);
	});

	it('should call the cleanup() before executing the executor again', () => {
		const cleanup = mock.fn();
		const executor = mock.fn(() => cleanup);
		const syncEffect = new SyncEffect(executor);

		syncEffect.notify();
		syncEffect.notify();

		assert.strictEqual(cleanup.mock.calls.length, 1);
		assert.strictEqual(executor.mock.calls.length, 2);
	});
});
