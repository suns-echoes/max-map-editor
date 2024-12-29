import template from './x-main-window.html';
import globalStyle from '../../styles/global.style';
import style from './x-main-window.style';

import './components/x-wgl-map/x-wgl-map.ts';


template.content.appendChild(globalStyle);
template.content.appendChild(style);

export class XMainWindow extends HTMLElement {
	constructor() {
		super();
		const shadowRoot = this.attachShadow({ mode: 'open' });
        shadowRoot.appendChild(template.content.cloneNode(true));
	}

	connectedCallback() {
		(async () => {
			// const shadowRoot = this.shadowRoot!;

		})();
	}
}

customElements.define('x-main-window', XMainWindow);
