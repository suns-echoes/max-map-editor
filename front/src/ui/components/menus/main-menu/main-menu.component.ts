import { Div, Section } from '^reactive/reactive-node.elements.ts';
import { MenuButton } from '^src/ui/components/buttons/menu-button.component';

import { Submenu } from '../submenu/submenu.component.ts';

import type { MainMenu } from './main-menu.types.ts';
import style from './main-menu.module.css';
import { ReactiveNode } from '^reactive/reactive-node.ts';


export function MainMenu(menu: MainMenu) {
	let previousSubmenu: HTMLElement | null = null;
	let isMenuBlocked = false;
	let mainMenu: ReactiveNode;

	function closeMenu() {
		if (previousSubmenu) {
			previousSubmenu.classList.remove(style.open);
			previousSubmenu = null;
		}
	}

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
	}

	document.addEventListener('click', closeMenu);

	function handleDestroy() {
		document.removeEventListener('click', closeMenu);
	}

	function requestMenuLock() {
		if (isMenuBlocked) return false;
		mainMenu.element.classList.add(style.blocked);
		return isMenuBlocked = true;
	}

	function unlockMenu() {
		isMenuBlocked = false;
		mainMenu.element.classList.remove(style.blocked);
	}

	mainMenu = (
		Section('main-menu').class(style.mainMenu).onDispose(handleDestroy).nodes(
			menu.map(function (item) {
				const hasSubmenu = !!item.submenu && item.submenu.length > 0;
				let menuButton;

				const menuItem = Div().nodes(
					(hasSubmenu ? [
						menuButton = MenuButton().text(item.label),
						Submenu({
							menu: item.submenu!,
							requestMenuLock,
							unlockMenu,
						}),
					] : [
						menuButton = MenuButton().text(item.label),
					]) as any,
				);

				if (item.help) {
					menuItem.element.dataset.help = item.help;
				}

				if (item.disabled) {
					menuButton.disable();
				} else {
					if (item.action) {
						menuButton.on('click', async function () {
							if (requestMenuLock()) return;
							await item.action?.();
							unlockMenu();
						});
					}
					if (hasSubmenu) {
						menuButton.on('click', function (event) {
							if (isMenuBlocked) return;
							openSubmenu(event, menuItem.element);
						});
					}
				}

				return menuItem;
			}),
		)
	);

	return mainMenu;
}
