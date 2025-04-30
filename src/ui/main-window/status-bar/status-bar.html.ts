import { printDebugInfo } from '^utils/debug/debug.ts';
import { Section } from '^utils/reactive/html-node.elements.ts';

import style from './status-bar.module.css';
import { ScreenLabel } from '^src/ui/components/labels/screen-label.html.ts';


export function StatusBar() {
	printDebugInfo('UI::StatusBar');

	return (
		Section('status-bar').class(style.statusBar).nodes([
			ScreenLabel('visible-area-hint').style({ width: '21ch' }).text('048-094 : 121-124'),
			ScreenLabel('cursor-position-hint').style({ width: '11ch' }).text('056-101'),
			ScreenLabel('zoom-hint').style({ width: '14ch' }).text('zoom 1 x'),
		])
	);
}
