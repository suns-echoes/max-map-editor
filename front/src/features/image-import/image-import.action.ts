/**
 * Image Import Action
 *
 * Imports an image and converts it to a M.A.X. compatible map
 */

import { xlog } from '^lib/xlog/xlog.ts';
import { AppState, arrangeTilesData } from '^state/app-state.ts';
import { HistoryState } from '^state/history-state.ts';
import { RustAPI } from '^src/bff/rust-api';
import { ImageImportState } from './image-import.state.ts';
import { TILE_PIXELS, type ImageImportResult } from './image-import.types.ts';


/**
 * Import image from file path (Tauri native)
 */
export async function importImageFromPath(filePath: string): Promise<ImageImportResult> {
	xlog.info('importImageFromPath', filePath);

	ImageImportState.start();

	try {
		// Step 1: Call Rust to convert image
		ImageImportState.setProgress(10, 'Converting image...');
		const [palette, indexedTiles] = await RustAPI.imageToWRL(filePath);

		// Step 2: Process the result
		ImageImportState.setProgress(50, 'Processing tiles...');
		const result = processImageResult(palette, indexedTiles, getFileBaseName(filePath));

		ImageImportState.complete();
		return result;
	} catch (err) {
		const message = err instanceof Error ? err.message : 'Unknown error';
		xlog.error('Image import failed:', message);
		ImageImportState.reset();
		return { success: false, error: message };
	}
}


/**
 * Extract filename without extension
 */
function getFileBaseName(filePath: string): string {
	const fileName = filePath.split(/[/\\]/).pop() ?? 'Imported';
	return fileName.replace(/\.[^.]+$/, '');
}


/**
 * Process the result from Rust image conversion
 */
function processImageResult(
	paletteData: Uint8Array,
	tilesData: Uint8Array,
	mapName: string
): ImageImportResult {
	xlog.info('processImageResult', { paletteSize: paletteData.length, tilesSize: tilesData.length });

	// Calculate number of tiles
	const tileCount = tilesData.length / TILE_PIXELS;
	if (!Number.isInteger(tileCount)) {
		return {
			success: false,
			error: `Invalid tiles data: ${tilesData.length} bytes is not divisible by ${TILE_PIXELS}`,
		};
	}

	// Calculate map dimensions (assuming square map for now)
	const tilesPerSide = Math.sqrt(tileCount);
	if (!Number.isInteger(tilesPerSide)) {
		return {
			success: false,
			error: `Invalid tile count: ${tileCount} is not a perfect square`,
		};
	}

	const mapWidth = tilesPerSide;
	const mapHeight = tilesPerSide;

	xlog.info(`Map: ${mapWidth}x${mapHeight}, ${tileCount} tiles`);

	// Step 3: Create tiles
	ImageImportState.setProgress(60, 'Creating tiles...');
	const tiles = createTilesFromData(tilesData, tileCount);

	// Step 4: Create map (simple sequential tile indices)
	ImageImportState.setProgress(80, 'Building map...');
	const map = new Uint16Array(tileCount);
	for (let i = 0; i < tileCount; i++) {
		map[i] = i;
	}

	// Step 5: Create map project
	const mapProject = createMapProject(mapName, mapWidth, mapHeight, tiles);

	// Step 6: Clear history
	HistoryState.clear();

	// Step 7: Arrange tiles for WebGL
	ImageImportState.setProgress(90, 'Preparing renderer...');
	const wglMap = AppState.wglMap.value;
	if (wglMap) {
		arrangeTilesData(tiles, wglMap.getTileCapability());
	}

	// Step 8: Update app state
	AppState.mapProject.set(mapProject);
	AppState.palette.set(paletteData);
	AppState.tiles.set(tiles);
	AppState.map.set(map);

	xlog.info(`Image imported: ${mapWidth}x${mapHeight}, ${tileCount} tiles`);

	return {
		success: true,
		mapName: mapProject.name,
		width: mapWidth,
		height: mapHeight,
		tileCount,
	};
}


/**
 * Create tiles map from raw tile data
 */
function createTilesFromData(tilesData: Uint8Array, tileCount: number): Tiles {
	const tiles: Tiles = new Map();

	for (let i = 0; i < tileCount; i++) {
		const offset = i * TILE_PIXELS;
		const tileData = tilesData.slice(offset, offset + TILE_PIXELS);
		const tileId = `IMG${i.toString().padStart(4, '0')}`;

		// Determine tile type based on pixel content (heuristic)
		const tileType = analyzeTileType(tileData);

		const tile: Tile = {
			data: tileData,
			match: {
				N: [], W: [], S: [], E: [],
				'!N': [], '!W': [], '!S': [], '!E': [],
			},
			props: {
				type: tileType,
				hasVariants: false,
				useMaskColor: false,
				transformable: false,
			},
			transformation: 'N',
			variantsName: null,
			assetInfo: {
				assetName: 'IMPORTED',
				tileId: tileId,
			},
			inUse: true,
			location: {
				dataOffset: 0,
				textureLayer: 0,
				textureX: 0,
				textureY: 0,
			},
		};

		tiles.set(tileId, tile);
	}

	return tiles;
}


/**
 * Analyze tile data to determine type
 * This is a simple heuristic - could be improved
 */
function analyzeTileType(tileData: Uint8Array): TileProps['type'] {
	// Count unique colors
	const colorCounts = new Map<number, number>();
	for (const color of tileData) {
		colorCounts.set(color, (colorCounts.get(color) ?? 0) + 1);
	}

	// If very few colors and mostly one color, might be water
	const sortedCounts = [...colorCounts.values()].sort((a, b) => b - a);
	if (sortedCounts.length <= 3 && sortedCounts[0] > TILE_PIXELS * 0.9) {
		// Very uniform - likely water
		return 'water';
	}

	// Default to land
	return 'land';
}


/**
 * Create map project from imported data
 */
function createMapProject(
	name: string,
	width: number,
	height: number,
	tiles: Tiles
): MapProject {
	const tilesArray = Array.from(tiles.keys());

	// Build map grid
	const mapGrid: string[][] = [];
	let tileIndex = 0;

	for (let y = 0; y < height; y++) {
		const row: string[] = [];
		for (let x = 0; x < width; x++) {
			row.push(tilesArray[tileIndex] ?? 'IMG0000');
			tileIndex++;
		}
		mapGrid.push(row);
	}

	return {
		version: '1',
		name,
		description: `Imported from image`,
		width,
		height,
		use: [
			{
				name: 'IMPORTED',
				tileset: true,
				palette: true,
				version: 1,
			}
		],
		map: mapGrid,
	};
}
