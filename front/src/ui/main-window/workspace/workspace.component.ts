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

	const leftWindow = DockableWindow({ id: 'minimap', title: 'Minimap', content: 'Docked minimap (placeholder)' });
	const rightTopWindow = DockableWindow({ id: 'tile-explorer', title: 'Tile Explorer', content: 'Tiles list (placeholder)' });
	const rightBottomWindow = DockableWindow({ id: 'color-palette', title: 'Color Palette', content: 'Palette (placeholder)' });
	const bottomWindow = DockableWindow({ id: 'toolbox', title: 'Toolbox', content: 'Tools (placeholder)' });
	const floatingLayer = Div('floating-layer').class(style.floatingLayer);

	const rightDock = Div('dock-right').class(style.dockRight).nodes([
		rightTopWindow,
		rightBottomWindow,
	]);

	const leftDock = Div('dock-left').class(style.dockLeft).nodes([
		leftWindow,
	]);
	const bottomDock = Div('dock-bottom').class(style.dockBottom).nodes([
		bottomWindow,
	]);

	const content = Div('workspace-content').class(style.content).nodes([
		leftDock,
		leftResizer,
		Div('workspace-center').class(style.center).nodes([
			WGLMap(),
		]),
		rightResizer,
		rightDock,
		bottomResizer,
		bottomDock,
		floatingLayer,
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
	const minPanelHeight = 50;
	const minPanelWidth = 150;
	const splitterSize = 6;
	let draggingWindow: HTMLElement | null = null;
	let dragSourceDock: { container: HTMLElement; axis: 'vertical' | 'horizontal' } | null = null;
	function setFloating(windowEl: HTMLElement, x: number, y: number) {
		windowEl.classList.add(style.floatingWindow);
		windowEl.style.left = `${x}px`;
		windowEl.style.top = `${y}px`;
		floatingLayer.element.appendChild(windowEl);
	}

	function setDocked(windowEl: HTMLElement) {
		windowEl.classList.remove(style.floatingWindow);
		windowEl.style.left = '';
		windowEl.style.top = '';
		windowEl.style.flex = '';
	}

	function getDockWindows(container: HTMLElement) {
		return Array.from(container.children).filter((child) =>
			(child as HTMLElement).dataset.windowId
		) as HTMLElement[];
	}

	function getPanelSize(el: HTMLElement, axis: 'vertical' | 'horizontal') {
		const rect = el.getBoundingClientRect();
		return axis === 'vertical' ? rect.height : rect.width;
	}

	function setFixedSize(el: HTMLElement, size: number) {
		el.dataset.size = `${size}`;
		el.style.flex = `0 0 ${size}px`;
	}

	function applyDockSizing(container: HTMLElement, axis: 'vertical' | 'horizontal') {
		const windows = getDockWindows(container);
		if (windows.length === 0) return;

		const minSize = axis === 'vertical' ? minPanelHeight : minPanelWidth;
		const last = windows[windows.length - 1];
		last.dataset.size = '';
		last.style.flex = '1 1 auto';

		for (let i = 0; i < windows.length - 1; i += 1) {
			const win = windows[i];
			const current = parseInt(win.dataset.size || '', 10) || getPanelSize(win, axis);
			setFixedSize(win, Math.max(minSize, current));
		}
	}

	function removeDockSplitters(container: HTMLElement) {
		Array.from(container.children).forEach((child) => {
			const el = child as HTMLElement;
			if (el.dataset.role === 'dock-splitter') {
				el.remove();
			}
		});
	}

	function insertByAxis(container: HTMLElement, windowEl: HTMLElement, clientX: number, clientY: number, axis: 'vertical' | 'horizontal') {
		const windows = getDockWindows(container).filter(el => el !== windowEl);
		if (windows.length === 0) {
			container.appendChild(windowEl);
			return;
		}
		const target = windows.find((el) => {
			const rect = el.getBoundingClientRect();
			return axis === 'vertical'
				? clientY < rect.top + rect.height / 2
				: clientX < rect.left + rect.width / 2;
		});
		if (target) {
			container.insertBefore(windowEl, target);
		} else {
			container.appendChild(windowEl);
		}
	}

	function registerDock(container: HTMLElement, axis: 'vertical' | 'horizontal') {
		container.addEventListener('dragover', (event) => {
			event.preventDefault();
		});
		container.addEventListener('drop', (event) => {
			event.preventDefault();
			if (!draggingWindow) return;
			setDocked(draggingWindow);
			insertByAxis(container, draggingWindow, event.clientX, event.clientY, axis);
			rebuildDock(container, axis);
			applyDockSizing(container, axis);
			if (dragSourceDock && dragSourceDock.container !== container) {
				rebuildDock(dragSourceDock.container, dragSourceDock.axis);
				applyDockSizing(dragSourceDock.container, dragSourceDock.axis);
			}
			dragSourceDock = null;
		});
	}

	function enableDrag(windowEl: HTMLElement) {
		const titlebar = windowEl.querySelector('[data-role="dock-titlebar"]') as HTMLElement | null;
		if (!titlebar) return;
		titlebar.draggable = true;
		titlebar.addEventListener('dragstart', (event) => {
			draggingWindow = windowEl;
			windowEl.classList.add(style.dragging);
			const parent = windowEl.parentElement as HTMLElement | null;
			if (parent === leftDock.element) {
				dragSourceDock = { container: leftDock.element, axis: 'vertical' };
			} else if (parent === rightDock.element) {
				dragSourceDock = { container: rightDock.element, axis: 'vertical' };
			} else if (parent === bottomDock.element) {
				dragSourceDock = { container: bottomDock.element, axis: 'horizontal' };
			} else {
				dragSourceDock = null;
			}
			event.dataTransfer?.setData('text/plain', windowEl.dataset.windowId ?? 'dock-window');
			event.dataTransfer?.setDragImage(windowEl, 20, 20);
		});
		titlebar.addEventListener('dragend', () => {
			windowEl.classList.remove(style.dragging);
			draggingWindow = null;
			if (dragSourceDock) {
				rebuildDock(dragSourceDock.container, dragSourceDock.axis);
				dragSourceDock = null;
			}
		});
	}

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

	function resizeSplit(container: HTMLElement, prevIndex: number, event: MouseEvent, axis: 'vertical' | 'horizontal') {
		const windows = getDockWindows(container);
		const prev = windows[prevIndex];
		if (!prev) return;

		const minSize = axis === 'vertical' ? minPanelHeight : minPanelWidth;
		const containerSize = axis === 'vertical' ? container.clientHeight : container.clientWidth;
		const available = containerSize - splitterSize * Math.max(0, windows.length - 1);
		const lastMin = minSize;

		let fixedSum = 0;
		for (let i = 0; i < windows.length - 1; i += 1) {
			if (i === prevIndex) continue;
			const win = windows[i];
			const size = parseInt(win.dataset.size || '', 10) || getPanelSize(win, axis);
			fixedSum += Math.max(minSize, size);
		}

		const maxPrev = Math.max(minSize, available - fixedSum - lastMin);
		const prevRect = prev.getBoundingClientRect();
		const desired = axis === 'vertical'
			? event.clientY - prevRect.top
			: event.clientX - prevRect.left;
		const nextPrev = Math.min(maxPrev, Math.max(minSize, desired));
		setFixedSize(prev, nextPrev);
		applyDockSizing(container, axis);
	}

	function rebuildDock(container: HTMLElement, axis: 'vertical' | 'horizontal') {
		removeDockSplitters(container);
		const windows = getDockWindows(container);
		if (windows.length <= 1) {
			applyDockSizing(container, axis);
			return;
		}

		applyDockSizing(container, axis);

		for (let i = 0; i < windows.length - 1; i += 1) {
			const splitter = Div('dock-splitter').class(axis === 'vertical' ? style.dockSplitterHorizontal : style.dockSplitterVertical);
			splitter.element.dataset.role = 'dock-splitter';
			splitter.element.addEventListener('mousedown', startDrag((event) => {
				resizeSplit(container, i, event, axis);
			}));
			windows[i + 1].parentElement?.insertBefore(splitter.element, windows[i + 1]);
		}
	}

	registerDock(leftDock.element, 'vertical');
	registerDock(rightDock.element, 'vertical');
	registerDock(bottomDock.element, 'horizontal');

	rebuildDock(leftDock.element, 'vertical');
	rebuildDock(rightDock.element, 'vertical');
	rebuildDock(bottomDock.element, 'horizontal');

	floatingLayer.element.addEventListener('dragover', (event) => {
		event.preventDefault();
	});

	floatingLayer.element.addEventListener('drop', (event) => {
		event.preventDefault();
		if (!draggingWindow) return;
		const rect = contentEl.getBoundingClientRect();
		const x = event.clientX - rect.left - 20;
		const y = event.clientY - rect.top - 20;
		setFloating(draggingWindow, x, y);
		if (dragSourceDock) {
			rebuildDock(dragSourceDock.container, dragSourceDock.axis);
			applyDockSizing(dragSourceDock.container, dragSourceDock.axis);
		}
		dragSourceDock = null;
	});

	[leftWindow, rightTopWindow, rightBottomWindow, bottomWindow].forEach((windowNode) => {
		enableDrag(windowNode.element);
	});

	return Section('workspace').class(style.workspace).nodes([
		tabs,
		content,
	]);
}
