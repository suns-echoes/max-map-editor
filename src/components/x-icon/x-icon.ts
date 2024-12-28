import { updateMaxPath } from '../../actions/config/update-max-path.ts';
import { openFolderDialog } from '../../utils/dialogs/open-folder-dialog.ts';
import template from './x-icon.html';


export class XIcon extends HTMLElement {
	constructor() {
		super();
		const shadow = this.attachShadow({ mode: 'open' });
        shadow.appendChild(template.content.cloneNode(true));

		this.shadowRoot!.querySelector('img')!.addEventListener('click', async function () {
			const path = await openFolderDialog();
			if (typeof path === 'string') {
				updateMaxPath(path + '/MAX.RES');
			}
		});

		// this.shadowRoot!.querySelector('img')!.addEventListener('dblclick', async function () {
		// 	const path = await open();
		// 	if (typeof path === 'string') {
		// 		updateMaxPath(path);
		// 	}
		// });
	}
};

customElements.define('x-icon', XIcon);
