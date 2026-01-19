import { debounce } from './debounce.ts';


describe('debounce', () => {
	describe('When called once', () => {
		it('should call the function after the wait time', async () => {
			return new Promise<void>((resolve) => {
				// Arrange
				let callCount = 0;
				const func = () => { callCount++; };
				const debouncedFunc = debounce(func, 20);

				// Act
				debouncedFunc();

				// Assert (after wait)
				setTimeout(() => {
					assert.equal(callCount, 1);
					resolve();
				}, 30);
			});
		});
	});

	describe('When called multiple times within wait period', () => {
		it('should only call the function once after the last call', async () => {
			return new Promise<void>((resolve) => {
				// Arrange
				let callCount = 0;
				const func = () => { callCount++; };
				const debouncedFunc = debounce(func, 20);

				// Act
				debouncedFunc();
				debouncedFunc();
				debouncedFunc();

				// Assert (after wait)
				setTimeout(() => {
					assert.equal(callCount, 1);
					resolve();
				}, 30);
			});
		});
	});

	describe('When called again after wait period', () => {
		it('should call the function twice total', async () => {
			return new Promise<void>((resolve) => {
				// Arrange
				let callCount = 0;
				const func = () => { callCount++; };
				const debouncedFunc = debounce(func, 20);

				// Act - first call
				debouncedFunc();

				// Act - second call after wait period
				setTimeout(() => {
					debouncedFunc();

					// Assert (after second wait)
					setTimeout(() => {
						assert.equal(callCount, 2);
						resolve();
					}, 30);
				}, 30);
			});
		});
	});
});
