import { loadMapProject } from '^actions/load-map-project/load-map-project.ts';
import { AppEvents } from '^events/app-events.ts';
import { AppState } from '^state/app-state.ts';
import { resolveTextResource } from '^tauri-apps/api/path.ts';
import { printDebugInfo } from '^lib/debug/debug.ts';
import { Effect } from '^lib/reactive/effect.class.ts';
import { Canvas, Div, Section } from '^lib/reactive/html-node.elements.ts';
import { Value } from '^lib/reactive/value.class.ts';

import style from './wgl-map.module.css';
import { BigInset } from '../../components/frames/big-inset.component.ts';
import { WglMap } from './wgl/wgl-map.ts';
import { makeMapInteractive } from './utils/make-map-interactive.ts';


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

		await Value.toPromise(AppState.mapSize, function (size) { return size !== null; });

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
		}).watch([AppEvents.windowResizeSignal]);

		new Effect(function () {
			wglMap.cleanup();
		}).watch([AppEvents.windowCloseSignal]);
	})();

	return WGLMap;
}
