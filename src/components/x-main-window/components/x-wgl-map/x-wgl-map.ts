import { resolveResource } from '@tauri-apps/api/path';

import template from './x-wgl-map.html';
import style from './x-wgl-map.style';

import { AppState } from '^src/state/app-state.ts';
import { WglMap } from '^components/x-main-window/components/x-wgl-map/wgl/wgl-map';
import { loadMapProject } from '^actions/load-map-project/load-map-project.ts';
import { signalValue } from '^utils/reactive/signalValue.ts';
import { AppSignals } from '^src/state/app-signals.ts';
import { effect } from '^utils/reactive/effect.ts';
import { throttle } from '^utils/flow-control/throttle.ts';


template.content.appendChild(style);


export class XWglMap extends HTMLElement {
	constructor() {
		super();
		const shadowRoot = this.attachShadow({ mode: 'open' });
        shadowRoot.appendChild(template.content.cloneNode(true));
	}

	connectedCallback() {
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

			setInterval(() => { console.log('render');
				wglMap.render();
			}, 100);

			makeCanvasInteractive(canvas, (dx, dy, dz) => {
				wglMap.moveCamera(dx, dy, dz);
				wglMap.render();
			});

			effect([AppSignals.windowResizeSignal], function () {
				wglMap.onCanvasResize();
			});

			effect.once([AppSignals.windowCloseRequested], async function WglMapCleanup() {
				wglMap.cleanup();
			});
		})();
	}
}


customElements.define('x-wgl-map', XWglMap);
