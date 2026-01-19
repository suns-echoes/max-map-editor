import { Div } from '^reactive/reactive-node.elements.ts';

import style from './label-screen.module.css';


export function LabelScreen(text?: string) {
	const labelScreen = (
		Div(text).baseClass(style.labelScreen).nodes([
			Div(),
		])
	);

	(labelScreen as any).text = function (newText: string) {
		labelScreen.element.firstElementChild!.textContent = newText;
		return labelScreen;
	}

	return labelScreen;
}
