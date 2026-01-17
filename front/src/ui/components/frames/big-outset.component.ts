import { Div } from '^reactive/reactive-node.elements.ts';

import style from './big-outset.module.css';


export function BigOutset(debugName?: string) {
	const bigOutset = Div(debugName).class(style.bigOutset);

	(bigOutset as any).class = function (className: string) {
		bigOutset.classes(style.bigOutset, className);
		return bigOutset;
	};

	return bigOutset;
}
