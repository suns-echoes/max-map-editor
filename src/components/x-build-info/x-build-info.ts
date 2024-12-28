import template from './x-build-info.html';
import style from './x-build-info.style';

template.content.appendChild(style);

export class XBuildInfo extends HTMLElement {
	constructor() {
		super();
		const shadowRoot = this.attachShadow({ mode: 'open' });
        shadowRoot.appendChild(template.content.cloneNode(true));
	}
}

customElements.define('x-build-info', XBuildInfo);
