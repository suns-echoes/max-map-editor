/**
 * Pixel Editor Actions
 *
 * Actions for pixel-level tile editing with undo/redo support.
 */

import { xlog } from '^lib/xlog/xlog.ts';
import { AppState } from '^state/app-state.ts';
import { HistoryState } from '^state/history-state.ts';
import { PixelEditorState } from './pixel-editor.state.ts';
import {
	type PixelChange,
	type PixelUndoData,
	type ColorReplaceParams,
	pixelToIndex,
	isValidPixel,
	getPixelColor,
	findPixelsWithColor,
} from './pixel-editor.types.ts';


// ============================================================================
// Single Tile Operations
// ============================================================================

/**
 * Get tile by ID
 */
function getTile(tileId: string): Tile | null {
	const tiles = AppState.tiles.value;
	if (!tiles) return null;
	return tiles.get(tileId) ?? null;
}

/**
 * Pick color from pixel (eyedropper)
 */
export function pickColorAt(tileId: string, x: number, y: number): number | null {
	const tile = getTile(tileId);
	if (!tile || !isValidPixel(x, y)) return null;

	const colorIndex = getPixelColor(tile.data, x, y);
	PixelEditorState.selectColor(colorIndex);

	xlog.info('PixelEditor', `Picked color ${colorIndex} at (${x}, ${y})`);

	return colorIndex;
}

/**
 * Draw single pixel with pencil
 */
export function drawPixel(tileId: string, x: number, y: number): boolean {
	const tile = getTile(tileId);
	if (!tile || !isValidPixel(x, y)) return false;

	const colorIndex = PixelEditorState.selectedColor.value;
	const pixelIndex = pixelToIndex(x, y);
	const oldColor = tile.data[pixelIndex];

	if (oldColor === colorIndex) return false;

	// Apply change
	tile.data[pixelIndex] = colorIndex;

	// Record for undo
	recordChange([{
		tileId,
		pixelIndex,
		oldColor,
		newColor: colorIndex,
	}]);

	// Trigger update
	triggerTileUpdate();

	return true;
}

/**
 * Draw with brush (multiple pixels based on brush size)
 */
export function drawBrush(tileId: string, centerX: number, centerY: number): boolean {
	const tile = getTile(tileId);
	if (!tile) return false;

	const colorIndex = PixelEditorState.selectedColor.value;
	const brushSize = PixelEditorState.brushSize.value;
	const halfSize = Math.floor(brushSize / 2);

	const changes: PixelChange[] = [];

	for (let dy = -halfSize; dy < brushSize - halfSize; dy++) {
		for (let dx = -halfSize; dx < brushSize - halfSize; dx++) {
			const x = centerX + dx;
			const y = centerY + dy;

			if (!isValidPixel(x, y)) continue;

			const pixelIndex = pixelToIndex(x, y);
			const oldColor = tile.data[pixelIndex];

			if (oldColor !== colorIndex) {
				tile.data[pixelIndex] = colorIndex;
				changes.push({
					tileId,
					pixelIndex,
					oldColor,
					newColor: colorIndex,
				});
			}
		}
	}

	if (changes.length === 0) return false;

	recordChange(changes);
	triggerTileUpdate();

	return true;
}

/**
 * Draw line between two points (for smooth drawing)
 */
export function drawLine(
	tileId: string,
	x0: number, y0: number,
	x1: number, y1: number
): boolean {
	const tile = getTile(tileId);
	if (!tile) return false;

	const colorIndex = PixelEditorState.selectedColor.value;
	const changes: PixelChange[] = [];

	// Bresenham's line algorithm
	const dx = Math.abs(x1 - x0);
	const dy = Math.abs(y1 - y0);
	const sx = x0 < x1 ? 1 : -1;
	const sy = y0 < y1 ? 1 : -1;
	let err = dx - dy;

	let x = x0;
	let y = y0;

	while (true) {
		if (isValidPixel(x, y)) {
			const pixelIndex = pixelToIndex(x, y);
			const oldColor = tile.data[pixelIndex];

			if (oldColor !== colorIndex) {
				tile.data[pixelIndex] = colorIndex;
				changes.push({
					tileId,
					pixelIndex,
					oldColor,
					newColor: colorIndex,
				});
			}
		}

		if (x === x1 && y === y1) break;

		const e2 = 2 * err;
		if (e2 > -dy) {
			err -= dy;
			x += sx;
		}
		if (e2 < dx) {
			err += dx;
			y += sy;
		}
	}

	if (changes.length === 0) return false;

	recordChange(changes);
	triggerTileUpdate();

	return true;
}


// ============================================================================
// Color Replace Operations
// ============================================================================

/**
 * Replace color in a single tile
 */
export function replaceColorInTile(
	tileId: string,
	fromColor: number,
	toColor: number
): number {
	const tile = getTile(tileId);
	if (!tile || fromColor === toColor) return 0;

	const pixelIndices = findPixelsWithColor(tile.data, fromColor);
	if (pixelIndices.length === 0) return 0;

	const changes: PixelChange[] = [];

	for (const pixelIndex of pixelIndices) {
		tile.data[pixelIndex] = toColor;
		changes.push({
			tileId,
			pixelIndex,
			oldColor: fromColor,
			newColor: toColor,
		});
	}

	recordChange(changes);
	triggerTileUpdate();

	xlog.info('PixelEditor', `Replaced ${changes.length} pixels in tile ${tileId}`);

	return changes.length;
}

/**
 * Replace color across multiple tiles
 */
