import { isTauri } from '@tauri-apps/api/core';
import {
	getCurrentWindow as _getCurrentWindow,
} from '@tauri-apps/api/window';


const getCurrentWindow = isTauri() ? _getCurrentWindow : () => ({
	// @ts-ignore
	setSize: (size: LogicalSize | PhysicalSize) => Promise.resolve(undefined),
	setPosition: () => Promise.resolve(),
	maximize: () => Promise.resolve(),
	scaleFactor: () => Promise.resolve(1),
});


export {
	getCurrentWindow,
};
