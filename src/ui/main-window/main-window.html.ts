import { Section } from '^utils/reactive/html-node.elements.ts';
import { WGLMap } from './wgl-map/wgl-map.html.ts';
import { printDebugInfo } from '^utils/debug/debug.ts';
import { saveMainWindowParams } from '^actions/main-window/save-main-window-params.ts';
import { AppEvents } from '^events/app-events.ts';
import { AsyncEffect } from '^utils/reactive/async-effect.class.ts';
import { BuildInfo } from './build-info/build.info.html.ts';
import { MainLayout } from './main-layout/main-layout.html.ts';
import { MainMenu } from './main-menu/main-menu.html.ts';
import { StatusBar } from './status-bar/status-bar.html.ts';


export function MainWindow() {
	printDebugInfo('UI::MainWindow');

	new AsyncEffect(saveMainWindowParams).watch([AppEvents.windowResizeSignal]);

	return (
		Section('main-window').nodes([
			BuildInfo(),
			MainLayout().nodes([
				MainMenu(),
				Section().text('Minimap'),
				Section().text('MainToolbar'),
				Section().text('SideToolbar'),
				WGLMap(),
				Section().text('BottomToolbar'),
				StatusBar(),
			]),
		])
	);
}
