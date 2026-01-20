/**
 * WRL File Format Types
 *
 * The WRL file structure (from M.A.X. game):
 *
 *  Content   | Size in bytes       | Data type
 * -----------+---------------------+-----------
 *  header    | 5                   | uint8
 *  width     | 2                   | uint16
 *  height    | 2                   | uint16
 *  minimap   | width * height      | uint8
 *  bigmap    | width * height * 2  | uint16
 *  tileCount | 2                   | uint16
 *  tiles     | 64 * 64 * tileCount | uint8
 *  palette   | 256 * 3             | uint8
 *  passtab   | tileCount           | uint8
 */

export const WRL_HEADER = new Uint8Array([0x57, 0x52, 0x4C, 0x20, 0x02]); // "WRL " + version 2
export const TILE_SIZE = 64;
export const TILE_PIXELS = TILE_SIZE * TILE_SIZE;
export const PALETTE_SIZE = 256 * 3;

export interface WrlData {
	header: Uint8Array;
	width: number;
	height: number;
	minimap: Uint8Array;
	bigmap: Uint16Array;
	tileCount: number;
	tiles: Uint8Array[];
	palette: Uint8Array;
	passtab: Uint8Array;
}

/**
 * Pass table values for tile types
 */
export const PASS_VALUES = {
	LAND: 0x00,
	WATER: 0x01,
	SHORE: 0x02,
	OBSTRUCTION: 0x03,
} as const;

/**
 * Get pass table value for a tile type
 */
export function getTilePassValue(type: TileProps['type']): number {
	switch (type) {
		case 'water': return PASS_VALUES.WATER;
		case 'shore': return PASS_VALUES.SHORE;
		case 'land': return PASS_VALUES.LAND;
		case 'obstruction': return PASS_VALUES.OBSTRUCTION;
		default: return PASS_VALUES.LAND;
	}
}

/**
 * Get tile type from pass table value
 */
export function getTileTypeFromPass(passValue: number): TileProps['type'] {
	switch (passValue) {
		case PASS_VALUES.WATER: return 'water';
		case PASS_VALUES.SHORE: return 'shore';
		case PASS_VALUES.OBSTRUCTION: return 'obstruction';
		default: return 'land';
	}
}