export function replaceColorInTiles(
	tileIds: string[],
	fromColor: number,
	toColor: number
): number {
	const tiles = AppState.tiles.value;
	if (!tiles || fromColor === toColor) return 0;

	const allChanges: PixelChange[] = [];

	for (const tileId of tileIds) {
		const tile = tiles.get(tileId);
		if (!tile) continue;

		const pixelIndices = findPixelsWithColor(tile.data, fromColor);

		for (const pixelIndex of pixelIndices) {
			tile.data[pixelIndex] = toColor;
			allChanges.push({
				tileId,
				pixelIndex,
				oldColor: fromColor,
				newColor: toColor,
			});
		}
	}

	if (allChanges.length === 0) return 0;

	recordChange(allChanges);
	triggerTileUpdate();

	xlog.info('PixelEditor', `Replaced ${allChanges.length} pixels across ${tileIds.length} tiles`);

	return allChanges.length;
}

/**
 * Replace color in all tiles
 */
export function replaceColorInAllTiles(fromColor: number, toColor: number): number {
	const tiles = AppState.tiles.value;
	if (!tiles) return 0;

	const allTileIds = [...tiles.keys()];
	return replaceColorInTiles(allTileIds, fromColor, toColor);
}

/**
 * Replace color based on current state settings
 */
export function replaceColor(params?: Partial<ColorReplaceParams>): number {
	const fromColor = params?.fromColor ?? PixelEditorState.secondaryColor.value;
	const toColor = params?.toColor ?? PixelEditorState.selectedColor.value;
	const scope = params?.scope ?? PixelEditorState.replaceScope.value;
	const tileIds = params?.tileIds;

	if (fromColor === null || fromColor === toColor) return 0;

	switch (scope) {
		case 'tile': {
			const editingTileId = PixelEditorState.editingTileId.value;
			if (!editingTileId) return 0;
			return replaceColorInTile(editingTileId, fromColor, toColor);
		}

		case 'selected': {
			const selectedIds = tileIds ?? [...PixelEditorState.selectedTileIds.value];
			if (selectedIds.length === 0) return 0;
			return replaceColorInTiles(selectedIds, fromColor, toColor);
		}

		case 'all':
			return replaceColorInAllTiles(fromColor, toColor);

		default:
			return 0;
	}
}


// ============================================================================
// Flood Fill
// ============================================================================

/**
 * Flood fill from a starting point
 */
export function floodFill(tileId: string, startX: number, startY: number): boolean {
	const tile = getTile(tileId);
	if (!tile || !isValidPixel(startX, startY)) return false;

	const targetColor = getPixelColor(tile.data, startX, startY);
	const fillColor = PixelEditorState.selectedColor.value;

	if (targetColor === fillColor) return false;

	const changes: PixelChange[] = [];
	const visited = new Set<number>();
	const stack: Array<{ x: number; y: number }> = [{ x: startX, y: startY }];

	while (stack.length > 0) {
		const { x, y } = stack.pop()!;
		const pixelIndex = pixelToIndex(x, y);

		if (visited.has(pixelIndex)) continue;
		if (!isValidPixel(x, y)) continue;
		if (tile.data[pixelIndex] !== targetColor) continue;

		visited.add(pixelIndex);
		tile.data[pixelIndex] = fillColor;
		changes.push({
			tileId,
			pixelIndex,
			oldColor: targetColor,
			newColor: fillColor,
		});

		// Add neighbors
		stack.push({ x: x + 1, y });
		stack.push({ x: x - 1, y });
		stack.push({ x, y: y + 1 });
		stack.push({ x, y: y - 1 });
	}

	if (changes.length === 0) return false;

	recordChange(changes);
	triggerTileUpdate();

	xlog.info('PixelEditor', `Flood filled ${changes.length} pixels`);

	return true;
}


// ============================================================================
// History Integration
// ============================================================================

/** Pending changes for batching */
let pendingChanges: PixelChange[] = [];
let batchTimeout: ReturnType<typeof setTimeout> | null = null;

/**
 * Record pixel changes (with batching for smooth drawing)
 */
function recordChange(changes: PixelChange[]) {
	pendingChanges.push(...changes);

	// Debounce to batch rapid changes
	if (batchTimeout) {
		clearTimeout(batchTimeout);
	}

	batchTimeout = setTimeout(() => {
		if (pendingChanges.length > 0) {
			commitChanges();
		}
	}, 100);
}

/**
 * Commit pending changes to history
 */
function commitChanges() {
	if (pendingChanges.length === 0) return;

	const undoData: PixelUndoData = { changes: [...pendingChanges] };

	const description = pendingChanges.length === 1
		? 'Draw pixel'
		: `Draw ${pendingChanges.length} pixels`;

	HistoryState.push('pixel', undoData, undoData, description);

	pendingChanges = [];
	batchTimeout = null;
}

/**
 * Force commit any pending changes
 */
export function flushChanges() {
	if (batchTimeout) {
		clearTimeout(batchTimeout);
		batchTimeout = null;
	}
	commitChanges();
}

/**
 * Apply undo for pixel changes
 */
export function applyPixelUndo(data: PixelUndoData) {
	const tiles = AppState.tiles.value;
	if (!tiles) return;

	for (const change of data.changes) {
		const tile = tiles.get(change.tileId);
		if (tile) {
			tile.data[change.pixelIndex] = change.oldColor;
		}
	}

	triggerTileUpdate();
}

/**
 * Apply redo for pixel changes
 */
export function applyPixelRedo(data: PixelUndoData) {
	const tiles = AppState.tiles.value;
	if (!tiles) return;

	for (const change of data.changes) {
		const tile = tiles.get(change.tileId);
		if (tile) {
			tile.data[change.pixelIndex] = change.newColor;
		}
	}

	triggerTileUpdate();
}

/**
 * Trigger reactive update for tiles
 */
function triggerTileUpdate() {
	const tiles = AppState.tiles.value;
	if (tiles) {
		AppState.tiles.set(new Map(tiles));
	}
}
