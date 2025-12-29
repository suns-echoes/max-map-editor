/**
 * Check if a promise is settled (fulfilled or rejected).
 * This function will not wait for the promise to settle, it will return promise immediately.
 */
export async function isPromiseSettled(promise: Promise<any>): Promise<boolean> {
	const PENDING = Symbol();
	return (await Promise.race([promise, Promise.resolve(PENDING)])) !== PENDING;
}
