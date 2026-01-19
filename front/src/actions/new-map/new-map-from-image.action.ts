import { showErrorModalAction } from '^src/ui/actions/show-error-modal/show-error-modal.action';
import { xlog } from '^lib/xlog/xlog.ts';
import { openFileDialog } from '^lib/dialogs/open-file-dialog.ts';
import { RustAPI } from '^src/bff/rust-api';
import { tryCatchAsync } from '^lib/errors/errors.ts';


export function NewMapFromImageAction(): LockPromise {
	xlog.info('Actions::NewMapFromImageAction');

	return new Promise<void>(async function (resolve) {
		const [dialogError, filePath] = await tryCatchAsync(openFileDialog({
			title: 'Open Image File',
			filters: [
				{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'gif', 'bmp', 'tiff'] },
				{ name: 'All Files', extensions: ['*'] },
			],
		}));

		if (dialogError) {
			xlog.error('Actions::NewMapFromImageAction::Error opening file dialog:', dialogError.message);
			await showErrorModalAction({
				title: `ERROR 0x${Math.floor(Math.random() * 0x10000).toString(16).padStart(4, '0')}`,
				message: `
					<p># N3w_Maq ?rOm I#@gE ../</p>
					<p>Failed to open file dialog.</p>
					<p>${dialogError.message}</p>
				`.trim(),
				onClose: () => xlog.info('Error popup closed'),
			});
			resolve();
			return;
		}

		if (!filePath) {
			resolve();
			return;
		}

		xlog.info('Actions::NewMapFromImageAction::File selected:', filePath);

		const [convertError, result] = await tryCatchAsync(RustAPI.imageToWRL(filePath));

		if (convertError) {
			xlog.error('Actions::NewMapFromImageAction::Error converting image:', convertError.message);
			await showErrorModalAction({
				title: `ERROR 0x${Math.floor(Math.random() * 0x10000).toString(16).padStart(4, '0')}`,
				message: `
					<p># N3w_Maq ?rOm I#@gE ../</p>
					<p>It seems the image data caused a spatial anomaly, leading to some... unforeseen consequences.</p>
					<p>${convertError.message}</p>
				`.trim(),
				onClose: () => xlog.info('Error popup closed'),
			});
			resolve();
			return;
		}

		const [palette, indexedImage] = result;
		xlog.info('Actions::NewMapFromImageAction::Palette size:', String(palette.length), 'Indexed Image size:', String(indexedImage.length));

		resolve();
	});
}
