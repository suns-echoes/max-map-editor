import { printDebugInfo } from '^utils/debug/debug.ts';
import { Section } from '^utils/reactive/html-node.elements.ts';

import style from './status-bar.module.css';
import { LabelScreen } from '^src/ui/components/screens/label-screen.component';


export function StatusBar() {
	printDebugInfo('UI::StatusBar');

	return (
		Section('status-bar').class(style.statusBar).nodes([
			LabelScreen('visible-area-hint').style({ width: '21ch' }).text('048-094 : 121-124'),
			LabelScreen('cursor-position-hint').style({ width: '11ch' }).text('056-101'),
			LabelScreen('zoom-hint').style({ width: '14ch' }).text('zoom 1 x'),
		])
	);
}
