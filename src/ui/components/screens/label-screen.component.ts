import { Div } from '^utils/reactive/html-node.elements.ts';

import style from './label-screen.module.css';


export function LabelScreen(debugName?: string) {
	const labelScreen = (
		Div(debugName).class(style.labelScreen).nodes([
			Div(),
		])
	);

	labelScreen.text = function (newText: string) {
		labelScreen.element.firstElementChild!.textContent = newText;
		return labelScreen;
	}

	return labelScreen;
}
