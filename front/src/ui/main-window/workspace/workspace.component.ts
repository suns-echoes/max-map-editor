import { Value } from '^reactive/value.ts';
import { Button, Div, Section, Span } from '^reactive/reactive-node.elements.ts';
import { xlog } from '^lib/xlog/xlog.ts';
import { WGLMap } from '../wgl-map/wgl-map.component.ts';
import { DockableWindow } from '../dockable-window/dockable-window.component.ts';

import style from './workspace.module.css';


const DEFAULT_TABS = ['GREEN_1.json', 'DESERT_1.json', 'CRATER_1.json'];


export function Workspace() {
	xlog.info('UI::Workspace');

	const activeIndex = new Value(0);
	const tabButtons: Array<{ button: ReturnType<typeof Button>; container: ReturnType<typeof Div> }> = [];

	const tabs = Div('workspace-tabs').class(style.tabs).nodes(
		DEFAULT_TABS.map((label, index) => {
			const title = Span(label).class(style.tabTitle);
			const closeButton = Button('×').class(style.closeButton);
			const button = Button().class(style.tabButton).nodes([title, closeButton]);
			const container = Div().class(style.tabItem).nodes([button]);

			if (index === activeIndex.value) {
				button.element.classList.add(style.active);
			}

			button.on('click', () => {
				if (activeIndex.value === index) return;
				activeIndex.value = index;
				tabButtons.forEach((item, idx) => {
					item.button.element.classList.toggle(style.active, idx === index);
				});
			});

			closeButton.on('click', (event) => {
				event.stopPropagation();
				if (tabButtons.length <= 1) return;
				const removeIndex = tabButtons.findIndex(item => item.container === container);
				if (removeIndex === -1) return;

				tabButtons.splice(removeIndex, 1);
				container.dispose();

				if (activeIndex.value === removeIndex) {
					activeIndex.value = Math.max(0, removeIndex - 1);
				} else if (activeIndex.value > removeIndex) {
					activeIndex.value -= 1;
				}
				tabButtons.forEach((item, idx) => {
					item.button.element.classList.toggle(style.active, idx === activeIndex.value);
				});
			});

			tabButtons.push({ button, container });
			return container;
		})
	);

	const leftResizer = Div('dock-left-resizer').class(style.resizerLeft);
	const rightResizer = Div('dock-right-resizer').class(style.resizerRight);
	const bottomResizer = Div('dock-bottom-resizer').class(style.resizerHorizontal);

	const rightTopWindow = DockableWindow({ title: 'Tile Explorer', content: 'Tiles list (placeholder)' });
	const rightBottomWindow = DockableWindow({ title: 'Color Palette', content: 'Palette (placeholder)' });
	const rightSplitter = Div('dock-right-splitter').class(style.dockSplitterHorizontal);

	const rightDock = Div('dock-right').class(style.dockRight).nodes([
		rightTopWindow,
		rightSplitter,
		rightBottomWindow,
	]);

	const content = Div('workspace-content').class(style.content).nodes([
		Div('dock-left').class(style.dockLeft).nodes([
			DockableWindow({ title: 'Minimap', content: 'Docked minimap (placeholder)' }),
		]),
		leftResizer,
		Div('workspace-center').class(style.center).nodes([
			WGLMap(),
		]),
		rightResizer,
		rightDock,
		bottomResizer,
		Div('dock-bottom').class(style.dockBottom).nodes([
			DockableWindow({ title: 'Toolbox', content: 'Tools (placeholder)' }),
		]),
	]);

	const contentEl = content.element as HTMLElement;
	contentEl.style.setProperty('--dock-left-width', '240px');
	contentEl.style.setProperty('--dock-right-width', '240px');
	contentEl.style.setProperty('--dock-bottom-height', '180px');

	function startDrag(onMove: (event: MouseEvent) => void) {
		function handleMove(event: MouseEvent) {
			onMove(event);
		}

		function handleUp() {
			window.removeEventListener('mousemove', handleMove);
			window.removeEventListener('mouseup', handleUp);
		}

		return function handleDown(event: MouseEvent) {
			event.preventDefault();
			window.addEventListener('mousemove', handleMove);
			window.addEventListener('mouseup', handleUp);
		};
	}

	const minDockWidth = 160;
	const maxDockWidth = 480;
	const minDockHeight = 120;
	const maxDockHeight = 360;
	const minPanelSize = 80;

	leftResizer.element.addEventListener('mousedown', startDrag((event) => {
		const rect = contentEl.getBoundingClientRect();
		const next = Math.min(maxDockWidth, Math.max(minDockWidth, event.clientX - rect.left));
		contentEl.style.setProperty('--dock-left-width', `${next}px`);
	}));

	rightResizer.element.addEventListener('mousedown', startDrag((event) => {
		const rect = contentEl.getBoundingClientRect();
		const next = Math.min(maxDockWidth, Math.max(minDockWidth, rect.right - event.clientX));
		contentEl.style.setProperty('--dock-right-width', `${next}px`);
	}));

	bottomResizer.element.addEventListener('mousedown', startDrag((event) => {
		const rect = contentEl.getBoundingClientRect();
		const next = Math.min(maxDockHeight, Math.max(minDockHeight, rect.bottom - event.clientY));
		contentEl.style.setProperty('--dock-bottom-height', `${next}px`);
	}));

	function resizeVerticalSplit(container: HTMLElement, topEl: HTMLElement, bottomEl: HTMLElement, event: MouseEvent) {
		const containerRect = container.getBoundingClientRect();
		const splitterHeight = rightSplitter.element.getBoundingClientRect().height;
		const padding = 16;
		const available = containerRect.height - splitterHeight - padding;

		let nextTop = event.clientY - containerRect.top;
		nextTop = Math.min(available - minPanelSize, Math.max(minPanelSize, nextTop));
		const nextBottom = Math.max(minPanelSize, available - nextTop);

		topEl.style.flex = `0 0 ${nextTop}px`;
		bottomEl.style.flex = `0 0 ${nextBottom}px`;
	}

	rightTopWindow.element.style.flex = '1 1 0';
	rightBottomWindow.element.style.flex = '1 1 0';
	rightSplitter.element.addEventListener('mousedown', startDrag((event) => {
		resizeVerticalSplit(rightDock.element, rightTopWindow.element, rightBottomWindow.element, event);
	}));

	return Section('workspace').class(style.workspace).nodes([
		tabs,
		content,
	]);
}
