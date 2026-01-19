import { Button } from '^reactive/reactive-node.elements.ts';
import style from './simple-button.module.css';


export function SimpleButton(text?: string) {
	return Button(text).baseClass(style.simpleButton);
}
