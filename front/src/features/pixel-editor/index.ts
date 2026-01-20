/**
 * Pixel Editor Feature
 *
 * Tools for editing tile pixels: color picker, pencil, color replace
 */

// Types
export {
	type PixelTool,
	type ReplaceScope,
	type PixelChange,
	type PixelUndoData,
	type ColorReplaceParams,
	TILE_SIZE,
	TILE_PIXELS,
	pixelToIndex,
	indexToPixel,
	isValidPixel,
	getPixelColor,
	setPixelColor,
	findPixelsWithColor,
	countColors,
	getUsedColors,
} from './pixel-editor.types';

// State
export { PixelEditorState } from './pixel-editor.state';

// Actions
export {
	pickColorAt,
	drawPixel,
	drawBrush,
	drawLine,
	replaceColorInTile,
	replaceColorInTiles,
	replaceColorInAllTiles,
	replaceColor,
	floodFill,
	flushChanges,
	applyPixelUndo,
	applyPixelRedo,
} from './pixel-editor.actions';

// UI Components
export { PixelEditorPanel } from './ui/pixel-editor-panel.component';
export { PixelEditorToolbar } from './ui/pixel-editor-toolbar.component';
export { PixelEditorCanvas } from './ui/pixel-editor-canvas.component';
export { PixelEditorColorBar } from './ui/pixel-editor-colorbar.component';
