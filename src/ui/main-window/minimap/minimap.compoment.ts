import { BoxScreen } from '^src/ui/components/screens/box-screen.component.ts';
import { Section } from '^utils/reactive/html-node.elements.ts';

import style from './minimap.module.css';


export function Minimap() {
	return (
		Section('minimap').class(style.minimap).nodes([
			BoxScreen().class(style.minimapContent).text('MINIMAP'),
		])
	)
}
