import { SimpleButton } from '^src/ui/components/buttons/simple-button.component';
import { xlog } from '^lib/xlog/xlog.ts';
import { Section } from '^reactive/reactive-node.elements.ts';
import { VerticalSeparator } from '^src/ui/components/separators/vertical-separator.component';
import { applyAutoShore } from '^src/features/auto-shore/auto-shore.actions.ts';
import { PassTableState } from '^src/features/pass-table-editor/index.ts';
import { PixelEditorState } from '^src/features/pixel-editor/index.ts';
import { EditorState } from '^state/editor-state.ts';
import { Effect } from '^reactive/effect.ts';

import style from './main-toolbar.module.css';


export function MainToolbar() {
	xlog.info('UI::MainToolbar');

	const autoShoreButton = SimpleButton().text('Auto fix shore');
	autoShoreButton.on('click', () => {
		const changedCount = applyAutoShore();
		xlog.info(`Auto-shore: ${changedCount} tiles changed`);
	});

	// Pass Table Editor mode button
	const passTableButton = SimpleButton().text('Pass Table');
	passTableButton.on('click', () => {
		if (EditorState.mode.value === 'passTable') {
			PassTableState.exitMode();
		} else {
			PassTableState.enterMode();
		}
	});

	// Update button state when mode changes
	new Effect(function updatePassTableButton() {
		if (EditorState.mode.value === 'passTable') {
			passTableButton.element.classList.add(style.active);
		} else {
			passTableButton.element.classList.remove(style.active);
		}
	}, { strong: true }).on([EditorState.mode]);

	// Pixel Editor mode button
	const pixelEditorButton = SimpleButton().text('Pixel Editor');
	pixelEditorButton.on('click', () => {
		if (EditorState.mode.value === 'pixel') {
			PixelEditorState.exitMode();
		} else {
			PixelEditorState.enterMode();
		}
	});

	// Update button state when mode changes
	new Effect(function updatePixelEditorButton() {
		if (EditorState.mode.value === 'pixel') {
			pixelEditorButton.element.classList.add(style.active);
		} else {
			pixelEditorButton.element.classList.remove(style.active);
		}
	}, { strong: true }).on([EditorState.mode]);

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
			autoShoreButton,
			passTableButton,
			pixelEditorButton,
		])
	);
}
