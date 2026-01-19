import { Div } from '^reactive/reactive-node.elements.ts';

import style from './big-outset.module.css';


export function BigOutset(text?: string) {
	return Div(text).baseClass(style.bigOutset);
}
