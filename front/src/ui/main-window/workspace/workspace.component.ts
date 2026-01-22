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

	const content = Div('workspace-content').class(style.content).nodes([
		WGLMap(),
	]);

	return Section('workspace').class(style.workspace).nodes([
		tabs,
		content,
	]);
}
