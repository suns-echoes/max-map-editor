import { Button } from '^reactive/reactive-node.elements.ts';

import { SquareText } from '../square-text/square-text.component.ts';

import style from './submenu-button.module.css';


export function SubmenuButton(text: string) {
	return Button().baseClass(style.submenuButton).nodes([
		SquareText(text),
	]);
}
