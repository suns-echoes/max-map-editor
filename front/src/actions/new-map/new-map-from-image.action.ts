import { showErrorModalAction } from '^src/ui/actions/show-error-modal/show-error-modal.action';
import { xlog } from '^lib/xlog/xlog.ts';
import { openFileDialog } from '^lib/dialogs/open-file-dialog.ts';
import { RustAPI } from '^src/bff/rust-api';


export function NewMapFromImageAction(): LockPromise {
	xlog.info('Actions::NewMapFromImageAction');

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
				xlog.info('Actions::NewMapFromImageAction::File selected:', filePath);

				const [palette, indexedImage] = await RustAPI.imageToWRL(filePath);
				xlog.info('Actions::NewMapFromImageAction::Palette size:', String(palette.length), 'Indexed Image size:', String(indexedImage.length));
			}
		} catch (error) {
			xlog.error('Actions::NewMapFromImageAction::Error opening file dialog:', String(error));

			await showErrorModalAction({
				title: `ERROR 0x${Math.floor(Math.random() * 0x10000).toString(16).padStart(4, '0')}`,
				message: `
					<p># N3w_Maq ?rOm I#@gE ../</p>
					<p>It seems the image data caused a spatial anomaly, leading to some... unforeseen consequences.</p>
					<p>${error}</p>
				`.trim(),
				onClose: () => {
					xlog.info('Error popup closed');
				},
			});
		} finally {
			resolve();
		}
	});
}
