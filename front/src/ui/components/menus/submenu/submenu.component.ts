import { Div, Section } from '^reactive/reactive-node.elements.ts';

import { BigOutset } from '../../frames/big-outset.component.ts';
import { SubmenuButton } from '../../buttons/submenu-button.component.ts';

import type { Submenu, SubmenuItem } from './submenu.types.ts';

import style from './submenu.module.css';


interface SubmenuProps {
	menu: Submenu;
	requestMenuLock: () => boolean;
	unlockMenu: () => void;
	variant?: 'root' | 'nested';
}

export function Submenu({ menu, requestMenuLock: requestBlockMenu, unlockMenu: unblockMenu, variant = 'root' }: SubmenuProps) {
	let previousNested: HTMLElement | null = null;

	function isSubmenuItem(item: SubmenuItem | { label: '-' }): item is SubmenuItem {
		return item.label !== '-';
	}

	function closeNested() {
		if (previousNested) {
			previousNested.classList.remove(style.open);
			previousNested = null;
		}
	}

	const className = variant === 'nested' ? `${style.submenu} ${style.nested}` : style.submenu;

	return Section('submenu').class(className).nodes([
		BigOutset().class(style.panel).nodes(menu.map(item => {
			if (!isSubmenuItem(item)) {
				return Div().class(style.separator);
			}

			const hasSubmenu = !!item.submenu && item.submenu.length > 0;
			const itemContainer = Div().class(style.submenuItem);
			const button = SubmenuButton(item.label);

			if (item.disabled) {
				button.disable();
			} else if (item.action) {
				button.on('click', async function () {
					if (!requestBlockMenu()) return;
					await item.action?.();
					unblockMenu();
				});
			}

			itemContainer.nodes([
				button,
				hasSubmenu ? Submenu({
					menu: item.submenu!,
					requestMenuLock: requestBlockMenu,
					unlockMenu: unblockMenu,
					variant: 'nested',
				}) : null,
			].filter(Boolean) as any);

			if (hasSubmenu && !item.disabled) {
				button.on('click', function (event) {
					event.stopPropagation();
					if (previousNested === itemContainer.element) {
						closeNested();
						return;
					}
					closeNested();
					itemContainer.element.classList.add(style.open);
					previousNested = itemContainer.element;
				});
			}

			return itemContainer;
		})),
	]);
}
