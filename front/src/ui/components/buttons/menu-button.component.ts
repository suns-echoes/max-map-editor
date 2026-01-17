import { Button } from '^reactive/reactive-node.elements.ts';
import style from './menu-button.module.css';


export function MenuButton(debugName?: string) {
	return Button(debugName).class(style.menuButton);
}
