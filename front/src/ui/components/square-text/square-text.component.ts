import { Div, Span } from '^reactive/reactive-node.elements.ts';

import style from './square-text.module.css';


export function SquareText(text: string) {
	return Div().class(style.squareText).nodes([
		Span().text(text),
		Span().text(text),
	]);
}
