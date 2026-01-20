import { AppState } from '^state/app-state.ts';
import { EditorState } from '^state/editor-state.ts';
import { HistoryState } from '^state/history-state.ts';


// ============================================================================
// Types
// ============================================================================

/** A single tile change for undo/redo */
export type TileChange = {
	x: number;
	y: number;
	oldTileIndex: number;
	newTileIndex: number;
};

/** Undo data for paint operations */
export type PaintUndoData = {
	changes: TileChange[];
};


// ============================================================================
// Internal Helpers
// ============================================================================

/**
 * Get tile index from tile ID using the tiles map.
 * Returns -1 if tile not found.
 */
function getTileIndex(tileId: string): number {
	const tiles = AppState.tiles.value;
	if (!tiles) return -1;

	let index = 0;
	for (const [id] of tiles) {
		if (id === tileId) return index;
		index++;
	}
	return -1;
}

/**
 * Get map cell index from x,y coordinates.
 */
function getMapIndex(x: number, y: number): number {
	const mapProject = AppState.mapProject.value;
	if (!mapProject) return -1;
	return y * mapProject.width + x;
}

/**
 * Check if coordinates are within map bounds.
 */
function isInBounds(x: number, y: number): boolean {
	const mapProject = AppState.mapProject.value;
	if (!mapProject) return false;
	return x >= 0 && x < mapProject.width && y >= 0 && y < mapProject.height;
}

/**
 * Apply changes to the map and trigger WebGL update.
 */
function applyChanges(changes: TileChange[], useNewValue: boolean) {
	const map = AppState.map.value;
	const wglMap = AppState.wglMap.value;
	if (!map || !wglMap) return;

	for (const change of changes) {
		const index = getMapIndex(change.x, change.y);
		if (index >= 0) {
			map[index] = useNewValue ? change.newTileIndex : change.oldTileIndex;
		}
	}

	// Trigger reactive update
	AppState.map.set(map);
}


// ============================================================================
// Painting Actions
// ============================================================================

/**
 * Paint a single tile at the given coordinates.
 */
export function paintTile(x: number, y: number): boolean {
	const tileId = EditorState.selectedTile.value;
	const map = AppState.map.value;
	if (!tileId || !map) return false;
	if (!isInBounds(x, y)) return false;

	const newTileIndex = getTileIndex(tileId);
	if (newTileIndex < 0) return false;

	const mapIndex = getMapIndex(x, y);
	const oldTileIndex = map[mapIndex];

	// Skip if same tile
	if (oldTileIndex === newTileIndex) return false;

	const change: TileChange = { x, y, oldTileIndex, newTileIndex };

	// Apply change
	map[mapIndex] = newTileIndex;
	AppState.map.set(map);

	// Record in history
	HistoryState.push('paint', { changes: [change] }, { changes: [change] }, `Paint tile at (${x}, ${y})`);

	return true;
}

/**
 * Paint multiple tiles in a brush stroke.
 * Collects all changes into a single history entry.
 */
export function paintBrush(cells: Array<{ x: number; y: number }>): boolean {
	const tileId = EditorState.selectedTile.value;
	const map = AppState.map.value;
	if (!tileId || !map) return false;

	const newTileIndex = getTileIndex(tileId);
	if (newTileIndex < 0) return false;

	const changes: TileChange[] = [];

	for (const { x, y } of cells) {
		if (!isInBounds(x, y)) continue;

		const mapIndex = getMapIndex(x, y);
		const oldTileIndex = map[mapIndex];

		// Skip if same tile
		if (oldTileIndex === newTileIndex) continue;

		changes.push({ x, y, oldTileIndex, newTileIndex });
		map[mapIndex] = newTileIndex;
	}

	if (changes.length === 0) return false;

	// Trigger reactive update
	AppState.map.set(map);

	// Record in history
	HistoryState.push('paint', { changes }, { changes }, `Paint ${changes.length} tiles`);

	return true;
}

/**
 * Paint a filled rectangle.
 */
export function paintRect(x1: number, y1: number, x2: number, y2: number): boolean {
	const minX = Math.min(x1, x2);
	const maxX = Math.max(x1, x2);
	const minY = Math.min(y1, y2);
	const maxY = Math.max(y1, y2);

	const cells: Array<{ x: number; y: number }> = [];
	for (let y = minY; y <= maxY; y++) {
		for (let x = minX; x <= maxX; x++) {
			cells.push({ x, y });
		}
	}

	return paintBrush(cells);
}

/**
 * Paint a filled ellipse.
 */
export function paintEllipse(cx: number, cy: number, rx: number, ry: number): boolean {
	const cells: Array<{ x: number; y: number }> = [];

	for (let y = cy - ry; y <= cy + ry; y++) {
		for (let x = cx - rx; x <= cx + rx; x++) {
			// Check if point is inside ellipse
			const dx = (x - cx) / rx;
			const dy = (y - cy) / ry;
			if (dx * dx + dy * dy <= 1) {
				cells.push({ x, y });
			}
		}
	}

	return paintBrush(cells);
}

/**
 * Flood fill starting from a position.
 * Fills all connected tiles of the same type.
 */
export function floodFill(startX: number, startY: number): boolean {
	const tileId = EditorState.selectedTile.value;
	const map = AppState.map.value;
	const mapProject = AppState.mapProject.value;
	if (!tileId || !map || !mapProject) return false;
	if (!isInBounds(startX, startY)) return false;

	const newTileIndex = getTileIndex(tileId);
	if (newTileIndex < 0) return false;

	const startIndex = getMapIndex(startX, startY);
	const targetTileIndex = map[startIndex];

	// Skip if same tile
	if (targetTileIndex === newTileIndex) return false;

	const visited = new Set<number>();
	const stack: Array<{ x: number; y: number }> = [{ x: startX, y: startY }];
	const changes: TileChange[] = [];

	while (stack.length > 0) {
		const { x, y } = stack.pop()!;
		const mapIndex = getMapIndex(x, y);

		if (visited.has(mapIndex)) continue;
		if (!isInBounds(x, y)) continue;
		if (map[mapIndex] !== targetTileIndex) continue;

		visited.add(mapIndex);
		changes.push({ x, y, oldTileIndex: targetTileIndex, newTileIndex });
		map[mapIndex] = newTileIndex;

		// Add neighbors
		stack.push({ x: x - 1, y });
		stack.push({ x: x + 1, y });
		stack.push({ x, y: y - 1 });
		stack.push({ x, y: y + 1 });
	}

	if (changes.length === 0) return false;

	// Trigger reactive update
	AppState.map.set(map);

	// Record in history
	HistoryState.push('fill', { changes }, { changes }, `Fill ${changes.length} tiles`);

	return true;
}

/**
 * Pick a tile from the map (eyedropper tool).
 */
export function pickTile(x: number, y: number): boolean {
	const map = AppState.map.value;
	const tiles = AppState.tiles.value;
	if (!map || !tiles) return false;
	if (!isInBounds(x, y)) return false;

	const mapIndex = getMapIndex(x, y);
	const tileIndex = map[mapIndex];

	// Find tile ID by index
	let index = 0;
	for (const [id] of tiles) {
		if (index === tileIndex) {
			EditorState.selectTile(id);
			return true;
		}
		index++;
	}

	return false;
}


// ============================================================================
// Undo/Redo Handlers
// ============================================================================

/**
 * Apply undo for paint operations.
 */
export function undoPaint(undoData: PaintUndoData) {
	applyChanges(undoData.changes, false);
}

/**
 * Apply redo for paint operations.
 */
export function redoPaint(redoData: PaintUndoData) {
	applyChanges(redoData.changes, true);
}
