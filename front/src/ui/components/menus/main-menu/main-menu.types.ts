import type { Submenu } from '../submenu/submenu.types.ts';

export type MainMenu = MainMenuItem[];

export interface MainMenuItem {
	label: string;
	help: string;
	submenu?: Submenu;
	action?: () => Promise<void>;
	disabled?: boolean;
}
