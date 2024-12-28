export function debounce<T extends (...args: any[]) => void>(func: T, wait: number): (...args: Parameters<T>) => void {
	let timeout: ReturnType<typeof setTimeout> | null = null;

	return function debounced(...args: Parameters<T>): void {
		if (timeout) {
			clearTimeout(timeout);
		}

		timeout = setTimeout(function () {
			func(...args);
			timeout = null
		}, wait);
	};
}
