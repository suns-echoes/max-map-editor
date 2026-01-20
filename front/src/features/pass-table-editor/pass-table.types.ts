/**
 * Pass Table Editor Types
 *
 * Types and constants for the pass table editor feature.
 * Pass values determine how units can traverse different tile types.
 */


// ============================================================================
// Pass Value Constants
// ============================================================================

/**
 * Pass values for different terrain types.
 * These values are stored in the WRL file's pass table.
 */
export const PASS_VALUES = {
	LAND: 0x00,        // Ground units can traverse
	WATER: 0x01,       // Water/sea units only
	SHORE: 0x02,       // Both ground and water units (coast tiles)
	OBSTRUCTION: 0x03, // No units can traverse (cliffs, walls, etc.)
} as const;

export type PassValue = typeof PASS_VALUES[keyof typeof PASS_VALUES];


// ============================================================================
// Pass Value Metadata
// ============================================================================

/**
 * Human-readable info for each pass value
 */
export const PASS_INFO: Record<PassValue, { label: string; color: string; description: string }> = {
	[PASS_VALUES.LAND]: {
		label: 'Land',
		color: '#4a7c23',
		description: 'Ground units can traverse',
	},
	[PASS_VALUES.WATER]: {
		label: 'Water',
		color: '#2060a0',
		description: 'Water/naval units only',
	},
	[PASS_VALUES.SHORE]: {
		label: 'Shore',
		color: '#a0a060',
		description: 'Both ground and water units',
	},
	[PASS_VALUES.OBSTRUCTION]: {
		label: 'Obstruction',
		color: '#802020',
		description: 'Impassable terrain',
	},
};


// ============================================================================
// Types
// ============================================================================

/**
 * A change to a tile's pass value
 */
export interface PassChange {
	tileId: string;
	oldValue: PassValue;
	newValue: PassValue;
}

/**
 * Undo data for pass table changes
 */
export interface PassTableUndoData {
	changes: PassChange[];
}

/**
 * Statistics about pass values in the current tileset
 */
export interface PassStats {
	land: number;
	water: number;
	shore: number;
	obstruction: number;
	total: number;
}


// ============================================================================
// Helpers
// ============================================================================

/**
 * Get pass value from tile type string
 */
export function getPassValueFromType(type: TileProps['type']): PassValue {
	switch (type) {
		case 'water': return PASS_VALUES.WATER;
		case 'shore': return PASS_VALUES.SHORE;
		case 'land': return PASS_VALUES.LAND;
		case 'obstruction': return PASS_VALUES.OBSTRUCTION;
		default: return PASS_VALUES.LAND;
	}
}

/**
 * Get tile type string from pass value
 */
export function getTypeFromPassValue(passValue: PassValue): TileProps['type'] {
	switch (passValue) {
		case PASS_VALUES.WATER: return 'water';
		case PASS_VALUES.SHORE: return 'shore';
		case PASS_VALUES.OBSTRUCTION: return 'obstruction';
		default: return 'land';
	}
}

/**
 * Get next pass value in cycle (for quick toggling)
 */
export function cyclePassValue(current: PassValue): PassValue {
	const order: PassValue[] = [
		PASS_VALUES.LAND,
		PASS_VALUES.WATER,
		PASS_VALUES.SHORE,
		PASS_VALUES.OBSTRUCTION,
	];
	const currentIndex = order.indexOf(current);
	return order[(currentIndex + 1) % order.length];
}
