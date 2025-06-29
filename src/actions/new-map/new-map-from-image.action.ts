import { printDebugInfo } from '^lib/debug/debug.ts';
import { openFileDialog } from '^lib/dialogs/open-file-dialog.ts';
import { RustAPI } from '^src/bff/rust-api';


export function NewMapFromImageAction(): LockPromise {
	printDebugInfo('Actions::NewMapFromImageAction');

	return new Promise<void>(async function (resolve) {

		try {
			const filePath = await openFileDialog({
				title: 'Open Image File',
				filters: [
					{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'gif', 'bmp', 'tiff'] },
					{ name: 'All Files', extensions: ['*'] },
				],
			});

			if (filePath) {
				printDebugInfo(`Actions::NewMapFromImageAction::File selected: ${filePath}`);

				const [palette, indexedImage] = await RustAPI.imageToWRL(filePath);
				printDebugInfo(`Actions::NewMapFromImageAction::Palette size: ${palette.length}, Indexed Image size: ${indexedImage.length}`);
			}
		} catch (error) {
			printDebugInfo(`Actions::NewMapFromImageAction::Error opening file dialog: ${error}`);
		}

		resolve();
	});
}
