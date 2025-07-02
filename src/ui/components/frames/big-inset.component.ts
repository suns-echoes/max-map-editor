import { Div } from '^lib/reactive/html-node.elements.ts';

import style from './big-inset.module.css';


export function BigInset(debugName?: string) {
	const bigInset = Div(debugName).class(style.bigInset);

	bigInset.class = function (className: string) {
		bigInset.classes(style.bigInset, className);
		return bigInset;
	};

	return bigInset;
}
