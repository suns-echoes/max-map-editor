import { Button, Div, Section, Span } from '^reactive/reactive-node.elements.ts';

import style from './dockable-window.module.css';


interface DockableWindowProps {
	id: string;
	title: string;
	content?: string;
}

export function DockableWindow({ id, title, content }: DockableWindowProps) {
	const titleBar = Div('dockable-window-titlebar').class(style.titleBar).nodes([
			Span(title).class(style.title),
			Button('×').class(style.closeButton),
		]);

	const windowNode = Section('dockable-window').class(style.window).nodes([
		titleBar,
		Div('dockable-window-body').class(style.body).text(content ?? ''),
	]);

	windowNode.element.dataset.windowId = id;
	titleBar.element.dataset.role = 'dock-titlebar';

	return windowNode;
}
