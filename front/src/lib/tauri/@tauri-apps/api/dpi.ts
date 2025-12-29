import { isTauri } from '@tauri-apps/api/core';
import {
	PhysicalPosition as _PhysicalPosition,
	LogicalPosition as _LogicalPosition,
	PhysicalSize,
	LogicalSize,
} from '@tauri-apps/api/dpi';


const LogicalPosition = isTauri() ? _LogicalPosition : class LogicalPosition {
	type: string = '';
	x: number = 0;
	y: number = 0;
	// @ts-ignore
	constructor(x: number, y: number) {}
	// @ts-ignore
	toPhysical(scaleFactor: number): _PhysicalPosition { return new PhysicalPosition(0, 0); }
} as typeof _LogicalPosition;

const PhysicalPosition = isTauri() ? _PhysicalPosition : class PhysicalPosition {
	type: string = '';
	x: number = 0;
	y: number = 0;
	// @ts-ignore
	constructor(x: number, y: number) {}
	// @ts-ignore
	toLogical(scaleFactor: number): _LogicalPosition { return new LogicalPosition(0, 0); }
} as typeof _PhysicalPosition;


export {
	LogicalPosition,
	PhysicalPosition,
	LogicalSize,
	PhysicalSize,
};
