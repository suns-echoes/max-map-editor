import type { Submenu } from '../submenu/submenu.types.ts';

export type MainMenu = MainMenuItem[];

export interface MainMenuItem {
	label: string;
	submenu?: Submenu;
	action?: () => void;
	disabled?: boolean;
}
