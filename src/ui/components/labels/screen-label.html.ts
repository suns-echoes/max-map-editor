import { Div } from '^utils/reactive/html-node.elements.ts';
import style from './screen-label.module.css';


export function ScreenLabel(debugName?: string) {
	const screenLabel = (
		Div(debugName).class(style.screenLabel).nodes([
			Div(),
		])
	);

	screenLabel.text = function (newText: string) {
		screenLabel.element.firstElementChild!.textContent = newText;
		return screenLabel;
	}

	return screenLabel;
}
