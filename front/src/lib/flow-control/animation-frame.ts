export function animationFrame(): Promise<void> {
	return new Promise<void>(resolve => {
		window.requestAnimationFrame(() => {
			resolve();
		});
	});
}
