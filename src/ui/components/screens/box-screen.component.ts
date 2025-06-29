import { BigInset } from '../frames/big-inset.component.ts';
import { Div } from '^lib/reactive/html-node.elements.ts';
import { HTMLNode } from '^lib/reactive/html-node.class.ts';

import style from './box-screen.module.css';


export function BoxScreen(debugName?: string) {
	let content;

	const boxScreen = (
		BigInset(debugName).class(style.boxScreen).nodes([
			Div().class(style.boxGlass).nodes([
				content = Div().class(style.boxContent),
			]),
		])
	);

	boxScreen.class = function (className: string) {
		HTMLNode.prototype.classes.call(content, style.boxContent, className);
		return boxScreen;
	};

	boxScreen.text = function (text: string) {
		HTMLNode.prototype.text.call(content, text);
		return boxScreen;
	};

	return boxScreen;
}
