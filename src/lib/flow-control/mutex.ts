export function Mutex() {
	const asyncQueue: Promise<void>[] = [];

	return async function mutex(debugName?: string) {
		debugName && console.log('mutex:', debugName);
		let release: () => void = undefined!;
		const promise = new Promise<void>(function (resolve) {
			debugName && console.log('mutex release:', debugName);
			release = resolve;
		});
		const next = asyncQueue.shift();
		asyncQueue.push(promise);
		await next;

		return release;
	};
}
