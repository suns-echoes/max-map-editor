import { Div, Section } from '^lib/reactive/html-node.elements.ts';

import { BigOutset } from '../../frames/big-outset.component.ts';
import { SubmenuButton } from '../../buttons/submenu-button.component.ts';

import type { Submenu } from './submenu.types.ts';

import style from './submenu.module.css';


interface SubmenuProps {
	menu: Submenu;
	requestMenuLock: () => boolean;
	unlockMenu: () => void;
}

export function Submenu({ menu, requestMenuLock: requestBlockMenu, unlockMenu: unblockMenu }: SubmenuProps) {
	return Section('submenu').class(style.submenu).nodes([
		BigOutset().nodes(menu.map(item => {
			if (item.label === '-') {
				return Div().class(style.separator);
			} else {
				const button = SubmenuButton(item.label);
				if (item.disabled) {
					button.disable();
				} else if (item.action) {
					button.addEventListener('click', async function () {
						if (!requestBlockMenu()) return;
						await item.action?.();
						unblockMenu();
					});
				}
				return button;
			}
		})),
	]);
}
