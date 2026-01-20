/**
 * Pass Table Editor Actions
 *
 * Actions for editing tile pass values with undo/redo support.
 */

import { xlog } from '^lib/xlog/xlog.ts';
import { AppState } from '^state/app-state.ts';
import { HistoryState } from '^state/history-state.ts';
import { PassTableState } from './pass-table.state.ts';
import {
	type PassValue,
	type PassChange,
	type PassTableUndoData,
	type PassStats,
	PASS_VALUES,
	getPassValueFromType,
	getTypeFromPassValue,
} from './pass-table.types.ts';


// ============================================================================
// Pass Table Read Operations
// ============================================================================

/**
 * Get pass value for a tile by ID
 */
export function getTilePassValue(tileId: string): PassValue | null {
	const tiles = AppState.tiles.value;
	if (!tiles) return null;

	const tile = tiles.get(tileId);
	if (!tile) return null;

	return getPassValueFromType(tile.props.type);
}

/**
 * Get all pass values as a map
 */
export function getAllPassValues(): Map<string, PassValue> {
	const tiles = AppState.tiles.value;
	const result = new Map<string, PassValue>();

	if (!tiles) return result;

	for (const [tileId, tile] of tiles) {
		result.set(tileId, getPassValueFromType(tile.props.type));
	}

	return result;
}

/**
 * Get pass statistics for current tileset
 */
export function getPassStats(): PassStats {
	const tiles = AppState.tiles.value;
	const stats: PassStats = { land: 0, water: 0, shore: 0, obstruction: 0, total: 0 };

	if (!tiles) return stats;

	for (const tile of tiles.values()) {
		stats.total++;
		switch (tile.props.type) {
			case 'land': stats.land++; break;
			case 'water': stats.water++; break;
			case 'shore': stats.shore++; break;
			case 'obstruction': stats.obstruction++; break;
		}
	}

	return stats;
}


// ============================================================================
// Pass Table Write Operations
// ============================================================================

/**
 * Set pass value for a single tile
 */
export function setTilePassValue(tileId: string, newValue: PassValue): boolean {
	const tiles = AppState.tiles.value;
	if (!tiles) return false;

	const tile = tiles.get(tileId);
	if (!tile) return false;

	const oldValue = getPassValueFromType(tile.props.type);
	if (oldValue === newValue) return false;

	const newType = getTypeFromPassValue(newValue);

	// Update tile props
	tile.props.type = newType;

	// Record change
	const change: PassChange = { tileId, oldValue, newValue };
	recordChange([change]);

	// Trigger reactive update
	AppState.tiles.set(new Map(tiles));

	xlog.info('PassTableEditor', `Set ${tileId} from ${oldValue} to ${newValue} (${newType})`);

	return true;
}

/**
 * Set pass value for multiple tiles at once
 */
export function setMultipleTilePassValues(
	tileIds: string[],
	newValue: PassValue
): number {
	const tiles = AppState.tiles.value;
	if (!tiles) return 0;

	const changes: PassChange[] = [];

	for (const tileId of tileIds) {
		const tile = tiles.get(tileId);
		if (!tile) continue;

		const oldValue = getPassValueFromType(tile.props.type);
		if (oldValue === newValue) continue;

		const newType = getTypeFromPassValue(newValue);
		tile.props.type = newType;

		changes.push({ tileId, oldValue, newValue });
	}

	if (changes.length === 0) return 0;

	// Record changes
	recordChange(changes);

	// Trigger reactive update
	AppState.tiles.set(new Map(tiles));

	xlog.info('PassTableEditor', `Updated ${changes.length} tiles to pass value ${newValue}`);

	return changes.length;
}

/**
 * Cycle pass value for a tile (land -> water -> shore -> obstruction -> land)
 */
export function cycleTilePassValue(tileId: string): boolean {
	const current = getTilePassValue(tileId);
	if (current === null) return false;

	const order: PassValue[] = [
		PASS_VALUES.LAND,
		PASS_VALUES.WATER,
		PASS_VALUES.SHORE,
		PASS_VALUES.OBSTRUCTION,
	];

	const currentIndex = order.indexOf(current);
	const nextValue = order[(currentIndex + 1) % order.length];

	return setTilePassValue(tileId, nextValue);
}


// ============================================================================
// Bulk Operations
// ============================================================================

/**
 * Set all tiles of a specific type to a new pass value
 */
