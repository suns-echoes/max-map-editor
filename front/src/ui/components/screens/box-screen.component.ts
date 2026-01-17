import { BigInset } from '../frames/big-inset.component.ts';
import { Div } from '^reactive/reactive-node.elements.ts';

import style from './box-screen.module.css';


export function BoxScreen() {
	const content = Div().baseClass(style.boxContent);

	const boxScreen = (
		BigInset().class(style.boxInset).nodes([
			Div().class(style.boxScreen).nodes([
				content,
			]),
		])
	);

	// Delegate text/html/nodes operations to content node
	boxScreen.setInterface(content);

	return boxScreen;
}
