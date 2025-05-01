import { Button } from '^utils/reactive/html-node.elements.ts';
import style from './simple-button.module.css';


export function SimpleButton(debugName?: string) {
	return Button(debugName).class(style.simpleButton);
}
