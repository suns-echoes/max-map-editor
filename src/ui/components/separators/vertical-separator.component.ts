import { Div } from '^utils/reactive/html-node.elements.ts';

import styte from './vertical-separator.module.css';


export function VerticalSeparator() {
	return Div('vertical-separator').class(styte.verticalSeparator);
}
