import { Div } from '^utils/reactive/html-node.elements.ts';

import styte from './horizontal-separator.module.css';


export function HorizontalSeparator() {
	return Div('horizontal-separator').class(styte.horizontalSeparator);
}
