import { Div } from '^reactive/reactive-node.elements.ts';

import style from './big-inset.module.css';


export function BigInset(debugName?: string) {
	return Div(debugName).baseClass(style.bigInset);
}
