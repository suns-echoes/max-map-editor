import { resolveResource } from '^tauri-apps/api/path.ts';

import template from './x-wgl-map.html';
import style from './x-wgl-map.style';

import { AppState } from '^state/app-state.ts';
import { WglMap } from '^components/x-main-window/components/x-wgl-map/wgl/wgl-map.ts';
import { loadMapProject } from '^actions/load-map-project/load-map-project.ts';
import { signalValue } from '^utils/reactive/signalValue.ts';
import { AppEvents } from '^events/app-events.ts';
import { effect } from '^utils/reactive/effect.ts';
import { throttle } from '^utils/flow-control/throttle.ts';
import { printDebugInfo } from '^utils/debug/debug.ts';


template.content.appendChild(style);


export class XWglMap extends HTMLElement {
	constructor() {
		printDebugInfo('XWglMap::constructor');

		super();
		const shadowRoot = this.attachShadow({ mode: 'open' });
        shadowRoot.appendChild(template.content.cloneNode(true));
	}

	connectedCallback() {
		printDebugInfo('XWglMap::connectedCallback');

		(async () => {
			function makeCanvasInteractive(canvas: HTMLCanvasElement, onDrag: (dx: number, dy: number, dz: number) => void) {
				canvas.addEventListener('mousedown', function (event) {
					let lastX = event.offsetX;
					let lastY = event.offsetY;

					function onMouseMove(event: MouseEvent) {
						const dx = event.offsetX - lastX;
						const dy = event.offsetY - lastY;

						lastX = event.offsetX;
						lastY = event.offsetY;

						onDrag(dx, dy, 0);
					}

					function onMouseUpOrLeave() {
						canvas.removeEventListener('mousemove', onMouseMove);
						canvas.removeEventListener('mouseup', onMouseUpOrLeave);
					}

					canvas.addEventListener('mousemove', onMouseMove);
					canvas.addEventListener('mouseup', onMouseUpOrLeave);
					canvas.addEventListener('mouseleave', onMouseUpOrLeave);
				});

				canvas.addEventListener('wheel', throttle(function (event) {
					onDrag(0, 0, Math.sign(event.deltaY));
				}, 50));
			}
			const shadowRoot = this.shadowRoot!;
			const canvas = shadowRoot.querySelector('canvas')!;

			await loadMapProject(await resolveResource('resources/maps/CRATER.template.json'));

			const wglMap = new WglMap(canvas);
			AppState.wglMap.set(wglMap);

			await signalValue(AppState.mapSize, function (size) { return size !== null; });

			wglMap.render();

			wglMap.enableAnimation();

			makeCanvasInteractive(canvas, (dx, dy, dz) => {
				wglMap.moveCamera(dx, dy, dz);
				wglMap.render();
			});

			effect([AppEvents.windowResizeSignal], function () {
				wglMap.onCanvasResize();
			});

			effect.once([AppEvents.windowCloseRequested], async function WglMapCleanup() {
				wglMap.cleanup();
			});
		})();
	}
}


printDebugInfo('register "x-wgl-map" web component');
customElements.define('x-wgl-map', XWglMap);