export function setPassValueForType(
	fromType: TileProps['type'],
	toValue: PassValue
): number {
	const tiles = AppState.tiles.value;
	if (!tiles) return 0;

	const tileIds: string[] = [];

	for (const [tileId, tile] of tiles) {
		if (tile.props.type === fromType) {
			tileIds.push(tileId);
		}
	}

	return setMultipleTilePassValues(tileIds, toValue);
}

/**
 * Auto-detect pass values based on tile analysis
 * (Uses simple heuristics based on existing type values)
 */
export function autoDetectPassValues(): number {
	const tiles = AppState.tiles.value;
	if (!tiles) return 0;

	let changedCount = 0;

	// This is mostly a no-op since we already use type for pass values,
	// but could be extended with more sophisticated detection
	xlog.info('PassTableEditor', 'Auto-detect pass values (using existing tile types)');

	return changedCount;
}


// ============================================================================
// Paint Operations (for editor mode)
// ============================================================================

/**
 * Paint pass value at map position
 * (Changes the pass value of the tile at that position)
 */
export function paintPassValueAt(x: number, y: number): boolean {
	if (!PassTableState.canPaint.value) return false;

	const mapProject = AppState.mapProject.value;
	const map = AppState.map.value;
	const tiles = AppState.tiles.value;

	if (!mapProject || !map || !tiles) return false;

	const { width, height } = mapProject;
	if (x < 0 || x >= width || y < 0 || y >= height) return false;

	const mapIndex = y * width + x;
	const tileIndex = map[mapIndex];

	// Get tile ID from index
	const tilesArray = [...tiles.entries()];
	if (tileIndex >= tilesArray.length) return false;

	const [tileId] = tilesArray[tileIndex];
	const selectedValue = PassTableState.selectedPassValue.value;

	return setTilePassValue(tileId, selectedValue);
}

/**
 * Pick pass value from map position (eyedropper)
 */
export function pickPassValueAt(x: number, y: number): boolean {
	const mapProject = AppState.mapProject.value;
	const map = AppState.map.value;
	const tiles = AppState.tiles.value;

	if (!mapProject || !map || !tiles) return false;

	const { width, height } = mapProject;
	if (x < 0 || x >= width || y < 0 || y >= height) return false;

	const mapIndex = y * width + x;
	const tileIndex = map[mapIndex];

	// Get tile ID from index
	const tilesArray = [...tiles.entries()];
	if (tileIndex >= tilesArray.length) return false;

	const [tileId] = tilesArray[tileIndex];
	const passValue = getTilePassValue(tileId);

	if (passValue !== null) {
		PassTableState.selectPassValue(passValue);
		return true;
	}

	return false;
}


// ============================================================================
// History Integration
// ============================================================================

/**
 * Record pass table changes for undo/redo
 */
function recordChange(changes: PassChange[]) {
	const undoData: PassTableUndoData = { changes };

	// Swap old/new for redo
	const redoChanges = changes.map((c) => ({
		tileId: c.tileId,
		oldValue: c.newValue,
		newValue: c.oldValue,
	}));
	const redoData: PassTableUndoData = { changes: redoChanges };

	const description = changes.length === 1
		? `Change pass value: ${changes[0].tileId}`
		: `Change pass values: ${changes.length} tiles`;

	HistoryState.push('pass-table', undoData, redoData, description);
}

/**
 * Apply undo/redo for pass table changes
 */
export function applyPassTableUndo(data: PassTableUndoData) {
	const tiles = AppState.tiles.value;
	if (!tiles) return;

	for (const change of data.changes) {
		const tile = tiles.get(change.tileId);
		if (tile) {
			// Undo: revert to old value
			tile.props.type = getTypeFromPassValue(change.oldValue);
		}
	}

	// Trigger reactive update
	AppState.tiles.set(new Map(tiles));
}

/**
 * Apply redo for pass table changes
 */
export function applyPassTableRedo(data: PassTableUndoData) {
	const tiles = AppState.tiles.value;
	if (!tiles) return;

	for (const change of data.changes) {
		const tile = tiles.get(change.tileId);
		if (tile) {
			// Redo: apply new value (which in redo data is stored as oldValue due to swap)
			tile.props.type = getTypeFromPassValue(change.oldValue);
		}
	}

	// Trigger reactive update
	AppState.tiles.set(new Map(tiles));
}
