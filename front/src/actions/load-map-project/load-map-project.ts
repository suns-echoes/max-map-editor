import { readTextFile } from '^tauri-apps/plugin-fs.ts';
import { AppState } from '^state/app-state.ts';
import { Perf } from '^lib/perf/perf.ts';
import { printDebugInfo } from '^lib/debug/debug.ts';
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
	printDebugInfo('loadMapProject');
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


function parseMapProject(mapProject: string): MapProject {
	// TODO: add validation
	return JSON.parse<MapProject>(mapProject);
}
