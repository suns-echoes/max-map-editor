export function disableContextMenu(element: HTMLElement) {
	element.addEventListener('contextmenu', function (event) {
		event.preventDefault();
	});
}
