import { xlog } from '^lib/xlog/xlog.ts';
import { Section } from '^reactive/reactive-node.elements.ts';

import style from './main-layout.module.css';


export function MainLayout() {
	xlog.info('UI::MainLayout');

	return (
		Section('main-layout').class(style.mainLayout)
	);
}
