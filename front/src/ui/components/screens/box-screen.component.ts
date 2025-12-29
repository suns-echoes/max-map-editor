import { BigInset } from '../frames/big-inset.component.ts';
import { Div } from '^lib/reactive/html-node.elements.ts';
import { HTMLNode } from '^lib/reactive/html-node.class.ts';

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

	boxScreen.class = function (className: string) {
		boxScreen.classes(boxScreenClass, className);
		return boxScreen;
	};

	boxScreen.text = function (text: string) {
		content.text(text);
		return boxScreen;
	};

	boxScreen.html = function (html: string) {
		content.html(html);
		return boxScreen;
	};

	boxScreen.nodes = function (nodes: HTMLNode[]) {
		content.nodes(nodes);
		return boxScreen;
	};

	return boxScreen;
}
