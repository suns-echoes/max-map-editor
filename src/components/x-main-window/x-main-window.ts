import template from './x-main-window.html';
import style from './x-main-window.style';

import './components/x-wgl-map/x-wgl-map.ts';
import { printDebugInfo } from '^utils/debug/debug.ts';
import { Effect } from '^utils/reactive/effect.class.ts';
import { AppEvents } from '^events/app-events.ts';
import { saveMainWindowParams } from '^actions/main-window/save-main-window-params.ts';


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

			new Effect(function () {
				saveMainWindowParams();
			}).watch([AppEvents.windowSize]);
		})();
	}
}


printDebugInfo('registering "x-main-window" web component');
customElements.define('x-main-window', XMainWindow);
