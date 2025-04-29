import { Section } from '^utils/reactive/html-node.elements.ts';
import { WGLMap } from './wgl-map/wgl-map.html.ts';
import { printDebugInfo } from '^utils/debug/debug.ts';
import { saveMainWindowParams } from '^actions/main-window/save-main-window-params.ts';
import { AppEvents } from '^events/app-events.ts';
import { AsyncEffect } from '^utils/reactive/async-effect.class.ts';
import { BuildInfo } from './build-info/build.info.html.ts';


export function MainWindow() {
	printDebugInfo('UI::MainWindow');

	new AsyncEffect(saveMainWindowParams).watch([AppEvents.windowResizeSignal]);

	return (
		Section('main-window').nodes([
			BuildInfo(),
			WGLMap(),
		])
	);
}
