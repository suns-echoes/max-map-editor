import { Button } from '^lib/reactive/html-node.elements.ts';
import style from './menu-button.module.css';


export function MenuButton(debugName?: string) {
	return Button(debugName).class(style.menuButton);
}
