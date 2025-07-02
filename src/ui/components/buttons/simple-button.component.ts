import { Button } from '^lib/reactive/html-node.elements.ts';
import style from './simple-button.module.css';


export function SimpleButton(debugName?: string) {
	const button = Button(debugName).class(style.simpleButton);

	button.class = function (className: string) {
		button.classes(style.simpleButton, className);
		return button;
	}

	return button;
}
