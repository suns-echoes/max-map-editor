/**
 * Palette Editor Feature
 *
 * Edit palette colors with visual tools.
 */

// Types
export {
	rgbToHex,
	hexToRgb,
	getPaletteColor,
	setPaletteColor,
	colorsEqual,
	colorDistance,
	blendColors,
	adjustBrightness,
} from './palette-editor.types.ts';
export type {
	RgbColor,
	PaletteChange,
	PaletteUndoData,
	PaletteColorInfo,
} from './palette-editor.types.ts';

// State
export { PaletteEditorState } from './palette-editor.state.ts';

// Actions
export {
	getColorAt,
	getPaletteWithUsage,
	findUnusedColors,
	findSimilarColors,
	setColorAt,
	setColorsAt,
	swapColors,
	createGradient,
	shiftColors,
	copySelectedColor,
	pasteColor,
	applyPaletteUndo,
	applyPaletteRedo,
} from './palette-editor.actions.ts';

// UI
export { PalettePanel } from './ui/palette-panel.component.ts';
