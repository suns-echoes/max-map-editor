import { Div, Section } from '^lib/reactive/html-node.elements.ts';

import { SubmenuButton } from '../../buttons/submenu-button.component.ts';

import type { Submenu } from './submenu.types.ts';

import style from './submenu.module.css';


export function Submenu(menu: Submenu) {
	return Section('submenu').class(style.submenu).nodes([
		Div().nodes(menu.map(item => {
			if (item.label === '-') {
				return Div().class(style.separator);
			} else {
				const button = SubmenuButton(item.label);
				if (item.disabled) {
					button.disable();
				} else if (item.action) {
					button.addEventListener('click', item.action);
				}
				return button;
			}
		})),
	]);
}
