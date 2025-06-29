import { debounce } from './debounce.ts';


describe('debounce', () => {
	it('should call the function immediately if no timeout is set', async () => new Promise<void>((resolve) => {
		let callCount = 0;
		const func = () => { callCount++; };
		const debouncedFunc = debounce(func, 20);

		debouncedFunc();
		assert.equal(callCount, 1);
		setTimeout(resolve, 30);
	}));

	it('should not call the function again before the wait time', async () => new Promise<void>((resolve) => {
		let callCount = 0;
		const func = () => { callCount++; };
		const debouncedFunc = debounce(func, 20);

		debouncedFunc();
		debouncedFunc();
		assert.equal(callCount, 1);
		setTimeout(resolve, 30);
	}));

	it('should call the function again after the wait time', async () => new Promise<void>((resolve) => {
		let callCount = 0;
		const func = () => { callCount++; };
		const debouncedFunc = debounce(func, 20);

		debouncedFunc();
		setTimeout(() => {
			debouncedFunc();
			assert.equal(callCount, 2);
			resolve();
		}, 30);
	}));
});
