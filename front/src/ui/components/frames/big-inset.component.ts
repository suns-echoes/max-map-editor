import { Div } from '^reactive/reactive-node.elements.ts';

import style from './big-inset.module.css';


export function BigInset(text?: string) {
	return Div(text).baseClass(style.bigInset);
}
