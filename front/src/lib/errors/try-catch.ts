/**
 * Try-Catch Utilities
 *
 * Type-safe utilities for handling errors without try-catch boilerplate.
 * Inspired by Result types but kept simple for JavaScript ergonomics.
 */


// ============================================================================
// Types
// ============================================================================

/** Result tuple: [error, undefined] or [undefined, value] */
export type Result<T, E = Error> = [E, undefined] | [undefined, T];


// ============================================================================
// Synchronous
// ============================================================================

/**
 * Wraps a synchronous operation, returning a Result tuple.
 *
 * @example
 * const [error, data] = tryCatch(() => JSON.parse(jsonString));
 * if (error) {
 *   console.error('Parse failed:', error.message);
 *   return;
 * }
 * // data is typed correctly here
 */
export function tryCatch<T>(fn: () => T): Result<T> {
	try {
		return [undefined, fn()];
	} catch (e) {
		return [toError(e), undefined];
	}
}


// ============================================================================
// Asynchronous
// ============================================================================

/**
 * Wraps a promise, returning a Result tuple.
 *
 * @example
 * const [error, data] = await tryCatchAsync(fetchData());
 * if (error) {
 *   showError(error.message);
 *   return;
 * }
 * // data is typed correctly here
 */
export async function tryCatchAsync<T>(promise: Promise<T>): Promise<Result<T>> {
	try {
		return [undefined, await promise];
	} catch (e) {
		return [toError(e), undefined];
	}
}


/**
 * Wraps an async function, returning a Result tuple.
 *
 * @example
 * const [error, data] = await tryCatchFn(async () => {
 *   const response = await fetch(url);
 *   return response.json();
 * });
 */
export async function tryCatchFn<T>(fn: () => Promise<T>): Promise<Result<T>> {
	try {
		return [undefined, await fn()];
	} catch (e) {
		return [toError(e), undefined];
	}
}


// ============================================================================
// Utilities
// ============================================================================

/**
 * Converts unknown error to Error instance.
 */
export function toError(e: unknown): Error {
	if (e instanceof Error) return e;
	if (typeof e === 'string') return new Error(e);
	return new Error(String(e));
}


/**
 * Unwraps a Result, throwing if it contains an error.
 * Useful when you want to propagate errors up the call stack.
 *
 * @example
 * const data = unwrap(tryCatch(() => JSON.parse(json)));
 */
export function unwrap<T>(result: Result<T>): T {
	const [error, value] = result;
	if (error) throw error;
	return value;
}
