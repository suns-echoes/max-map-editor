import template from './x-setup-screen.html';
import style from './x-setup-screen.style';

import { SettingsFile } from '^storage/perma-storage/settings-file.ts';
import { openFolderDialog } from '^utils/dialogs/open-folder-dialog.ts';
import { RustAPI } from '^utils/rust-api.ts';
import { printDebugInfo } from '^utils/debug/debug.ts';


template.content.appendChild(style);


export class XSetupScreen extends HTMLElement {
	constructor() {
		printDebugInfo('XSetupScreen::constructor');
		super();
		const shadowRoot = this.attachShadow({ mode: 'open' });
        shadowRoot.appendChild(template.content.cloneNode(true));
	}

	connectedCallback() {
		printDebugInfo('XSetupScreen::connectedCallback');
		(async () => {
			const shadowRoot = this.shadowRoot!;
			const maxPathInput = shadowRoot.getElementById<HTMLInputElement>('setup--max-path');
			const browseButton = shadowRoot.getElementById<HTMLButtonElement>('setup--browse-for-max-path');
			const errorMessageBox = shadowRoot.getElementById<HTMLDivElement>('setup--message-box');
			const errorMessage = errorMessageBox.firstElementChild!;
			const doneButton = shadowRoot.getElementById<HTMLButtonElement>('setup--done');


			function showError(message: string) {
				errorMessage.innerHTML = message;
				errorMessageBox.classList.remove('info');
				errorMessageBox.classList.add('error');
				errorMessageBox.classList.add('show');
			}

			function showMessage(message: string) {
				errorMessage.innerHTML = message;
				errorMessageBox.classList.remove('error');
				errorMessageBox.classList.add('info');
				errorMessageBox.classList.add('show');
			}

			function hideMessageBox() {
				errorMessageBox.classList.remove('show');
			}


			showMessage('Please provie M.A.X. installation path.');


			browseButton.addEventListener('click', async function () {
				const path = await openFolderDialog();

				if (path) {
					showMessage('Press DONE to verify the path.');
					maxPathInput.value = path;
				}
			});


			doneButton.addEventListener('click', async () => {
				const path = maxPathInput.value.replaceAll('\\', '/').replace(/^\.\//, '');

				if (!path) {
					showMessage('Please provie M.A.X. installation path.');
					return;
				}

				try {
					showMessage('Validating provided path.');
					const valid = await RustAPI.validateMAXDir(path);

					if (valid) {
						showMessage('Updating setting file.');
						console.info('Updating settings file:', { max: { path }, setup: false });
						SettingsFile.set({ max: { path }, setup: false });
						try {
							await SettingsFile.sync();
						} catch (e) {
							showError('Could not sync settings file.<br>This is a critical error.');
							console.error('Could not sync settings file:', e);
						}
						if (!await RustAPI.reloadMAXPath()) {
							showError('Backend could not reload M.A.X. path.<br>This is a critical error.');
							console.error('Tauri could not reload M.A.X. path.');
						}
						hideMessageBox();
						this.ondone?.();
					} else {
						showError('Invalid directory.<br>Please provide a valid M.A.X. installation path.');
					}

				} catch (e) {
					if (typeof e === 'string' && e.startsWith('ERROR_') && e in RustAPI) {
						showError(RustAPI[e as keyof typeof RustAPI] as string);
						console.error(RustAPI[e as keyof typeof RustAPI] as string);
					} else {
						console.error('Unknown error:', e);
					}
				}
			});

			maxPathInput.value = SettingsFile.get('max').path ?? '';
		})();
	}

	ondone: (() => void) | null = null;
}


printDebugInfo('registering "x-setup-screen" web component');
customElements.define('x-setup-screen', XSetupScreen);
