import { xlog } from '^lib/xlog/xlog.ts';

import { CloseAppAction } from '^actions/app/close-app.action.ts';
import { NewMapFromImageAction } from '^actions/new-map/new-map-from-image.action.ts';

import { MainMenu } from '^src/ui/components/menus/main-menu/main-menu.component.ts';


export function AppMainMenu() {
	xlog.info('UI::AppMainMenu');

	return MainMenu([
		{
			label: 'File',
			submenu: [
				{
					label: 'New Project',
					disabled: true,
				},
				{
					label: 'New Map from Image',
					action: NewMapFromImageAction,
				},
				{
					label: 'Save File',
					disabled: true,
				},
				{
					label: 'Save As...',
					disabled: true,
				},
				{
					label: 'Close File',
					disabled: true,
				},
				{
					label: '-',
				},
				{
					label: 'Exit',
					action: CloseAppAction,
				},
			],
		},
		{
			label: 'Edit',
			submenu: [
				{
					label: 'Undo',
					disabled: true,
				},
				{
					label: 'Redo',
					disabled: true,
				},
				{
					label: 'Cut',
					disabled: true,
				},
				{
					label: 'Copy',
					disabled: true,
				},
				{
					label: 'Paste',
					disabled: true,
				},
			],
		},
		{
			label: 'Utilities',
			submenu: [],
		},
		{
			label: 'View',
			submenu: [
				{
					label: 'Show Grid',
					disabled: true,
				},
				{
					label: 'Show Cell Types',
					disabled: true,
				},
			],
		},
		{
			label: 'About',
			submenu: [],
		},
	]);
}
