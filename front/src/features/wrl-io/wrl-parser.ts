/**
 * WRL File Parser
 *
 * Parses binary WRL files into structured data
 */

import { TILE_PIXELS, PALETTE_SIZE, type WrlData } from './wrl-format.ts';


/**
 * Parse a WRL file from an ArrayBuffer
 */
export function parseWrlFile(buffer: ArrayBuffer): WrlData {
	const dataView = new DataView(buffer);
	let offset = 0;

	// Header (5 bytes)
	const header = new Uint8Array(buffer, offset, 5);
	offset += 5;

	// Validate header signature ("WRL ")
	if (header[0] !== 0x57 || header[1] !== 0x52 || header[2] !== 0x4C || header[3] !== 0x20) {
		throw new Error('Invalid WRL file: incorrect header signature');
	}

	// Width (2 bytes, uint16 LE)
	const width = dataView.getUint16(offset, true);
	offset += 2;

	// Height (2 bytes, uint16 LE)
	const height = dataView.getUint16(offset, true);
	offset += 2;

	// Minimap (width * height bytes)
	const minimapSize = width * height;
	const minimap = new Uint8Array(buffer, offset, minimapSize);
	offset += minimapSize;

	// Bigmap (width * height * 2 bytes, uint16 LE)
	const bigmapSize = width * height * 2;
	const bigmapBuffer = buffer.slice(offset, offset + bigmapSize);
	const bigmap = new Uint16Array(bigmapBuffer);
	offset += bigmapSize;

	// Tile count (2 bytes, uint16 LE)
	const tileCount = dataView.getUint16(offset, true);
	offset += 2;

	// Tiles (64 * 64 * tileCount bytes)
	const tiles: Uint8Array[] = [];
	for (let i = 0; i < tileCount; i++) {
		tiles.push(new Uint8Array(buffer, offset, TILE_PIXELS));
		offset += TILE_PIXELS;
	}

	// Palette (256 * 3 bytes)
	const palette = new Uint8Array(buffer, offset, PALETTE_SIZE);
	offset += PALETTE_SIZE;

	// Pass table (tileCount bytes)
	const passtab = new Uint8Array(buffer, offset, tileCount);
	offset += tileCount;

	return {
		header: new Uint8Array(header),
		width,
		height,
		minimap: new Uint8Array(minimap),
		bigmap,
		tileCount,
		tiles,
		palette: new Uint8Array(palette),
		passtab: new Uint8Array(passtab),
	};
}


/**
 * Calculate the expected file size for a WRL file
 */
export function calculateWrlFileSize(width: number, height: number, tileCount: number): number {
	const headerSize = 5;
	const dimensionsSize = 4; // 2 + 2 for width and height
	const minimapSize = width * height;
	const bigmapSize = width * height * 2;
	const tileCountSize = 2;
	const tilesSize = TILE_PIXELS * tileCount;
	const paletteSize = PALETTE_SIZE;
	const passtabSize = tileCount;

	return headerSize + dimensionsSize + minimapSize + bigmapSize + tileCountSize + tilesSize + paletteSize + passtabSize;
}


/**
 * Validate that a buffer appears to be a valid WRL file
 */
export function validateWrlBuffer(buffer: ArrayBuffer): { valid: boolean; error?: string } {
	if (buffer.byteLength < 9) {
		return { valid: false, error: 'File too small to be a valid WRL file' };
	}

	const header = new Uint8Array(buffer, 0, 5);
	if (header[0] !== 0x57 || header[1] !== 0x52 || header[2] !== 0x4C || header[3] !== 0x20) {
		return { valid: false, error: 'Invalid WRL header signature' };
	}

	const dataView = new DataView(buffer);
	const width = dataView.getUint16(5, true);
	const height = dataView.getUint16(7, true);

	if (width === 0 || height === 0 || width > 512 || height > 512) {
		return { valid: false, error: `Invalid map dimensions: ${width}x${height}` };
	}

	// Calculate minimum expected size (header + dimensions + minimap + bigmap + tileCount)
	const minSize = 5 + 4 + width * height + width * height * 2 + 2;
	if (buffer.byteLength < minSize) {
		return { valid: false, error: 'File truncated: missing map data' };
	}

	return { valid: true };
}
