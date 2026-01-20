import { xlog } from '^lib/xlog/xlog.ts';

import { CloseAppAction } from '^actions/app/close-app.action.ts';
import { NewMapFromImageAction } from '^actions/new-map/new-map-from-image.action.ts';
import { importWrlFromFile, downloadWrlFile } from '^src/features/wrl-io/index.ts';
import { PaletteEditorState } from '^src/features/palette-editor/index.ts';

import { MainMenu } from '^src/ui/components/menus/main-menu/main-menu.component.ts';


async function ImportWrlAction() {
	const input: HTMLInputElement = document.createElement('input');
	input.type = 'file';
	input.accept = '.wrl';

	return new Promise<void>((resolve) => {
		input.onchange = async () => {
			const file = input.files?.[0];
			if (file) {
				const result = await importWrlFromFile(file);
				if (result.success) {
					xlog.info(`Imported: ${result.mapName} (${result.width}x${result.height})`);
				} else {
					xlog.error(`Import failed: ${result.error}`);
				}
			}
			resolve();
		};
		input.oncancel = () => resolve();
		input.click();
	});
}

async function ExportWrlAction() {
	const result = downloadWrlFile();
	if (result.success) {
		xlog.info(`Exported: ${result.fileName}`);
	} else {
		xlog.error(`Export failed: ${result.error}`);
	}
}


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
					label: '-',
				},
				{
					label: 'Import WRL...',
					action: ImportWrlAction,
				},
				{
					label: 'Export WRL',
					action: ExportWrlAction,
				},
				{
					label: '-',
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
				{
					label: '-',
				},
				{
					label: 'Palette Editor',
					action: async () => PaletteEditorState.togglePanel(),
				},
			],
		},
		{
			label: 'About',
			submenu: [],
		},
	]);
}
