import { SimpleButton } from '^src/ui/components/buttons/simple-button.component';
import { printDebugInfo } from '^utils/debug/debug.ts';
import { Section } from '^utils/reactive/html-node.elements.ts';
import { VerticalSeparator } from '^src/ui/components/separators/vertical-separator.component';

import style from './main-toolbar.module.css';


export function MainToolbar() {
	printDebugInfo('UI::MainToolbar');

	return (
		Section('main-toolbar').class(style.mainToolbar).nodes([
			SimpleButton().text('Select'),
			SimpleButton().text('Copy'),
			SimpleButton().text('Paste'),
			VerticalSeparator(),
			SimpleButton().text('Ground'),
			SimpleButton().text('Water'),
			VerticalSeparator(),
			SimpleButton().text('Brush'),
			SimpleButton().text('Rect'),
			SimpleButton().text('Ellipse'),
			SimpleButton().text('Fill'),
			VerticalSeparator(),
			SimpleButton().text('Auto fix shore'),
		])
	);
}
