import { getCurrentWindow, PhysicalPosition, PhysicalSize } from '@tauri-apps/api/window';
import { printDebugInfo } from '^utils/debug/debug.ts';

import type { XSetupScreen } from '^components/x-setup-screen/x-setup-screen.ts';
import '^components/x-setup-screen/x-setup-screen.ts';


export async function showSetupScreen() {
	await printDebugInfo('entering setup screen');

	const endSetup = Promise.withResolvers<void>();

	const width = 640;
	const height = 460;

	const currentWindow = getCurrentWindow();

	await currentWindow.setSize(
		new PhysicalSize(width, height)
	);

	// TODO: #18 Fix setup window size
	console.log('innerSize', (await currentWindow.innerSize()).width, 'x', (await currentWindow.innerSize()).height);
	console.log('outerSize', (await currentWindow.outerSize()).width, 'x', (await currentWindow.outerSize()).height);

	await currentWindow.setPosition(
		new PhysicalPosition((window.screen.width - width) / 2, (window.screen.height - height) / 2)
	);

	const xSetupScreen = document.createElement<XSetupScreen>('x-setup-screen');

	document.body.appendChild(xSetupScreen);

	xSetupScreen.ondone = function () {
		console.log('<< Setup done.');
		document.body.removeChild(xSetupScreen);
		endSetup.resolve();
	}

	return endSetup.promise;
}
