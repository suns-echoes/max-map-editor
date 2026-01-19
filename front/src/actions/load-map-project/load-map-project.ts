import { readTextFile } from '^tauri-apps/plugin-fs.ts';
import { AppState } from '^state/app-state.ts';
import { Perf } from '^lib/perf/perf.ts';
import { xlog } from '^lib/xlog/xlog.ts';
import { loadPalette } from './load-assets/load-palette.ts';
import { loadTileSet } from './load-assets/load-tile-set.ts';
import { parseMap } from './load-assets/load-map.ts';
import { arrangeTilesData } from '^state/app-state.ts';


/**
 * Load a map project and all its assets.
 * This is the main entry point for loading maps.
 * All loading is done imperatively in sequence, then state is updated.
 */
export async function loadMapProject(projectFilePath: string): Promise<void> {
	xlog.info('loadMapProject');
	const perf = Perf('loadMapProject');

	// 1. Parse map project file
	const mapFile = await readTextFile(projectFilePath);
	const mapProject = parseMapProject(mapFile);

	// 2. Load assets (palette + tiles)
	let palette: Uint8Array | null = null;
	const tiles: Tiles = new Map();

	for (const asset of mapProject.use) {
		if (asset.palette) {
			palette = await loadPalette(asset.name);
		}
		if (asset.tileset) {
			await loadTileSet(tiles, asset.name);
		}
	}

	if (!palette) {
		throw new Error('Fatal: No palette loaded');
	}

	// 3. Arrange tiles data (sets tile.location for each tile)
	// This must happen BEFORE parseMap because parseMap reads tile.location
	const wglMap = AppState.wglMap.value;
	if (wglMap) {
		arrangeTilesData(tiles, wglMap.getTileCapability());
	}

	// 4. Parse map data (requires tiles for location lookup)
	const map = parseMap(mapProject, tiles);

	// 5. Update state in one batch
	AppState.mapProject.set(mapProject);
	AppState.palette.set(palette);
	AppState.tiles.set(tiles);
	AppState.map.set(map);

	perf();
}


/**
 * Parse and validate a map project JSON string.
 * Throws detailed errors if validation fails.
 */
function parseMapProject(jsonString: string): MapProject {
	let data: unknown;
	try {
		data = JSON.parse(jsonString);
	} catch (e) {
		throw new MapProjectValidationError('Invalid JSON', e instanceof Error ? e.message : String(e));
	}

	validateMapProject(data);
	return data;
}


// ============================================================================
// Validation Error
// ============================================================================

class MapProjectValidationError extends Error {
	constructor(field: string, message: string) {
		super(`MapProject validation failed at '${field}': ${message}`);
		this.name = 'MapProjectValidationError';
	}
}


// ============================================================================
// Validation Functions
// ============================================================================

/**
 * Validates data matches the MapProject type.
 * Uses assertion function to narrow type.
 */
function validateMapProject(data: unknown): asserts data is MapProject {
	if (data === null || typeof data !== 'object') {
		throw new MapProjectValidationError('root', 'Expected an object');
	}

	const obj = data as Record<string, unknown>;

	// version: must be exactly 0.1
	if (obj.version !== 0.1) {
		throw new MapProjectValidationError('version', `Expected 0.1, got ${obj.version}`);
	}

	// name: required string
	if (typeof obj.name !== 'string' || obj.name.length === 0) {
		throw new MapProjectValidationError('name', 'Expected non-empty string');
	}

	// description: required string (can be empty)
	if (typeof obj.description !== 'string') {
		throw new MapProjectValidationError('description', 'Expected string');
	}

	// width: positive integer
	if (typeof obj.width !== 'number' || !Number.isInteger(obj.width) || obj.width <= 0) {
		throw new MapProjectValidationError('width', 'Expected positive integer');
	}

	// height: positive integer
	if (typeof obj.height !== 'number' || !Number.isInteger(obj.height) || obj.height <= 0) {
		throw new MapProjectValidationError('height', 'Expected positive integer');
	}

	// use: array of asset references
	if (!Array.isArray(obj.use)) {
		throw new MapProjectValidationError('use', 'Expected array');
	}
	if (obj.use.length === 0) {
		throw new MapProjectValidationError('use', 'Expected at least one asset');
	}
	for (let i = 0; i < obj.use.length; i++) {
		validateAssetReference(obj.use[i], `use[${i}]`);
	}

	// map: array of rows (string or string[])
	if (!Array.isArray(obj.map)) {
		throw new MapProjectValidationError('map', 'Expected array');
	}
	if (obj.map.length !== obj.height) {
		throw new MapProjectValidationError('map', `Expected ${obj.height} rows, got ${obj.map.length}`);
	}
	for (let y = 0; y < obj.map.length; y++) {
		validateMapRow(obj.map[y], obj.width, `map[${y}]`);
	}
}

/**
 * Validates an asset reference in the 'use' array.
 */
function validateAssetReference(data: unknown, path: string): void {
	if (data === null || typeof data !== 'object') {
		throw new MapProjectValidationError(path, 'Expected an object');
	}

	const obj = data as Record<string, unknown>;

	// name: required string
	if (typeof obj.name !== 'string' || obj.name.length === 0) {
		throw new MapProjectValidationError(`${path}.name`, 'Expected non-empty string');
	}

	// version: required number
	if (typeof obj.version !== 'number') {
		throw new MapProjectValidationError(`${path}.version`, 'Expected number');
	}

	// tileset: optional boolean
	if (obj.tileset !== undefined && typeof obj.tileset !== 'boolean') {
		throw new MapProjectValidationError(`${path}.tileset`, 'Expected boolean');
	}

	// palette: optional boolean
	if (obj.palette !== undefined && typeof obj.palette !== 'boolean') {
		throw new MapProjectValidationError(`${path}.palette`, 'Expected boolean');
	}
}

/**
 * Validates a map row. Each cell is either a string or string[].
 */
function validateMapRow(row: unknown, expectedWidth: number, path: string): void {
	if (!Array.isArray(row)) {
		throw new MapProjectValidationError(path, 'Expected array');
	}
	if (row.length !== expectedWidth) {
		throw new MapProjectValidationError(path, `Expected ${expectedWidth} cells, got ${row.length}`);
	}
	for (let x = 0; x < row.length; x++) {
		const cell = row[x];
		if (typeof cell === 'string') {
			continue; // valid: single tile ID
		}
		if (Array.isArray(cell)) {
			// valid: array of tile IDs (layered)
			for (let layer = 0; layer < cell.length; layer++) {
				if (typeof cell[layer] !== 'string') {
					throw new MapProjectValidationError(
						`${path}[${x}][${layer}]`,
						`Expected string, got ${typeof cell[layer]}`
					);
				}
			}
			continue;
		}
		throw new MapProjectValidationError(`${path}[${x}]`, `Expected string or string[], got ${typeof cell}`);
	}
}
