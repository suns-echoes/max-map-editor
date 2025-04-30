import { printDebugInfo } from '^utils/debug/debug.ts';
import { Section } from '^utils/reactive/html-node.elements.ts';

import style from './main-layout.module.css';


export function MainLayout() {
	printDebugInfo('UI::MainLayout');

	return (
		Section('main-layout').class(style.mainLayout)
	);
}
