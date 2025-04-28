import assert from 'node:assert';
import { describe, it, mock } from 'node:test';

import { Value } from '../value.class.ts';


describe('Value Class', () => {
	it('should initialize with the given value', () => {
		const value = new Value(10);
		assert.strictEqual(value.value, 10);
	});

	it('should update the value using set()', () => {
		const value = new Value(5);
		value.set(20);
		assert.strictEqual(value.value, 20);
	});

	it('should update the value using apply() callback', () => {
		const value = new Value(10);
		value.apply((v) => v * 2);
		assert.strictEqual(value.value, 20);
	});

	it('should update the value using a custom updater function', () => {
		const value = new Value(10);
		value.updater((v) => v + 5);
		value.set(15);
		assert.strictEqual(value.value, 20);
	});

	it('should call dispatch() when set() is called', () => {
		const value = new Value(10);
		const mockDispatch = mock.fn();
		value.dispatch = mockDispatch as any;
		value.set(20);
		assert.strictEqual(mockDispatch.mock.calls.length, 1);
	});

	it('should call dispatch() when apply() is called', () => {
		const value = new Value(10);
		const mockDispatch = mock.fn();
		value.dispatch = mockDispatch as any;
		value.apply((v) => v * 2);
		assert.strictEqual(mockDispatch.mock.calls.length, 1);
	});

	it('should destroy the value and clean up properties', () => {
		const value = new Value(10).updater((v) => v + 5);
		value.destroy();
		assert.strictEqual(value.value, null);
		assert.strictEqual(value.set, null);
	});
});
