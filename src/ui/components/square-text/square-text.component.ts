import { Div, Span } from '^lib/reactive/html-node.elements.ts';

import style from './square-text.module.css';


export function SquareText(text: string) {
	return Div().class(style.squareText).nodes([
		Span().text(text),
		Span().text(text),
	]);
}
