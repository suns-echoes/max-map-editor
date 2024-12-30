import template from './x-main-window.html';
import globalStyle from '../../styles/global.style';
import style from './x-main-window.style';

import './components/x-wgl-map/x-wgl-map.ts';
import { printDebugInfo } from '^utils/debug/debug.ts';


template.content.appendChild(globalStyle);
template.content.appendChild(style);

export class XMainWindow extends HTMLElement {
	constructor() {
		printDebugInfo('XMainWindow::constructor');
		super();
		const shadowRoot = this.attachShadow({ mode: 'open' });
        shadowRoot.appendChild(template.content.cloneNode(true));
	}

	connectedCallback() {
		printDebugInfo('XMainWindow::connectedCallback');
		(async () => {
			// const shadowRoot = this.shadowRoot!;

		})();
	}
}

printDebugInfo('registering "x-main-window" web component');
customElements.define('x-main-window', XMainWindow);
