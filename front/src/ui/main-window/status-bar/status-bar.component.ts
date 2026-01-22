import { xlog } from '^lib/xlog/xlog.ts';
import { Div, Section } from '^reactive/reactive-node.elements.ts';

import style from './status-bar.module.css';
import { LabelScreen } from '../../components/screens/label-screen.component.ts';


export function StatusBar() {
	xlog.info('UI::StatusBar');

	const modeLabel = LabelScreen('mode').style({ width: '16ch' }).text('Mode: Paint');
	const cursorLabel = LabelScreen('cursor').style({ width: '18ch' }).text('Tile: 000-000');
	const selectionLabel = LabelScreen('selection').style({ width: '20ch' }).text('Selected: 0');

	const helpLabel = LabelScreen('help').style({ width: '60ch' }).text('');
	const defaultContent = Div('status-default').class(style.defaultContent).nodes([
		modeLabel,
		cursorLabel,
		selectionLabel,
	]);
	const helpContent = Div('status-help').class(style.helpContent).nodes([
		helpLabel,
	]);

	function setHelp(text: string | null) {
		if (text && text.trim()) {
			helpLabel.text(text.trim());
			defaultContent.element.classList.add(style.hidden);
			helpContent.element.classList.remove(style.hidden);
			return;
		}
		defaultContent.element.classList.remove(style.hidden);
		helpContent.element.classList.add(style.hidden);
	}

	let lastHelpText: string | null = null;
	document.addEventListener('mousemove', (event) => {
		const target = event.target as HTMLElement | null;
		if (!target) return;
		const helpElement = target.closest('[data-help]') as HTMLElement | null;
		const nextHelp = helpElement?.dataset.help?.trim() || null;
		if (nextHelp === lastHelpText) return;
		lastHelpText = nextHelp;
		setHelp(nextHelp);
	});

	return (
		Section('status-bar').class(style.statusBar).nodes([
			defaultContent,
			helpContent.class(style.hidden),
		])
	);
}
