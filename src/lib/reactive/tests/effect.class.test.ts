import assert from 'node:assert';
import { describe, it, mock } from 'node:test';

import { Effect } from '../effect.class.ts';
import { Reactive } from '../reactive.class.ts';


describe('Effect', () => {
	it('should initialize with the given executor function', () => {
		const executor = mock.fn();
		const effect = new Effect(executor);

		assert.strictEqual(effect['_executor'], executor);
		assert.strictEqual(effect['_queued'], false);
		assert.strictEqual(effect['_cleanup'], undefined);
	});

	it('should execute the executor() function when notify() is called', async () => {
		const executor = mock.fn();
		const effect = new Effect(executor);

		effect.notify();
		await Reactive.sync();

		assert.strictEqual(executor.mock.calls.length, 1);
	});

	it('should call the cleanup() before calling the executor() again', async () => {
		const cleanup = mock.fn();
		const executor = mock.fn(() => cleanup);
		const effect = new Effect(executor);

		effect.notify();
		await Reactive.sync();

		effect.notify();
		await Reactive.sync();

		assert.strictEqual(cleanup.mock.calls.length, 1);
		assert.strictEqual(executor.mock.calls.length, 2);
	});

	it('should not call the executor() if already queued', async () => {
		const executor = mock.fn();
		const effect = new Effect(executor);

		effect.notify();
		effect.notify();
		await Reactive.sync();

		assert.strictEqual(executor.mock.calls.length, 1);
	});
});
