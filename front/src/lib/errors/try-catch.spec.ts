import { tryCatch, tryCatchAsync, tryCatchFn, toError, unwrap } from './try-catch.ts';


describe('tryCatch', () => {
	describe('When function succeeds', () => {
		it('should return undefined error and the value', () => {
			// Arrange
			const fn = () => 42;

			// Act
			const [error, value] = tryCatch(fn);

			// Assert
			assert.equal(error, undefined);
			assert.equal(value, 42);
		});
	});

	describe('When function throws an Error', () => {
		it('should return the error and undefined value', () => {
			// Arrange
			const fn = () => { throw new Error('test error'); };

			// Act
			const [error, value] = tryCatch(fn);

			// Assert
			assert.equal(value, undefined);
			assert.ok(error instanceof Error);
			assert.equal(error.message, 'test error');
		});
	});

	describe('When function throws a non-Error', () => {
		it('should convert string to Error', () => {
			// Arrange
			const fn = () => { throw 'string error'; };

			// Act
			const [error] = tryCatch(fn);

			// Assert
			assert.ok(error instanceof Error);
			assert.equal(error.message, 'string error');
		});
	});
});


describe('tryCatchAsync', () => {
	describe('When promise resolves', () => {
		it('should return undefined error and the value', async () => {
			// Arrange
			const promise = Promise.resolve(42);

			// Act
			const [error, value] = await tryCatchAsync(promise);

			// Assert
			assert.equal(error, undefined);
			assert.equal(value, 42);
		});
	});

	describe('When promise rejects', () => {
		it('should return the error and undefined value', async () => {
			// Arrange
			const promise = Promise.reject(new Error('async error'));

			// Act
			const [error, value] = await tryCatchAsync(promise);

			// Assert
			assert.equal(value, undefined);
			assert.ok(error instanceof Error);
			assert.equal(error.message, 'async error');
		});
	});
});


describe('tryCatchFn', () => {
	describe('When async function succeeds', () => {
		it('should return undefined error and the value', async () => {
			// Arrange
			const fn = async () => 'async result';

			// Act
			const [error, value] = await tryCatchFn(fn);

			// Assert
			assert.equal(error, undefined);
			assert.equal(value, 'async result');
		});
	});

	describe('When async function throws', () => {
		it('should return the error and undefined value', async () => {
			// Arrange
			const fn = async () => { throw new Error('fn error'); };

			// Act
			const [error, value] = await tryCatchFn(fn);

			// Assert
			assert.equal(value, undefined);
			assert.ok(error instanceof Error);
			assert.equal(error.message, 'fn error');
		});
	});
});


describe('toError', () => {
	describe('When given an Error', () => {
		it('should return the same Error instance', () => {
			// Arrange
			const original = new Error('original');

			// Act
			const result = toError(original);

			// Assert
			assert.strictEqual(result, original);
		});
	});

	describe('When given a string', () => {
		it('should wrap it in an Error', () => {
			// Arrange
			const message = 'string message';

			// Act
			const result = toError(message);

			// Assert
			assert.ok(result instanceof Error);
			assert.equal(result.message, 'string message');
		});
	});

	describe('When given other types', () => {
		it('should stringify numbers', () => {
			// Arrange & Act
			const result = toError(123);

			// Assert
			assert.ok(result instanceof Error);
			assert.equal(result.message, '123');
		});

		it('should stringify null', () => {
			// Arrange & Act
			const result = toError(null);

			// Assert
			assert.equal(result.message, 'null');
		});

		it('should stringify undefined', () => {
			// Arrange & Act
			const result = toError(undefined);

			// Assert
			assert.equal(result.message, 'undefined');
		});
	});
});


describe('unwrap', () => {
	describe('When result is success', () => {
		it('should return the value', () => {
			// Arrange
			const result: [undefined, number] = [undefined, 42];

			// Act
			const value = unwrap(result);

			// Assert
			assert.equal(value, 42);
		});
	});

	describe('When result is error', () => {
		it('should throw the error', () => {
			// Arrange
			const error = new Error('unwrap error');
			const result: [Error, undefined] = [error, undefined];

			// Act & Assert
			assert.throws(() => unwrap(result), /unwrap error/);
		});
	});

	describe('When used with tryCatch', () => {
		it('should return parsed value on valid JSON', () => {
			// Arrange
			const json = '{"a": 1}';

			// Act
			const value = unwrap(tryCatch(() => JSON.parse(json)));

			// Assert
			assert.deepEqual(value, { a: 1 });
		});
	});
});
