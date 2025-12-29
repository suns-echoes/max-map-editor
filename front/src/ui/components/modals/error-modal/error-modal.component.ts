import { Div, Img } from '^lib/reactive/html-node.elements.ts';

import { BigOutset } from '../../frames/big-outset.component.ts';
import { LabelScreen } from '../../screens/label-screen.component.ts';
import { BoxScreen } from '../../screens/box-screen.component.ts';
import { SimpleButton } from '../../buttons/simple-button.component.ts';

import style from './error-modal.module.css';


export interface ErrorModalProps {
	title?: string;
	message?: string;
	onClose?: () => void;
}

export function ErrorModal(props: ErrorModalProps) {
	let closeButton;

	const errorPopup = (
		Div().class(style.backdrop).nodes([
			Div('error-popup').class(style.modal).nodes([
				BigOutset().class(style.content).nodes([
					LabelScreen().class(style.title).text(props.title ?? 'Error'),
					BoxScreen().nodes([
						Img().src('/images/bio-icon.png').class(style.icon),
						Div().class(style.message).html(props.message ?? 'An unexpected error occurred.'),
					]),
					closeButton = SimpleButton().class(style.closeButton).text('Close')
				]),
			]),
		])
	);

	closeButton.addEventListener('click', function () {
		errorPopup.destroy();
		props.onClose?.();
	});

	document.body.appendChild(errorPopup.element);

	return errorPopup;
}
