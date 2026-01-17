import { BigInset } from '../frames/big-inset.component.ts';
import { Div } from '^reactive/reactive-node.elements.ts';

import style from './box-screen.module.css';


export function BoxScreen() {
	let content = Div();

	const boxScreen = (
		BigInset().class(style.boxInset).nodes([
			Div().class(style.boxScreen).nodes([
				content = Div().class(style.boxContent),
			]),
		])
	);

	const boxScreenClass = boxScreen.element.className;

	(boxScreen as any).class = function (className: string) {
		boxScreen.classes(boxScreenClass, className);
		return boxScreen;
	};

	(boxScreen as any).text = function (text: string) {
		content.text(text);
		return boxScreen;
	};

	(boxScreen as any).html = function (html: string) {
		content.html(html);
		return boxScreen;
	};

	(boxScreen as any).nodes = function (nodes: any[]) {
		content.nodes(nodes);
		return boxScreen;
	};

	return boxScreen;
}
