import { Div } from '^lib/reactive/html-node.elements.ts';

import style from './big-inset.module.css';
import { HTMLNode } from '^lib/reactive/html-node.class.ts';


export function BigInset(debugName?: string) {
	const bigInset = Div(debugName).class(style.bigInset);

	bigInset.class = function (className: string) {
		HTMLNode.prototype.classes.call(bigInset, style.bigInset, className);
		return bigInset;
	};

	return bigInset;
}
