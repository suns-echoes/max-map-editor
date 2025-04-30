import { printDebugInfo } from '^utils/debug/debug.ts';
import { Div, Section } from '^utils/reactive/html-node.elements.ts';
import { MenuButton } from '^src/ui/components/buttons/menu-button.html.ts';

import style from './main-menu.module.css';


export function MainMenu() {
	printDebugInfo('UI::MainMenu');

	return (
		Section('main-menu').class(style.mainMenu).nodes([
			Div().nodes([
				MenuButton('file-menu').text('File'),
				MenuButton('edit-menu').text('Edit'),
				MenuButton('utilities-menu').text('Utilities'),
				MenuButton('view-menu').text('View'),
				MenuButton('about-menu').text('About'),
			]),
		])
	);
}
