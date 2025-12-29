export type Submenu = (SubmenuItem | SubmenuSeparator)[];

export interface SubmenuItem {
	label: string;
	action?: () => Promise<void>;
	disabled?: boolean;
}

export interface SubmenuSeparator {
	label: '-';
	action: undefined;
	disabled: undefined;
}
