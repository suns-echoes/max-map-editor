export function throttle<T extends (...args: any[]) => void>(func: T, delay: number): (...args: Parameters<T>) => void {
	let timeout: ReturnType<typeof setTimeout> | null = null;

	return function throttled(...args: Parameters<T>): void {
		if (timeout) {
			return;
		}

		func(...args);

		timeout = setTimeout(function () {
			timeout = null;
		}, delay);
	};
}
