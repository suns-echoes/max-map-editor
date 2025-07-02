import { Div, Section } from '^lib/reactive/html-node.elements.ts';

import { BoxScreen } from '^src/ui/components/screens/box-screen.component.ts';

import style from './minimap.module.css';


export function Minimap() {
	return (
		Section('minimap').class(style.minimap).nodes([
			BoxScreen().nodes([
				Div().class(style.minimapContent).text('MINIMAP'),
			]),
		])
	)
}
