import { Value } from '^reactive/value.ts';
import { Button, Div, Section, Span } from '^reactive/reactive-node.elements.ts';
import { xlog } from '^lib/xlog/xlog.ts';
import { WGLMap } from '../wgl-map/wgl-map.component.ts';

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

	const content = Div('workspace-content').class(style.content).nodes([
		Div('dock-left').class(style.dockLeft).text('Left Dock'),
		leftResizer,
		Div('workspace-center').class(style.center).nodes([
			WGLMap(),
		]),
		rightResizer,
		Div('dock-right').class(style.dockRight).text('Right Dock'),
		bottomResizer,
		Div('dock-bottom').class(style.dockBottom).text('Bottom Dock'),
	]);

	const contentEl = content.element as HTMLElement;
	contentEl.style.setProperty('--dock-left-width', '240px');
	contentEl.style.setProperty('--dock-right-width', '240px');
	contentEl.style.setProperty('--dock-bottom-height', '180px');

	function startDrag(onMove: (dx: number, dy: number) => void) {
		let lastX = 0;
		let lastY = 0;

		function handleMove(event: MouseEvent) {
			const dx = event.clientX - lastX;
			const dy = event.clientY - lastY;
			lastX = event.clientX;
			lastY = event.clientY;
			onMove(dx, dy);
		}

		function handleUp() {
			window.removeEventListener('mousemove', handleMove);
			window.removeEventListener('mouseup', handleUp);
		}

		return function handleDown(event: MouseEvent) {
			event.preventDefault();
			lastX = event.clientX;
			lastY = event.clientY;
			window.addEventListener('mousemove', handleMove);
			window.addEventListener('mouseup', handleUp);
		};
	}

	const minDockWidth = 160;
	const maxDockWidth = 480;
	const minDockHeight = 120;
	const maxDockHeight = 360;

	leftResizer.element.addEventListener('mousedown', startDrag((dx) => {
		const current = parseInt(contentEl.style.getPropertyValue('--dock-left-width') || '240', 10);
		const next = Math.min(maxDockWidth, Math.max(minDockWidth, current + dx));
		contentEl.style.setProperty('--dock-left-width', `${next}px`);
	}));

	rightResizer.element.addEventListener('mousedown', startDrag((dx) => {
		const current = parseInt(contentEl.style.getPropertyValue('--dock-right-width') || '240', 10);
		const next = Math.min(maxDockWidth, Math.max(minDockWidth, current - dx));
		contentEl.style.setProperty('--dock-right-width', `${next}px`);
	}));

	bottomResizer.element.addEventListener('mousedown', startDrag((_dx, dy) => {
		const current = parseInt(contentEl.style.getPropertyValue('--dock-bottom-height') || '180', 10);
		const next = Math.min(maxDockHeight, Math.max(minDockHeight, current - dy));
		contentEl.style.setProperty('--dock-bottom-height', `${next}px`);
	}));

	return Section('workspace').class(style.workspace).nodes([
		tabs,
		content,
	]);
}
