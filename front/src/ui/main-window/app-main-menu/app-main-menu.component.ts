import { xlog } from '^lib/xlog/xlog.ts';

import { MainMenu } from '^src/ui/components/menus/main-menu/main-menu.component.ts';


export function AppMainMenu() {
	xlog.info('UI::AppMainMenu');

	return MainMenu([
		{
			label: 'File',
			submenu: [
				{
					label: 'Create new map ▶',
					submenu: [
						{ label: 'Using tiles 🡵' },
						{ label: 'From image 🡵' },
						{ label: 'From WRL 🡵' },
					],
				},
				{
					label: 'Load project ▶',
					submenu: [
						{ label: 'Browse… 🡵' },
						{ label: 'Recent… ▶', submenu: [
							{ label: 'CRATER_1' },
							{ label: 'GREEN_1' },
							{ label: '...' },
						] },
					],
				},
				{
					label: 'Quick load ▶',
					submenu: [
						{ label: 'CRATER_1' },
						{ label: 'GREEN_1' },
						{ label: '...' },
					],
				},
				{
					label: 'Load previous ▶',
					submenu: [
						{ label: 'Map 1' },
						{ label: 'Map 2' },
						{ label: '...' },
					],
				},
				{
					label: 'Save project ▶',
					submenu: [
						{ label: 'Save 🡵' },
						{ label: 'Save As… 🡵' },
					],
				},
				{
					label: 'Save project copy ▶',
					submenu: [
						{ label: 'Save Copy… 🡵' },
					],
				},
				{ label: 'Close project' },
				{ label: '-' },
				{
					label: 'Export to WRL ▶',
					submenu: [
						{ label: 'Export current 🡵' },
						{ label: 'Export selection 🡵' },
					],
				},
				{
					label: 'Import WRL ▶',
					submenu: [
						{ label: 'Import into project 🡵' },
						{ label: 'Open as new project 🡵' },
					],
				},
				{ label: '-' },
				{
					label: 'Export as image ▶',
					submenu: [
						{ label: 'Export image 🡵' },
						{ label: 'Export image with overlays 🡵' },
					],
				},
			],
		},
		{
			label: 'Edit',
			submenu: [
				{ label: 'Undo' },
				{ label: 'Redo' },
				{ label: 'Undo History' },
				{ label: '-' },
				{ label: 'Cut' },
				{ label: 'Copy' },
				{ label: 'Paste' },
				{ label: 'Clear' },
				{ label: '-' },
				{ label: 'Preferences' },
			],
		},
		{
			label: 'Mode',
			submenu: [
				{
					label: 'Render Mode ▶',
					submenu: [
						{ label: '🔘 Default' },
						{ label: '🔘 Terrain' },
						{ label: '🔘 Pass' },
					],
				},
				{
					label: 'Tile Layer ▶',
					submenu: [
						{ label: '🔘 Ground' },
						{ label: '🔘 Water' },
						{ label: '🔘 Objects' },
					],
				},
				{ label: '-' },
				{ label: '🔘 Pass Editor' },
				{ label: '🔘 Tile Pixel Editor' },
				{ label: '-' },
				{
					label: 'Immersive mode ▶',
					submenu: [
						{ label: '🔘 Off' },
						{ label: '🔘 Minimal UI' },
						{ label: '🔘 Fullscreen' },
					],
				},
			],
		},
		{
			label: 'Snapshot',
			submenu: [
				{ label: 'Take Snapshot' },
				{ label: '-' },
				{
					label: 'Revert to Snapshot ▶',
					submenu: [
						{ label: 'User snapshot A' },
						{ label: 'User snapshot B' },
						{ label: '...' },
					],
				},
				{ label: 'Show all Snapshot' },
				{ label: 'User snapshot A' },
				{ label: 'User snapshot B' },
				{ label: '-' },
				{ label: 'Clear Snapshots' },
			],
		},
		{
			label: 'View',
			submenu: [
				{ label: '☑ Show grid' },
				{
					label: 'Zoom ▶',
					submenu: [
						{ label: '🔘 25%' },
						{ label: '🔘 50%' },
						{ label: '🔘 100%' },
						{ label: '🔘 200%' },
					],
				},
				{ label: '-' },
				{ label: '☑ Tile Explorer' },
				{ label: '☑ Color Palette' },
				{ label: '☑ Pass Types Palette' },
				{ label: '☑ Minimap' },
				{ label: '☑ Templates Explorer' },
				{ label: '☑ Tile Packs Manager' },
				{ label: '☑ Tile Editing Toolbox' },
			],
		},
		{
			label: 'Select',
			submenu: [
				{ label: 'Select ALL' },
				{ label: 'INVERT selection' },
				{ label: 'CLEAR selection' },
				{ label: '-' },
				{ label: 'ADD to selection' },
				{ label: 'SUBTRACT from selection' },
				{ label: '-' },
				{ label: 'Select SIMILAR' },
			],
		},
		{
			label: 'Templates',
			submenu: [
				{ label: 'Create template from selection' },
				{ label: '-' },
				{ label: 'Clone selected template' },
				{ label: '-' },
				{ label: 'Create new template' },
				{ label: 'Open template explorer ▶', submenu: [
					{ label: 'Open 🡵' },
					{ label: 'Open recent ▶', submenu: [
						{ label: 'Template A' },
						{ label: 'Template B' },
						{ label: '...' },
					] },
				] },
				{ label: 'Delete selected template' },
				{ label: 'Import template ▶', submenu: [
					{ label: 'Import from file 🡵' },
				] },
				{ label: 'Export selection as template ▶', submenu: [
					{ label: 'Export to file 🡵' },
				] },
			],
		},
		{
			label: 'Tools',
			submenu: [
				{ label: 'Auto fix shore' },
				{ label: '-' },
				{
					label: 'Auto generate pass table ▶',
					submenu: [
						{ label: 'Generate for current map' },
						{ label: 'Generate for all layers' },
					],
				},
			],
		},
		{
			label: 'Windows',
			submenu: [
				{ label: 'Tile Tools' },
				{ label: 'Template Tools' },
				{ label: 'Palette Tools' },
				{ label: 'Hand Tools' },
				{ label: 'Pixel Tools' },
				{ label: 'Advanced Tools' },
				{ label: 'Selection Tools' },
				{ label: 'Auto Tools' },
			],
		},
		{
			label: 'Help',
			submenu: [
				{ label: 'Documentation' },
				{ label: 'Check for updates' },
				{ label: '-' },
				{ label: 'About' },
			],
		},
	]);
}
