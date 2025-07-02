import { Div } from '^lib/reactive/html-node.elements.ts';

import style from './big-outset.module.css';


export function BigOutset(debugName?: string) {
	const bigOutset = Div(debugName).class(style.bigOutset);

	bigOutset.class = function (className: string) {
		bigOutset.classes(style.bigOutset, className);
		return bigOutset;
	};

	return bigOutset;
}
