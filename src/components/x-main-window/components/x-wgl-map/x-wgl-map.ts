import { resolveTextResource } from '^tauri-apps/api/path.ts';

import template from './x-wgl-map.html';
import style from './x-wgl-map.style';

import { Effect } from '^utils/reactive/effect.class.ts';
import { Value } from '^utils/reactive/value.class.ts';
import { AppState } from '^state/app-state.ts';
import { WglMap } from '^components/x-main-window/components/x-wgl-map/wgl/wgl-map.ts';
import { loadMapProject } from '^actions/load-map-project/load-map-project.ts';
import { AppEvents } from '^events/app-events.ts';
import { printDebugInfo } from '^utils/debug/debug.ts';
import { makeMapInteractive } from './utils/make-map-interactive.ts';


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
			const shadowRoot = this.shadowRoot!;
			const canvas = shadowRoot.querySelector('canvas')!;

			await loadMapProject(await resolveTextResource('resources/maps/CRATER.template.json'));

			const wglMap = new WglMap(canvas);
			AppState.wglMap.set(wglMap);

			await Value.toPromise(AppState.mapSize, function (size) { return size !== null; });

			wglMap.enableAnimation();
			wglMap.render();

			makeMapInteractive(canvas, (cursorX, cursorY, panDeltaX, panDeltaY, zoomDelta) => {
				if (panDeltaX !== 0 || panDeltaY !== 0 || zoomDelta !== 0) {
					wglMap.moveCamera(panDeltaX, panDeltaY, zoomDelta, cursorX, cursorY);
				}
				if (cursorX !== 0 || cursorY !== 0) {
					wglMap.moveCursor(cursorX, cursorY);
				}
			});

			new Effect(function () {
				wglMap.onCanvasResize();
			}).watch([AppEvents.windowSize]);

			new Effect(function () {
				wglMap.cleanup();
			}).watch([AppEvents.windowCloseSignal]);
		})();
	}
}


printDebugInfo('register "x-wgl-map" web component');
customElements.define('x-wgl-map', XWglMap);
