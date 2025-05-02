import { Section } from '^utils/reactive/html-node.elements.ts';
import { WGLMap } from './wgl-map/wgl-map.component.ts';
import { printDebugInfo } from '^utils/debug/debug.ts';
import { saveMainWindowParams } from '^actions/main-window/save-main-window-params.ts';
import { AppEvents } from '^events/app-events.ts';
import { AsyncEffect } from '^utils/reactive/async-effect.class.ts';
import { BuildInfo } from './build-info/build.info.component.ts';
import { MainLayout } from './main-layout/main-layout.component.ts';
import { MainMenu } from './main-menu/main-menu.component.ts';
import { Minimap } from './minimap/minimap.compoment.ts';
import { StatusBar } from './status-bar/status-bar.component.ts';
import { MainToolbar } from './main-toolbar/main-toolbar.component.ts';
import { MapSelector } from './map-selector/map-selector.component.ts';


export function MainWindow() {
	printDebugInfo('UI::MainWindow');

	new AsyncEffect(saveMainWindowParams).watch([AppEvents.windowResizeSignal]);

	return (
		Section('main-window').nodes([
			BuildInfo(),
			MainLayout().nodes([
				MainMenu(),
				Minimap(),
				MainToolbar(),
				Section().text('Sidebar'),
				WGLMap(),
				Section().nodes([
					MapSelector(),
				]),
				StatusBar(),
			]),
		])
	);
}
