import { Value } from '^reactive/value.ts';
import { Button, Div, Section, Span, TextInput } from '^reactive/reactive-node.elements.ts';
import { xlog } from '^lib/xlog/xlog.ts';
import { WGLMap } from '../wgl-map/wgl-map.component.ts';
import { DockableWindow } from '../dockable-window/dockable-window.component.ts';

import style from './workspace.module.css';
import windowStyle from '../dockable-window/dockable-window.module.css';


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
	const topResizer = Div('dock-top-resizer').class(style.resizerTop);
	const bottomResizer = Div('dock-bottom-resizer').class(style.resizerHorizontal);

	const leftWindow = DockableWindow({
		id: 'minimap',
		title: 'Minimap',
		content: 'Controls',
		minWidth: 180,
		minHeight: 140,
		defaultWidth: 260,
		defaultHeight: 200,
		resizable: true,
	});
	const rightTopWindow = DockableWindow({
		id: 'tile-explorer',
		title: 'Tile Explorer',
		content: 'Controls',
		minWidth: 200,
		minHeight: 160,
		maxWidth: 520,
		maxHeight: 420,
		defaultWidth: 320,
		defaultHeight: 240,
		resizable: true,
	});
	const rightBottomWindow = DockableWindow({
		id: 'color-palette',
		title: 'Color Palette',
		content: 'Controls',
		minWidth: 180,
		minHeight: 120,
		defaultWidth: 260,
		defaultHeight: 200,
		resizable: true,
	});
	const bottomWindow = DockableWindow({
		id: 'toolbox',
		title: 'Toolbox',
		content: 'Controls',
		minWidth: 260,
		minHeight: 120,
		defaultWidth: 360,
		defaultHeight: 200,
		resizable: false,
	});
	const topWindow = DockableWindow({
		id: 'quick-tools',
		title: 'Quick Tools',
		content: 'Controls',
		minWidth: 260,
		minHeight: 100,
		defaultWidth: 360,
		defaultHeight: 160,
		resizable: true,
	});
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
	const topDock = Div('dock-top').class(style.dockTop).nodes([
		topWindow,
	]);

	const content = Div('workspace-content').class(style.content).nodes([
		topDock,
		topResizer,
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

	let leftDockWidth = 240;
	let rightDockWidth = 240;
	let topDockHeight = 160;
	let bottomDockHeight = 180;

	const contentEl = content.element as HTMLElement;
	contentEl.style.setProperty('--dock-left-width', `${leftDockWidth}px`);
	contentEl.style.setProperty('--dock-right-width', `${rightDockWidth}px`);
	contentEl.style.setProperty('--dock-left-resizer', '6px');
	contentEl.style.setProperty('--dock-right-resizer', '6px');
	contentEl.style.setProperty('--dock-top-height', `${topDockHeight}px`);
	contentEl.style.setProperty('--dock-top-resizer', '6px');
	contentEl.style.setProperty('--dock-bottom-height', `${bottomDockHeight}px`);
	contentEl.style.setProperty('--dock-bottom-resizer', '6px');

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
	const dockPeekDistance = 64;
	let dragSourceDock: { container: HTMLElement; axis: 'vertical' | 'horizontal' } | null = null;
	let forceShowLeftDock = false;
	let forceShowRightDock = false;
	let forceShowTopDock = false;
	let forceShowBottomDock = false;
	function parseOptionalNumber(value?: string): number | null {
		if (!value) return null;
		const parsed = Number(value);
		return Number.isFinite(parsed) ? parsed : null;
	}

	function getDockRequiredHeight(container: HTMLElement) {
		const windows = getDockWindows(container);
		if (windows.length === 0) return minDockHeight;
		const maxMin = windows.reduce((acc, win) => {
			const min = parseOptionalNumber(win.dataset.minHeight) ?? minDockHeight;
			return Math.max(acc, min);
		}, minDockHeight);
		return Math.max(minDockHeight, maxMin);
	}

	function updateDockVisibility() {
		const hasLeft = getDockWindows(leftDock.element).length > 0;
		const hasRight = getDockWindows(rightDock.element).length > 0;
		const hasTop = getDockWindows(topDock.element).length > 0;
		const hasBottom = getDockWindows(bottomDock.element).length > 0;
		const showLeft = hasLeft || forceShowLeftDock;
		const showRight = hasRight || forceShowRightDock;
		const showTop = hasTop || forceShowTopDock;
		const showBottom = hasBottom || forceShowBottomDock;

		if (hasTop) {
			topDockHeight = Math.max(topDockHeight, getDockRequiredHeight(topDock.element));
		}
		if (hasBottom) {
			bottomDockHeight = Math.max(bottomDockHeight, getDockRequiredHeight(bottomDock.element));
		}

		leftDock.element.style.display = showLeft ? 'flex' : 'none';
		rightDock.element.style.display = showRight ? 'flex' : 'none';
		topDock.element.style.display = showTop ? 'flex' : 'none';
		leftResizer.element.style.display = hasLeft ? 'block' : 'none';
		rightResizer.element.style.display = hasRight ? 'block' : 'none';
		topResizer.element.style.display = hasTop ? 'block' : 'none';
		bottomResizer.element.style.display = hasBottom ? 'block' : 'none';

		contentEl.style.setProperty('--dock-left-width', showLeft ? `${leftDockWidth}px` : '0px');
		contentEl.style.setProperty('--dock-right-width', showRight ? `${rightDockWidth}px` : '0px');
		contentEl.style.setProperty('--dock-left-resizer', hasLeft ? '6px' : '0px');
		contentEl.style.setProperty('--dock-right-resizer', hasRight ? '6px' : '0px');
		contentEl.style.setProperty('--dock-top-height', showTop ? `${topDockHeight}px` : '0px');
		contentEl.style.setProperty('--dock-top-resizer', hasTop ? '6px' : '0px');
		contentEl.style.setProperty('--dock-bottom-height', showBottom ? `${bottomDockHeight}px` : '0px');
		contentEl.style.setProperty('--dock-bottom-resizer', hasBottom ? '6px' : '0px');
	}

	function updateDockPeek(clientX: number, clientY: number) {
		const rect = contentEl.getBoundingClientRect();
		const nextForceLeft = clientX - rect.left <= dockPeekDistance;
		const nextForceRight = rect.right - clientX <= dockPeekDistance;
		const nextForceTop = clientY - rect.top <= dockPeekDistance;
		const nextForceBottom = rect.bottom - clientY <= dockPeekDistance;
		if (nextForceLeft === forceShowLeftDock && nextForceRight === forceShowRightDock && nextForceTop === forceShowTopDock && nextForceBottom === forceShowBottomDock) return;
		forceShowLeftDock = nextForceLeft;
		forceShowRightDock = nextForceRight;
		forceShowTopDock = nextForceTop;
		forceShowBottomDock = nextForceBottom;
		updateDockVisibility();
	}

	function applyFloatingConstraints(windowEl: HTMLElement, useDefaults: boolean) {
		const baseMinWidth = 150;
		const baseMinHeight = 50;
		const minWidth = Math.max(baseMinWidth, parseOptionalNumber(windowEl.dataset.minWidth) ?? baseMinWidth);
		const minHeight = Math.max(baseMinHeight, parseOptionalNumber(windowEl.dataset.minHeight) ?? baseMinHeight);
		const maxWidth = parseOptionalNumber(windowEl.dataset.maxWidth) ?? Infinity;
		const maxHeight = parseOptionalNumber(windowEl.dataset.maxHeight) ?? Infinity;
		const defaultWidth = parseOptionalNumber(windowEl.dataset.defaultWidth);
		const defaultHeight = parseOptionalNumber(windowEl.dataset.defaultHeight);

		windowEl.style.setProperty('--dock-min-width', `${minWidth}px`);
		windowEl.style.setProperty('--dock-min-height', `${minHeight}px`);
		if (Number.isFinite(maxWidth)) {
			windowEl.style.setProperty('--dock-max-width', `${maxWidth}px`);
		} else {
			windowEl.style.removeProperty('--dock-max-width');
		}
		if (Number.isFinite(maxHeight)) {
			windowEl.style.setProperty('--dock-max-height', `${maxHeight}px`);
		} else {
			windowEl.style.removeProperty('--dock-max-height');
		}

		const currentRect = windowEl.getBoundingClientRect();
		let nextWidth = useDefaults && defaultWidth ? defaultWidth : currentRect.width;
		let nextHeight = useDefaults && defaultHeight ? defaultHeight : currentRect.height;
		nextWidth = Math.min(maxWidth, Math.max(minWidth, nextWidth));
		nextHeight = Math.min(maxHeight, Math.max(minHeight, nextHeight));
		windowEl.style.width = `${nextWidth}px`;
		windowEl.style.height = `${nextHeight}px`;
	}

	function setFloating(windowEl: HTMLElement, x: number, y: number) {
		windowEl.classList.add(style.floatingWindow);
		windowEl.classList.add('floatingWindow');
		const defaultWidth = parseOptionalNumber(windowEl.dataset.defaultWidth) ?? 260;
		const defaultHeight = parseOptionalNumber(windowEl.dataset.defaultHeight) ?? 220;
		windowEl.style.setProperty('--dock-default-width', `${defaultWidth}px`);
		windowEl.style.setProperty('--dock-default-height', `${defaultHeight}px`);
		windowEl.style.width = `${defaultWidth}px`;
		windowEl.style.height = `${defaultHeight}px`;
		windowEl.style.left = `${x}px`;
		windowEl.style.top = `${y}px`;
		floatingLayer.element.appendChild(windowEl);
		applyFloatingConstraints(windowEl, true);
		updateDockVisibility();
	}

	function setDocked(windowEl: HTMLElement) {
		windowEl.classList.remove(style.floatingWindow);
		windowEl.classList.remove('floatingWindow');
		windowEl.style.removeProperty('--dock-min-width');
		windowEl.style.removeProperty('--dock-max-width');
		windowEl.style.removeProperty('--dock-min-height');
		windowEl.style.removeProperty('--dock-max-height');
		windowEl.style.removeProperty('--dock-default-width');
		windowEl.style.removeProperty('--dock-default-height');
		windowEl.style.width = '';
		windowEl.style.height = '';
		windowEl.style.left = '';
		windowEl.style.top = '';
		windowEl.style.flex = '';
		updateDockVisibility();
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

	function registerDock(container: HTMLElement) {
		container.addEventListener('dragover', (event) => {
			event.preventDefault();
		});
	}

	function enableDrag(windowEl: HTMLElement) {
		const titlebar = windowEl.querySelector('[data-role="dock-titlebar"]') as HTMLElement | null;
		const resizeHandle = windowEl.querySelector(`.${windowStyle.resizeHandle}`) as HTMLElement | null;
		if (!titlebar) return;
			titlebar.addEventListener('mousedown', (event) => {
			if (event.button !== 0) return;
			event.preventDefault();
			event.stopPropagation();

			const contentRect = contentEl.getBoundingClientRect();
			const windowRect = windowEl.getBoundingClientRect();
				let offsetX = event.clientX - windowRect.left;
				let offsetY = event.clientY - windowRect.top;
				const startX = event.clientX;
				const startY = event.clientY;
				let didUndock = windowEl.classList.contains(style.floatingWindow);
			const parent = windowEl.parentElement as HTMLElement | null;
			if (parent === leftDock.element) {
				dragSourceDock = { container: leftDock.element, axis: 'vertical' };
			} else if (parent === rightDock.element) {
				dragSourceDock = { container: rightDock.element, axis: 'vertical' };
				} else if (parent === topDock.element) {
					dragSourceDock = { container: topDock.element, axis: 'horizontal' };
			} else if (parent === bottomDock.element) {
				dragSourceDock = { container: bottomDock.element, axis: 'horizontal' };
			} else {
				dragSourceDock = null;
			}

			function handleMove(moveEvent: MouseEvent) {
					if (!didUndock) {
						const dx = Math.abs(moveEvent.clientX - startX);
						const dy = Math.abs(moveEvent.clientY - startY);
						if (dx < 3 && dy < 3) return;

						const x = windowRect.left - contentRect.left;
						const y = windowRect.top - contentRect.top;
						setFloating(windowEl, x, y);
						didUndock = true;
						if (dragSourceDock?.container === bottomDock.element || dragSourceDock?.container === topDock.element) {
							offsetX = windowRect.width / 2;
						}
						if (dragSourceDock) {
							rebuildDock(dragSourceDock.container, dragSourceDock.axis);
							applyDockSizing(dragSourceDock.container, dragSourceDock.axis);
						}
					}

				const currentRect = windowEl.getBoundingClientRect();
				const x = (dragSourceDock?.container === bottomDock.element || dragSourceDock?.container === topDock.element)
					? moveEvent.clientX - contentRect.left - currentRect.width / 2
					: moveEvent.clientX - contentRect.left - offsetX;
				const y = moveEvent.clientY - contentRect.top - offsetY;
				windowEl.style.left = `${x}px`;
				windowEl.style.top = `${y}px`;
				updateDockPeek(moveEvent.clientX, moveEvent.clientY);
			}

			function handleUp(upEvent: MouseEvent) {
				window.removeEventListener('mousemove', handleMove);
				window.removeEventListener('mouseup', handleUp);

				const docks = [
					{ container: leftDock.element, axis: 'vertical' as const },
					{ container: rightDock.element, axis: 'vertical' as const },
					{ container: topDock.element, axis: 'horizontal' as const },
					{ container: bottomDock.element, axis: 'horizontal' as const },
				];

				const targetDock = docks.find((dock) => {
					const rect = dock.container.getBoundingClientRect();
					return upEvent.clientX >= rect.left && upEvent.clientX <= rect.right
						&& upEvent.clientY >= rect.top && upEvent.clientY <= rect.bottom;
				});

				if (targetDock) {
					setDocked(windowEl);
					insertByAxis(targetDock.container, windowEl, upEvent.clientX, upEvent.clientY, targetDock.axis);
					rebuildDock(targetDock.container, targetDock.axis);
					applyDockSizing(targetDock.container, targetDock.axis);
					if (dragSourceDock && dragSourceDock.container !== targetDock.container) {
						rebuildDock(dragSourceDock.container, dragSourceDock.axis);
						applyDockSizing(dragSourceDock.container, dragSourceDock.axis);
					}
					dragSourceDock = null;
				}

				forceShowLeftDock = false;
				forceShowRightDock = false;
				forceShowTopDock = false;
				forceShowBottomDock = false;
				updateDockVisibility();
			}

			window.addEventListener('mousemove', handleMove);
			window.addEventListener('mouseup', handleUp);
		});

		resizeHandle?.addEventListener('mousedown', (event) => {
			if (!windowEl.classList.contains(style.floatingWindow)) return;
			if (windowEl.dataset.resizable !== 'true') return;
			event.preventDefault();
			event.stopPropagation();

			const startRect = windowEl.getBoundingClientRect();
			const startX = event.clientX;
			const startY = event.clientY;
			const minW = Math.max(150, parseOptionalNumber(windowEl.dataset.minWidth) ?? 150);
			const minH = Math.max(50, parseOptionalNumber(windowEl.dataset.minHeight) ?? 50);
			const maxW = parseOptionalNumber(windowEl.dataset.maxWidth) ?? Infinity;
			const maxH = parseOptionalNumber(windowEl.dataset.maxHeight) ?? Infinity;

			function handleResize(moveEvent: MouseEvent) {
				const nextW = Math.min(maxW, Math.max(minW, startRect.width + (moveEvent.clientX - startX)));
				const nextH = Math.min(maxH, Math.max(minH, startRect.height + (moveEvent.clientY - startY)));
				windowEl.style.width = `${nextW}px`;
				windowEl.style.height = `${nextH}px`;
			}

			function stopResize() {
				window.removeEventListener('mousemove', handleResize);
				window.removeEventListener('mouseup', stopResize);
			}

			window.addEventListener('mousemove', handleResize);
			window.addEventListener('mouseup', stopResize);
		});
		return;
	}

	function attachControls(windowEl: HTMLElement) {
		const body = windowEl.querySelector(`.${windowStyle.body}`) as HTMLElement | null;
		if (!body) return;
		body.textContent = '';

		const controls = Div().class(windowStyle.controls).nodes([
			Div().class(windowStyle.controlLabel).text('Window Controls'),
		]);

		function addNumberControl(label: string, key: keyof HTMLElement['dataset']) {
			const input = TextInput().class(windowStyle.controlInput);
			input.element.placeholder = 'auto';
			input.value(windowEl.dataset[key] ?? '');
			input.on('input', (event) => {
				const value = (event.target as HTMLInputElement).value.trim();
				if (value) {
					windowEl.dataset[key] = value;
				} else {
					windowEl.dataset[key] = '';
				}
				if (windowEl.classList.contains(style.floatingWindow)) {
					applyFloatingConstraints(windowEl, key === 'defaultWidth' || key === 'defaultHeight');
				}
			});

			controls.nodes([
				Div().class(windowStyle.controlRow).nodes([
					Span(label).class(windowStyle.controlLabel),
					input,
				]),
			]);
		}

		addNumberControl('Width (default)', 'defaultWidth');
		addNumberControl('Height (default)', 'defaultHeight');
		addNumberControl('Min width', 'minWidth');
		addNumberControl('Max width', 'maxWidth');
		addNumberControl('Min height', 'minHeight');
		addNumberControl('Max height', 'maxHeight');

		const resizableToggle = Button(`Resizable: ${windowEl.dataset.resizable === 'false' ? 'no' : 'yes'}`).class(windowStyle.controlToggle);
		resizableToggle.on('click', () => {
			const next = windowEl.dataset.resizable === 'false' ? 'true' : 'false';
			windowEl.dataset.resizable = next;
			resizableToggle.text(`Resizable: ${next === 'true' ? 'yes' : 'no'}`);
		});

		controls.nodes([
			Div().class(windowStyle.controlRow).nodes([
				Span('Resizable').class(windowStyle.controlLabel),
				resizableToggle,
			]),
		]);

		body.appendChild(controls.element);
	}

	leftResizer.element.addEventListener('mousedown', startDrag((event) => {
		const rect = contentEl.getBoundingClientRect();
		const next = Math.min(maxDockWidth, Math.max(minDockWidth, event.clientX - rect.left));
		contentEl.style.setProperty('--dock-left-width', `${next}px`);
		leftDockWidth = next;
	}));

	rightResizer.element.addEventListener('mousedown', startDrag((event) => {
		const rect = contentEl.getBoundingClientRect();
		const next = Math.min(maxDockWidth, Math.max(minDockWidth, rect.right - event.clientX));
		contentEl.style.setProperty('--dock-right-width', `${next}px`);
		rightDockWidth = next;
	}));

	topResizer.element.addEventListener('mousedown', startDrag((event) => {
		const rect = contentEl.getBoundingClientRect();
		const requiredMin = getDockRequiredHeight(topDock.element);
		const next = Math.min(maxDockHeight, Math.max(requiredMin, event.clientY - rect.top));
		contentEl.style.setProperty('--dock-top-height', `${next}px`);
		topDockHeight = next;
	}));

	bottomResizer.element.addEventListener('mousedown', startDrag((event) => {
		const rect = contentEl.getBoundingClientRect();
		const requiredMin = getDockRequiredHeight(bottomDock.element);
		const next = Math.min(maxDockHeight, Math.max(requiredMin, rect.bottom - event.clientY));
		contentEl.style.setProperty('--dock-bottom-height', `${next}px`);
		bottomDockHeight = next;
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

	registerDock(leftDock.element);
	registerDock(rightDock.element);
	registerDock(topDock.element);
	registerDock(bottomDock.element);

	rebuildDock(leftDock.element, 'vertical');
	rebuildDock(rightDock.element, 'vertical');
	rebuildDock(topDock.element, 'horizontal');
	rebuildDock(bottomDock.element, 'horizontal');
	updateDockVisibility();

	[leftWindow, rightTopWindow, rightBottomWindow, bottomWindow, topWindow].forEach((windowNode) => {
		enableDrag(windowNode.element);
		attachControls(windowNode.element);
	});

	return Section('workspace').class(style.workspace).nodes([
		tabs,
		content,
	]);
}
