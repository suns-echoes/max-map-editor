/**
 * WRL Import Action
 *
 * Imports WRL files and converts them to the app's internal format
 */

import { xlog } from '^lib/xlog/xlog.ts';
import { AppState, arrangeTilesData } from '^state/app-state.ts';
import { HistoryState } from '^state/history-state.ts';
import { parseWrlFile, validateWrlBuffer } from './wrl-parser.ts';
import { getTileTypeFromPass, type WrlData } from './wrl-format.ts';


export interface WrlImportResult {
	success: boolean;
	error?: string;
	mapName?: string;
	width?: number;
	height?: number;
	tileCount?: number;
}


/**
 * Import a WRL file from an ArrayBuffer
 */
export async function importWrlFile(buffer: ArrayBuffer, fileName: string): Promise<WrlImportResult> {
	xlog.info('importWrlFile', fileName);

	// Validate the buffer
	const validation = validateWrlBuffer(buffer);
	if (!validation.valid) {
		xlog.error('WRL validation failed:', validation.error);
		return { success: false, error: validation.error };
	}

	try {
		// Parse the WRL file
		const wrlData = parseWrlFile(buffer);

		// Convert to app format
		const { mapProject, map, tiles, palette } = convertWrlToAppFormat(wrlData, fileName);

		// Clear history since we're loading a new map
		HistoryState.clear();

		// Arrange tiles for WebGL
		const wglMap = AppState.wglMap.value;
		if (wglMap) {
			arrangeTilesData(tiles, wglMap.getTileCapability());
		}

		// Update app state
		AppState.mapProject.set(mapProject);
		AppState.palette.set(palette);
		AppState.tiles.set(tiles);
		AppState.map.set(map);

		xlog.info(`WRL imported: ${wrlData.width}x${wrlData.height}, ${wrlData.tileCount} tiles`);

		return {
			success: true,
			mapName: mapProject.name,
			width: wrlData.width,
			height: wrlData.height,
			tileCount: wrlData.tileCount,
		};
	} catch (err) {
		const message = err instanceof Error ? err.message : 'Unknown error';
		xlog.error('WRL import failed:', message);
		return { success: false, error: message };
	}
}


/**
 * Convert WRL data to app format
 */
function convertWrlToAppFormat(wrlData: WrlData, fileName: string): {
	mapProject: MapProject;
	map: Uint16Array;
	tiles: Tiles;
	palette: Uint8Array;
} {
	// Create map project
	const mapName = fileName.replace(/\.wrl$/i, '');
	const mapProject: MapProject = {
		version: '1',
		name: mapName,
		description: `Imported from ${fileName}`,
		width: wrlData.width,
		height: wrlData.height,
		use: [
			{
				name: 'IMPORTED',
				tileset: true,
				palette: true,
				version: 1,
			}
		],
		map: [], // Will be populated from bigmap indices
	};

	// Create tiles map
	const tiles: Tiles = new Map();
	for (let i = 0; i < wrlData.tileCount; i++) {
		const tileId = `IMP${i.toString().padStart(3, '0')}`;
		const passValue = wrlData.passtab[i];
		const tileType = getTileTypeFromPass(passValue);

		const tile: Tile = {
			data: wrlData.tiles[i],
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

	// Map data is already in the correct format (Uint16Array of tile indices)
	// but we need to clone it
	const map = new Uint16Array(wrlData.bigmap);

	// Build the mapProject.map array for JSON serialization
	const tilesArray = Array.from(tiles.keys());
	for (let y = 0; y < wrlData.height; y++) {
		const row: string[] = [];
		for (let x = 0; x < wrlData.width; x++) {
			const index = y * wrlData.width + x;
			const tileIndex = wrlData.bigmap[index];
			row.push(tilesArray[tileIndex] ?? 'IMP000');
		}
		mapProject.map.push(row);
	}

	return {
		mapProject,
		map,
		tiles,
		palette: wrlData.palette,
	};
}


/**
 * Import WRL from File object (for drag-and-drop or file input)
 */
export async function importWrlFromFile(file: File): Promise<WrlImportResult> {
	try {
		const buffer = await file.arrayBuffer();
		return importWrlFile(buffer, file.name);
	} catch (err) {
		const message = err instanceof Error ? err.message : 'Failed to read file';
		xlog.error('Failed to read WRL file:', message);
		return { success: false, error: message };
	}
}
