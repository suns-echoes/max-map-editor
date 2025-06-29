import { Div, Section } from '^lib/reactive/html-node.elements.ts';
import { MenuButton } from '^src/ui/components/buttons/menu-button.component';

import { Submenu } from '../submenu/submenu.component.ts';

import type { MainMenu } from './main-menu.types.ts';
import style from './main-menu.module.css';


export function MainMenu(menu: MainMenu) {
	let previousSubmenu: HTMLElement | null = null;

	function openSubmenu(event: MouseEvent, element: HTMLElement) {
		event.stopPropagation();

		if (previousSubmenu === element) {
			element.classList.remove(style.open);
			previousSubmenu = null;
			return;
		} else if (previousSubmenu) {
			previousSubmenu.classList.remove(style.open);
		}
		element.classList.add(style.open);
		previousSubmenu = element;

		document.addEventListener('click', () => {
			if (previousSubmenu) {
				previousSubmenu.classList.remove(style.open);
				previousSubmenu = null;
			}
		}, { once: true });
	}

	return (
		Section('main-menu').class(style.mainMenu).nodes(
			menu.map(item => {
				const hasSubmenu = !!item.submenu && item.submenu.length > 0;
				let menuButton;

				const menuItem = Div().nodes(
					hasSubmenu ? [
						menuButton = MenuButton('file-menu').text(item.label),
						Submenu(item.submenu!),
					] : [
						menuButton = MenuButton('file-menu').text(item.label),
					],
				);

				if (item.disabled) {
					menuButton.disable();
				} else {
					if (item.action) {
						menuButton.addEventListener('click', item.action);
					}
					if (hasSubmenu) {
						menuButton.addEventListener('click', (event) => openSubmenu(event, menuItem.element));
					}
				}

				return menuItem;
			}),
		)
	);
}
