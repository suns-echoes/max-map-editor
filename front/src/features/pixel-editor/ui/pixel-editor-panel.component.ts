/**
 * Pixel Editor Panel Component
 *
 * Main container for the pixel editor interface.
 */

import { Section, Div } from '^reactive/reactive-node.elements.ts';
import { Effect } from '^reactive/effect.ts';
import { xlog } from '^lib/xlog/xlog.ts';
import { PixelEditorState } from '../pixel-editor.state.ts';
import { PixelEditorToolbar } from './pixel-editor-toolbar.component.ts';
import { PixelEditorCanvas } from './pixel-editor-canvas.component.ts';
import { PixelEditorColorBar } from './pixel-editor-colorbar.component.ts';

import style from './pixel-editor-panel.module.css';


/**
 * Pixel Editor Panel - main container for pixel editing tools.
 */
export function PixelEditorPanel() {
	xlog.info('UI::PixelEditorPanel');

	const panel = Section('pixel-editor-panel').class(style.pixelEditorPanel);

	// Effect: Show/hide panel based on mode
	new Effect(function updatePanelVisibility() {
		if (PixelEditorState.isActive.value) {
			panel.element.classList.add(style.visible);
		} else {
			panel.element.classList.remove(style.visible);
		}
	}, { strong: true }).on([PixelEditorState.isActive]);

	const tileIdDisplay = Div().class(style.tileIdDisplay);

	// Effect: Update tile ID display
	new Effect(function updateTileIdDisplay() {
		const tileId = PixelEditorState.editingTileId.value;
		if (tileId) {
			tileIdDisplay.text(`Editing: ${tileId}`);
		} else {
			tileIdDisplay.text('Click a tile in the palette to edit');
		}
	}, { strong: true }).on([PixelEditorState.editingTileId]);

	return panel.nodes([
		Div().class(style.header).nodes([
			Div().class(style.title).text('Pixel Editor'),
			tileIdDisplay,
		]),
		PixelEditorToolbar(),
		PixelEditorCanvas(),
		PixelEditorColorBar(),
	]);
}
