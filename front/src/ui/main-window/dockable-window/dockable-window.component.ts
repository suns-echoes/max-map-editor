import { Button, Div, Section, Span } from '^reactive/reactive-node.elements.ts';

import style from './dockable-window.module.css';


interface DockableWindowProps {
	title: string;
	content?: string;
}

export function DockableWindow({ title, content }: DockableWindowProps) {
	return Section('dockable-window').class(style.window).nodes([
		Div('dockable-window-titlebar').class(style.titleBar).nodes([
			Span(title).class(style.title),
			Button('×').class(style.closeButton),
		]),
		Div('dockable-window-body').class(style.body).text(content ?? ''),
	]);
}
