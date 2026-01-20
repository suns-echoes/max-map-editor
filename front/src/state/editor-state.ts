import { Value } from '^reactive/value.ts';
import { Expr } from '^reactive/expr.ts';


// ============================================================================
// Editor Types
// ============================================================================

/** Available editor tools */
export type EditorTool = 'select' | 'brush' | 'rect' | 'ellipse' | 'fill' | 'eyedropper';

/** Editor modes - determines what layer/data is being edited */
export type EditorMode = 'ground' | 'water' | 'passTable' | 'pixel';

/** Rectangle selection on the map (tile coordinates) */
export type MapSelection = {
	x: number;
	y: number;
	width: number;
	height: number;
};


// ============================================================================
// Editor State - Reactive Values
// ============================================================================

/** Current editor mode - determines what layer is being edited */
const mode = new Value<EditorMode>('ground');

/** Currently active tool */
const tool = new Value<EditorTool>('brush');

/** Currently selected tile ID for painting */
const selectedTile = new Value<string | null>(null);

/** Brush size (1 = single tile, 2 = 2x2, etc.) */
const brushSize = new Value(1);

/** Current rectangular selection on the map */
const selection = new Value<MapSelection | null>(null);

/** Recently used tiles for quick access */
const recentTiles = new Value<string[]>([]);


// ============================================================================
// Derived State
// ============================================================================

/** True when user can paint (has tool and tile selected) */
const canPaint = new Expr(() =>
	tool.value !== 'select' &&
	tool.value !== 'eyedropper' &&
	selectedTile.value !== null
);

/** True when in a painting tool mode */
const isPaintingTool = new Expr(() =>
	tool.value === 'brush' ||
	tool.value === 'rect' ||
	tool.value === 'ellipse' ||
	tool.value === 'fill'
);


// ============================================================================
// Actions
// ============================================================================

/** Select a tile and add it to recent tiles */
function selectTile(tileId: string) {
	selectedTile.set(tileId);

	// Add to recent tiles (max 10, no duplicates)
	const recent = recentTiles.value.filter((id: string) => id !== tileId);
	recent.unshift(tileId);
	if (recent.length > 10) recent.pop();
	recentTiles.set(recent);
}

/** Clear current selection */
function clearSelection() {
	selection.set(null);
}

/** Set selection rectangle */
function setSelection(x: number, y: number, width: number, height: number) {
	selection.set({ x, y, width, height });
}

/** Reset editor to default state */
function reset() {
	mode.set('ground');
	tool.set('brush');
	selectedTile.set(null);
	brushSize.set(1);
	selection.set(null);
	// Note: recentTiles preserved across resets
}


// ============================================================================
// Export
// ============================================================================

export const EditorState = {
	// Values
	mode,
	tool,
	selectedTile,
	brushSize,
	selection,
	recentTiles,

	// Derived
	canPaint,
	isPaintingTool,

	// Actions
	selectTile,
	clearSelection,
	setSelection,
	reset,
};
