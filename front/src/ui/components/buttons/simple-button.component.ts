import { Button } from '^reactive/reactive-node.elements.ts';
import style from './simple-button.module.css';


export function SimpleButton(debugName?: string) {
	return Button(debugName).baseClass(style.simpleButton);
}
