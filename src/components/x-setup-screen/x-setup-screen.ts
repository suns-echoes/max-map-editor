import template from './x-setup-screen.html';
import globalStyle from '../../styles/global.style';
import style from './x-setup-screen.style';

import { SettingsFile } from '^storage/perma-storage/settings-file.ts';
import { openFolderDialog } from '^utils/dialogs/open-folder-dialog.ts';
import { RustAPI } from '^utils/rust-api.ts';


template.content.appendChild(globalStyle);
template.content.appendChild(style);

export class XSetupScreen extends HTMLElement {
	constructor() {
		super();
		const shadowRoot = this.attachShadow({ mode: 'open' });
        shadowRoot.appendChild(template.content.cloneNode(true));
	}

	connectedCallback() {
		(async () => {
			const shadowRoot = this.shadowRoot!;
			const maxPathInput = shadowRoot.getElementById<HTMLInputElement>('setup--max-path');
			const browseButton = shadowRoot.getElementById<HTMLButtonElement>('setup--browse-for-max-path');
			const errorMessage = shadowRoot.getElementById<HTMLDivElement>('setup--error-message');
			const doneButton = shadowRoot.getElementById<HTMLButtonElement>('setup--done');

			browseButton.addEventListener('click', async function () {
				const path = await openFolderDialog();

				if (path) {
					maxPathInput.classList.remove('invalid');
					errorMessage.classList.remove('show');
					maxPathInput.value = path;
				}
			});

			doneButton.addEventListener('click', async () => {
				const path = maxPathInput.value;

				if (await RustAPI.checkMAXDir(path)) {
					maxPathInput.classList.remove('invalid');
					errorMessage.classList.remove('show');
					await SettingsFile.set({ max: { path }, setup: false });
					await SettingsFile.sync();
					if (!await RustAPI.updateMAXPath()) {
						console.log('Could not update MAX directory:', path);
						errorMessage.classList.add('show');
					} else {
						if (this.ondone) {
							this.ondone();
						}
					}
				} else {
					console.log('Invalid MAX directory:', path);
					maxPathInput.classList.add('invalid');
					errorMessage.classList.add('show');
				}
			});

			maxPathInput.value = SettingsFile.get('max').path ?? '';
		})();
	}

	ondone: (() => void) | null = null;
}

customElements.define('x-setup-screen', XSetupScreen);
