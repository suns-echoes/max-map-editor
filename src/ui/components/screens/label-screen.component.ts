import { Div } from '^lib/reactive/html-node.elements.ts';
import { HTMLNode } from '^lib/reactive/html-node.class.ts';

import style from './label-screen.module.css';


export function LabelScreen(debugName?: string) {
	const labelScreen = (
		Div(debugName).class(style.labelScreen).nodes([
			Div(),
		])
	);

	labelScreen.class = function (className: string) {
		HTMLNode.prototype.classes.call(labelScreen, style.labelScreen, className);
		return labelScreen;
	};

	labelScreen.text = function (newText: string) {
		labelScreen.element.firstElementChild!.textContent = newText;
		return labelScreen;
	}

	return labelScreen;
}
