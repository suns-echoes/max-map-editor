/**
 * WRL File Builder
 *
 * Creates binary WRL files from app data
 */

import { WRL_HEADER, TILE_PIXELS, PALETTE_SIZE, getTilePassValue, type WrlData } from './wrl-format.ts';
import { calculateWrlFileSize } from './wrl-parser.ts';


/**
 * Build a WRL file buffer from structured data
 */
export function buildWrlFile(data: WrlData): ArrayBuffer {
	const fileSize = calculateWrlFileSize(data.width, data.height, data.tileCount);
	const buffer = new ArrayBuffer(fileSize);
	const view = new DataView(buffer);
	const bytes = new Uint8Array(buffer);
	let offset = 0;

	// Header (5 bytes)
	bytes.set(data.header, offset);
	offset += 5;

	// Width (2 bytes, uint16 LE)
	view.setUint16(offset, data.width, true);
	offset += 2;

	// Height (2 bytes, uint16 LE)
	view.setUint16(offset, data.height, true);
	offset += 2;

	// Minimap (width * height bytes)
	bytes.set(data.minimap, offset);
	offset += data.minimap.length;

	// Bigmap (width * height * 2 bytes, uint16 LE)
	const bigmapBytes = new Uint8Array(data.bigmap.buffer, data.bigmap.byteOffset, data.bigmap.byteLength);
	bytes.set(bigmapBytes, offset);
	offset += bigmapBytes.length;

	// Tile count (2 bytes, uint16 LE)
	view.setUint16(offset, data.tileCount, true);
	offset += 2;

	// Tiles (64 * 64 * tileCount bytes)
	for (const tile of data.tiles) {
		bytes.set(tile, offset);
		offset += TILE_PIXELS;
	}

	// Palette (256 * 3 bytes)
	bytes.set(data.palette, offset);
	offset += PALETTE_SIZE;

	// Pass table (tileCount bytes)
	bytes.set(data.passtab, offset);

	return buffer;
}


/**
 * Generate minimap data from map and tiles
 * Uses the center pixel's color from each tile
 */
export function generateMinimap(
	width: number,
	height: number,
	map: Uint16Array,
	tiles: Uint8Array[]
): Uint8Array {
	const minimap = new Uint8Array(width * height);
	const centerOffset = 32 * 64 + 32; // Center of 64x64 tile

	for (let i = 0; i < map.length; i++) {
		const tileIndex = map[i];
		if (tileIndex < tiles.length) {
			// Get center pixel color index from tile
			minimap[i] = tiles[tileIndex][centerOffset];
		} else {
			minimap[i] = 0;
		}
	}

	return minimap;
}


/**
 * Build WRL data from app state
 */
export function buildWrlDataFromAppState(
	mapProject: MapProject,
	map: Uint16Array,
	tiles: Tiles,
	palette: Uint8Array
): WrlData {
	// Convert tiles map to ordered array
	const tilesArray = Array.from(tiles.entries());
	const tileDataArray: Uint8Array[] = [];
	const passtab = new Uint8Array(tilesArray.length);

	for (let i = 0; i < tilesArray.length; i++) {
		const [, tile] = tilesArray[i];
		tileDataArray.push(tile.data);
		passtab[i] = getTilePassValue(tile.props.type);
	}

	// Generate minimap
	const minimap = generateMinimap(mapProject.width, mapProject.height, map, tileDataArray);

	return {
		header: new Uint8Array(WRL_HEADER),
		width: mapProject.width,
		height: mapProject.height,
		minimap,
		bigmap: map,
		tileCount: tileDataArray.length,
		tiles: tileDataArray,
		palette,
		passtab,
	};
}
