import { disableContextMenu } from '^utils/disable-context-menu/disable-context-menu.ts';
import { throttle } from '^utils/flow-control/throttle.ts';


const MOUSE_BUTTON_LEFT = 0;
const MOUSE_BUTTON_MIDDLE = 1;
const MOUSE_BUTTON_RIGHT = 2;


export function makeMapInteractive(canvas: HTMLCanvasElement, onDrag: (cursorX: number, cursorY: number, panDeltaX: number, panDeltaY: number, zoomDelta: number) => void) {
	disableContextMenu(canvas);

	let isPanning = false;
	let lastX = 0;
	let lastY = 0;

	function onMouseMove(event: MouseEvent) {
		if (isPanning) {
			const dx = event.offsetX - lastX;
			const dy = event.offsetY - lastY;

			lastX = event.offsetX;
			lastY = event.offsetY;

			onDrag(0, 0, dx, dy, 0);
		} else {
			onDrag(event.offsetX, event.offsetY, 0, 0, 0);
		}
	}

	canvas.addEventListener('mousedown', function (event) {
		isPanning = event.button === MOUSE_BUTTON_RIGHT;
		lastX = event.offsetX;
		lastY = event.offsetY;

		function onMouseUpOrLeave() {
			isPanning = false;
			canvas.removeEventListener('mouseup', onMouseUpOrLeave);
		}

		canvas.addEventListener('mouseup', onMouseUpOrLeave);
		canvas.addEventListener('mouseleave', onMouseUpOrLeave);
	});

	canvas.addEventListener('mousemove', onMouseMove);

	canvas.addEventListener('wheel', throttle(function (event) {
		onDrag(event.offsetX, event.offsetY, 0, 0, Math.sign(event.deltaY));
	}, 50));
}
