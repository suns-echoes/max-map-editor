import { Button } from '^reactive/reactive-node.elements.ts';
import style from './menu-button.module.css';


export function MenuButton(text?: string) {
	return Button(text).baseClass(style.menuButton);
}
