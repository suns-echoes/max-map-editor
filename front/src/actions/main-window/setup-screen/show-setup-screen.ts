import { getCurrentWindow, PhysicalPosition, PhysicalSize } from '@tauri-apps/api/window';
import { xlog } from '^lib/xlog/xlog.ts';
import { setupDoneSignal, SetupScreen } from '^src/ui/setup-screen/setup-screen.html';
import { Effect } from '^reactive/effect.ts';


export async function showSetupScreen() {
	xlog.info('App::showSetupScreen');

	const width = 640;
	const height = 480;

	const currentWindow = getCurrentWindow();
	await currentWindow.setSize(new PhysicalSize(width, height));
	await currentWindow.setPosition(new PhysicalPosition(
		(window.screen.width - width) / 2,
		(window.screen.height - height) / 2,
	));

	document.body.appendChild(SetupScreen().element);

	return new Promise<void>((resolve) => {
		let first = true;
		const effect = new Effect(() => {
			setupDoneSignal.value;
			if (first) { first = false; return; }
			effect.dispose();
			resolve();
		}, { strong: true });
	});
}
