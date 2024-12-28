export function Perf(title: string) {
	const start = Date.now();
	return function stop() {
		const end = Date.now();
		console.info(`Perf: ${title}: ${end - start}ms`);
	}
}
