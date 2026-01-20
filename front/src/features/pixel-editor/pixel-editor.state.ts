/**
 * Pixel Editor State
 *
 * Reactive state for pixel-level tile editing.
 */

import { Value } from '^reactive/value.ts';
import { Expr } from '^reactive/expr.ts';
import { EditorState } from '^state/editor-state.ts';
import { type PixelTool, type ReplaceScope } from './pixel-editor.types.ts';


// ============================================================================
// State Values
// ============================================================================

/** Whether pixel editor mode is active */
const isActive = new Expr(() => EditorState.mode.value === 'pixel');

/** Current pixel editing tool */
const tool = new Value<PixelTool>('pencil');

/** Primary selected color (palette index) for drawing */
const selectedColor = new Value<number>(0);

/** Secondary color (palette index) for swap/replace operations */
const secondaryColor = new Value<number | null>(null);

/** Brush size in pixels (1 = single pixel) */
const brushSize = new Value<number>(1);

/** Scope for color replace operations */
const replaceScope = new Value<ReplaceScope>('tile');

/** Currently editing tile ID (for focused editing) */
const editingTileId = new Value<string | null>(null);

/** Selected tile IDs for batch operations */
const selectedTileIds = new Value<Set<string>>(new Set());

/** Zoom level for pixel editor canvas */
const zoom = new Value<number>(4);

/** Show grid overlay */
const showGrid = new Value<boolean>(true);


// ============================================================================
// Derived State
// ============================================================================

/** True when actively editing a tile */
const isEditingTile = new Expr(() => editingTileId.value !== null);

/** True when can use pencil/picker tools */
const canDraw = new Expr(() =>
	isActive.value &&
	editingTileId.value !== null &&
	(tool.value === 'pencil' || tool.value === 'picker')
);

/** True when replace tool is usable */
const canReplace = new Expr(() =>
	isActive.value &&
	secondaryColor.value !== null &&
	selectedColor.value !== secondaryColor.value
);

/** Has tile selection for batch operations */
const hasSelection = new Expr(() => selectedTileIds.value.size > 0);


// ============================================================================
// Actions
// ============================================================================

/**
 * Enter pixel editor mode
 */
function enterMode() {
	EditorState.mode.set('pixel');
}

/**
 * Exit pixel editor mode
 */
function exitMode() {
	EditorState.mode.set('ground');
	editingTileId.set(null);
}

/**
 * Set active tool
 */
function setTool(newTool: PixelTool) {
	tool.set(newTool);
}

/**
 * Select primary color
 */
function selectColor(colorIndex: number) {
	selectedColor.set(Math.max(0, Math.min(255, colorIndex)));
}

/**
 * Select secondary color
 */
function selectSecondaryColor(colorIndex: number | null) {
	if (colorIndex === null) {
		secondaryColor.set(null);
	} else {
		secondaryColor.set(Math.max(0, Math.min(255, colorIndex)));
	}
}

/**
 * Swap primary and secondary colors
 */
function swapColors() {
	const primary = selectedColor.value;
	const secondary = secondaryColor.value;
	if (secondary !== null) {
		selectedColor.set(secondary);
		secondaryColor.set(primary);
	}
}

/**
 * Set brush size
 */
function setBrushSize(size: number) {
	brushSize.set(Math.max(1, Math.min(8, size)));
}

/**
 * Set replace scope
 */
function setReplaceScope(scope: ReplaceScope) {
	replaceScope.set(scope);
}

/**
 * Start editing a specific tile
 */
function startEditingTile(tileId: string) {
	editingTileId.set(tileId);
}

/**
 * Stop editing current tile
 */
function stopEditingTile() {
	editingTileId.set(null);
}

/**
 * Select tile for batch operations
 */
function selectTile(tileId: string) {
	const newSet = new Set(selectedTileIds.value);
	newSet.add(tileId);
	selectedTileIds.set(newSet);
}

/**
 * Deselect tile
 */
function deselectTile(tileId: string) {
	const newSet = new Set(selectedTileIds.value);
	newSet.delete(tileId);
	selectedTileIds.set(newSet);
}

/**
 * Toggle tile selection
 */
function toggleTileSelection(tileId: string) {
	if (selectedTileIds.value.has(tileId)) {
		deselectTile(tileId);
	} else {
		selectTile(tileId);
	}
}

/**
 * Clear tile selection
 */
function clearTileSelection() {
	selectedTileIds.set(new Set());
}

/**
 * Set zoom level
 */
function setZoom(level: number) {
	zoom.set(Math.max(1, Math.min(16, level)));
}

/**
 * Toggle grid overlay
 */
function toggleGrid() {
	showGrid.set(!showGrid.value);
}

/**
 * Reset state to defaults
 */
function reset() {
	tool.set('pencil');
	selectedColor.set(0);
	secondaryColor.set(null);
	brushSize.set(1);
	replaceScope.set('tile');
	editingTileId.set(null);
	selectedTileIds.set(new Set());
	zoom.set(4);
	showGrid.set(true);
}


// ============================================================================
// Export
// ============================================================================

export const PixelEditorState = {
	// Derived
	isActive,
	isEditingTile,
	canDraw,
	canReplace,
	hasSelection,

	// Values
	tool,
	selectedColor,
	secondaryColor,
	brushSize,
	replaceScope,
	editingTileId,
	selectedTileIds,
	zoom,
	showGrid,

	// Actions
	enterMode,
	exitMode,
	setTool,
	selectColor,
	selectSecondaryColor,
	swapColors,
	setBrushSize,
	setReplaceScope,
	startEditingTile,
	stopEditingTile,
	selectTile,
	deselectTile,
	toggleTileSelection,
	clearTileSelection,
	setZoom,
	toggleGrid,
	reset,
};
