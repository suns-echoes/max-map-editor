export class Reactive {
	/**
	 * Resolves after queued microtasks.
	 */
	static sync(): Promise<void> {
		return new Promise(function Reactive_flush(resolve) {
			queueMicrotask(resolve);
		});
	}

	/**
	 * Is object a reactive source?
	 */
	isSource(object: any): boolean {
		// TODO: Implement me
		return !!(object && object._reactive);
	}

	/**
	 * Is object a reactive target?
	 */
	isTarget(object: any): boolean {
		// TODO: Implement me
		return !!(object && object._reactive);
	}
}
