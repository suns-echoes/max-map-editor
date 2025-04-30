import { Button, Div, Section, Span, TextInput } from '^utils/reactive/html-node.elements.ts';
import { Signal } from '^utils/reactive/signal.class.ts';
import { openFolderDialog } from '^utils/dialogs/open-folder-dialog.ts';
import { SettingsFile } from '^storage/perma-storage/settings-file.ts';
import { RustAPI } from '^utils/rust-api.ts';
import style from './setup-screen.module.css';


export const setupDoneSignal = new Signal();


export function SetupScreen() {
	let maxPathInput, browseButton, doneButton, errorMessageBox, errorMessage;

	const SetupScreen = (
		Section('setupScreen').class(style.setupScreen).nodes([
			Div().class(style.caption).text('SETUP'),
			Div().class(style.maxPathSelector).nodes([
				Div().class(style.label).text('M.A.X. Directory Path:'),
				maxPathInput = TextInput().value(SettingsFile.get('max')?.path ?? ''),
				Div().class(style.splitApart).nodes([
					browseButton = Button().text('BROWSE'),
					doneButton = Button().text('DONE'),
				]),
				errorMessageBox = Div().class(style.messageBox).nodes([
					errorMessage = Span(),
				]),
			]),
		])
	);

	const showError = function (message: string) {
		errorMessage.html(message);
		errorMessageBox.classes(style.messageBox, style.show, style.error);
	};

	const showMessage = function (message: string) {
		errorMessage.html(message);
		errorMessageBox.classes(style.messageBox, style.show, style.info);
	};

	const browseForMaxPath = async function () {
		const path = await openFolderDialog();
		if (path) {
			showMessage('Press DONE to verify the path.');
			maxPathInput.value(path);
		}
	};

	/**
	 * @returns Error message if the path is invalid, null if valid.
	 */
	const verifyPath = async function (path: string): Promise<string | null> {
		showMessage('Validating provided path.');

		if (!path) return 'Please provie M.A.X. installation path.';

		try {
			if (!await RustAPI.validateMAXDir(path))
				return 'Invalid directory.<br>Please provide a valid M.A.X. installation path.';
		} catch (e) {
			if (typeof e === 'string' && e.startsWith('ERROR_') && e in RustAPI) {
				console.error(errorMessage);
				return RustAPI[e as keyof typeof RustAPI] as string;
			} else {
				console.error(e);
				return 'Unknown error occurred while validating the path.';
			}
		}

		return null;
	};

	const finishSetup = async function () {
		const path = maxPathInput.element.value.replaceAll('\\', '/').replace(/^\.\//, '');

		const error = await verifyPath(path);

		if (error) {
			showError(error);
			return;
		}

		try {
			showMessage('Updating setting file.');
			console.info('Updating settings file:', { max: { path }, setup: false });
			await SettingsFile.set({ max: { path }, setup: false }).sync();
		} catch (e) {
			showError('Could not sync settings file.<br>This is a critical error.');
			console.error('Could not sync settings file:', e);
		}
		if (!await RustAPI.reloadMAXPath()) {
			showError('Backend could not reload M.A.X. path.<br>This is a critical error.');
			console.error('Fatal: Tauri could not reload M.A.X. path.');
		}

		showMessage('Setup complete!');
		browseButton.removeAllEventListeners();
		doneButton.removeAllEventListeners();
		setupDoneSignal.dispatch();
	};

	showMessage('Please provie M.A.X. installation path.');

	browseButton.addEventListener('click', browseForMaxPath);
	doneButton.addEventListener('click', finishSetup);

	return SetupScreen;
}
