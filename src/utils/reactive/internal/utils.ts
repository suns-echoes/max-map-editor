export function trace() {
	const stack = new Error().stack ?? '';
	const lines = stack?.split('\n');
	return (lines[6] ?? lines[lines.length - 1])?.trim();
}
