import { getCurrentWindow, PhysicalPosition, PhysicalSize } from '@tauri-apps/api/window';
import { printDebugInfo } from '^utils/debug/debug.ts';
import { setupDoneSignal, SetupScreen } from '^src/ui/main-window/setup-screen/setup-screen.html.ts';
import { Signal } from '^utils/reactive/signal.class.ts';


export async function showSetupScreen() {
	await printDebugInfo('App::showSetupScreen');

	const width = 640;
	const height = 480;

	const currentWindow = getCurrentWindow();
	await currentWindow.setSize(new PhysicalSize(width, height));
	await currentWindow.setPosition(new PhysicalPosition(
		(window.screen.width - width) / 2,
		(window.screen.height - height) / 2,
	));

	document.body.appendChild(SetupScreen().element);

	return Signal.toPromise(setupDoneSignal);
}
