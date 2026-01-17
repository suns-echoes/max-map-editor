import { Button } from '^reactive/reactive-node.elements.ts';

import { SquareText } from '../square-text/square-text.component.ts';

import style from './submenu-button.module.css';


export function SubmenuButton(text: string, debugName?: string) {
	return Button(debugName).class(style.submenuButton).nodes([
		SquareText(text),
	]);
}
