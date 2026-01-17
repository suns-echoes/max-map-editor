import { loadMapProject } from '^actions/load-map-project/load-map-project.ts';
import { AppEvents } from '^events/app-events.ts';
import { AppState } from '^state/app-state.ts';
import { resolveTextResource } from '^tauri-apps/api/path.ts';
import { printDebugInfo } from '^lib/debug/debug.ts';
import { Effect } from '^reactive/effect.ts';
import { Canvas, Div, Section } from '^reactive/reactive-node.elements.ts';

import style from './wgl-map.module.css';
import { BigInset } from '../../components/frames/big-inset.component.ts';
import { WglMap } from './wgl/wgl-map.ts';
import { makeMapInteractive } from './utils/make-map-interactive.ts';


function waitForMapSize(): Promise<Size> {
	return new Promise((resolve) => {
		const effect = new Effect(() => {
			const size = AppState.mapSize.value;
			if (size.width > 0 && size.height > 0) {
				effect.dispose();
				resolve(size);
			}
		});
	});
}


export function WGLMap() {
	printDebugInfo('UI::WGLMap');

	let canvas;

	const WGLMap = (
		Section('wgl-map').class(style.wglMap).nodes([
			BigInset().nodes([
				Div().nodes([
					canvas = Canvas('canvas'),
				]),
			]),
		])
	);

	(async () => {
		const canvasElement = canvas.element;
		await loadMapProject(await resolveTextResource('resources/maps/SNOW_5.json'));

		const wglMap = new WglMap(canvasElement);
		AppState.wglMap.set(wglMap);

		await waitForMapSize();

		wglMap.enableAnimation();
		wglMap.render();

		makeMapInteractive(canvasElement, (cursorX: number, cursorY: number, panDeltaX: number, panDeltaY: number, zoomDelta: number) => {
			if (panDeltaX !== 0 || panDeltaY !== 0 || zoomDelta !== 0) {
				wglMap.moveCamera(panDeltaX, panDeltaY, zoomDelta, cursorX, cursorY);
			}
			if (cursorX !== 0 || cursorY !== 0) {
				wglMap.moveCursor(cursorX, cursorY);
			}
		});

		new Effect(function () {
			wglMap.onCanvasResize();
		}, { strong: true }).on([AppEvents.windowResizeSignal]);

		let initialized = false;
		new Effect(function () {
			if (!initialized) { initialized = true; return; }
			wglMap.disableAnimation();
			wglMap.cleanup();
		}, { strong: true }).on([AppEvents.windowCloseSignal]);
	})();

	return WGLMap;
}
