import { Div } from '^reactive/reactive-node.elements.ts';

import style from './big-outset.module.css';


export function BigOutset(debugName?: string) {
	return Div(debugName).baseClass(style.bigOutset);
}
