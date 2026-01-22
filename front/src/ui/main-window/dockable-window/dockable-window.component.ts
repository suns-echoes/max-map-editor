import { Button, Div, Section, Span } from '^reactive/reactive-node.elements.ts';

import style from './dockable-window.module.css';


interface DockableWindowProps {
	id: string;
	title: string;
	content?: string | ReturnType<typeof Div> | Array<ReturnType<typeof Div>>;
	minWidth?: number;
	maxWidth?: number;
	minHeight?: number;
	maxHeight?: number;
	defaultWidth?: number;
	defaultHeight?: number;
	resizable?: boolean;
}

export function DockableWindow({
	id,
	title,
	content,
	minWidth,
	maxWidth,
	minHeight,
	maxHeight,
	defaultWidth,
	defaultHeight,
	resizable = true,
}: DockableWindowProps) {
	const titleBar = Div('dockable-window-titlebar').class(style.titleBar).nodes([
			Span(title).class(style.title),
			Button('×').class(style.closeButton),
		]);

	const body = Div('dockable-window-body').class(style.body);
	if (typeof content === 'string' || content === undefined) {
		body.text(content ?? '');
	} else if (Array.isArray(content)) {
		body.nodes(content);
	} else {
		body.nodes([content]);
	}

	const windowNode = Section('dockable-window').class(style.window).nodes([
		titleBar,
		body,
		...(resizable ? [Div('dockable-window-resize-handle').class(style.resizeHandle)] : []),
	]);

	windowNode.element.dataset.windowId = id;
	titleBar.element.dataset.role = 'dock-titlebar';
	windowNode.element.dataset.minWidth = minWidth?.toString() ?? '';
	windowNode.element.dataset.maxWidth = maxWidth?.toString() ?? '';
	windowNode.element.dataset.minHeight = minHeight?.toString() ?? '';
	windowNode.element.dataset.maxHeight = maxHeight?.toString() ?? '';
	windowNode.element.dataset.defaultWidth = defaultWidth?.toString() ?? '';
	windowNode.element.dataset.defaultHeight = defaultHeight?.toString() ?? '';
	windowNode.element.dataset.resizable = resizable ? 'true' : 'false';

	return windowNode;
}
