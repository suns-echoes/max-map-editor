import {
	getCurrentWindow as _getCurrentWindow,
	PhysicalPosition as _PhysicalPosition,
	LogicalPosition as _LogicalPosition,
	PhysicalSize,
	LogicalSize,
} from '@tauri-apps/api/window';

import { isTauri } from '^tauri/is-tauri.ts';


const getCurrentWindow = isTauri ? _getCurrentWindow : () => ({
	// @ts-ignore
	setSize: (size: LogicalSize | PhysicalSize) => Promise.resolve(undefined),
	setPosition: () => Promise.resolve(),
	maximize: () => Promise.resolve(),
	scaleFactor: () => Promise.resolve(1),
});

const LogicalPosition = isTauri ? _LogicalPosition : class LogicalPosition {
	type: string = '';
	x: number = 0;
	y: number = 0;
	// @ts-ignore
	constructor(x: number, y: number) {}
	// @ts-ignore
	toPhysical(scaleFactor: number): _PhysicalPosition { return new PhysicalPosition(0, 0); }
};

const PhysicalPosition = isTauri ? _PhysicalPosition : class PhysicalPosition {
	type: string = '';
	x: number = 0;
	y: number = 0;
	// @ts-ignore
	constructor(x: number, y: number) {}
	// @ts-ignore
	toLogical(scaleFactor: number): _LogicalPosition { return new LogicalPosition(0, 0); }
};


export {
	getCurrentWindow,
	LogicalPosition,
	PhysicalPosition,
	LogicalSize,
	PhysicalSize,
};
