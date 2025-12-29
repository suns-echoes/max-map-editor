import { Effect } from '^lib/reactive/effect.class.ts';
import { Perf } from '^lib/perf/perf.ts';
import { AppState } from '^state/app-state.ts';
import { loadMap } from './load-map.ts';
import { loadPalette } from './load-palette.ts';
import { loadTileSet } from './load-tile-set.ts';


export async function loadAssets(mapProject: MapProject) {
	const perf = Perf('loadAssets');

	const tiles: Tiles = new Map();

	for (const asset of mapProject.use) {
		if (asset.palette) {
			await loadPalette(asset.name);
		}
		if (asset.tileset) {
			await loadTileSet(tiles, asset.name);
		}
	}

	AppState.tiles.set(tiles);

	perf();
}


new Effect(function () {
	const wglMap = AppState.wglMap.value;
	const mapProject = AppState.mapProject.value;
	const tiles = AppState.tiles.value;
	if (!wglMap || !mapProject || !tiles) return;
	const [tileset, layers] = arrangeTilesData(tiles, wglMap.getTileCapability());
	wglMap.initTilesets(tileset, layers);
	loadMap(mapProject, tiles);
}).watch([AppState.wglMap, AppState.mapProject, AppState.tiles]);


function arrangeTilesData(tiles: Tiles, tileCapability: WglTileCapability): [tileset: Uint8Array, layers: number] {
	const perf = Perf('arrangeTilesData');

	const { maxTextureSize, maxTilesPerTextureLayer, tilesPerRow, tilesPerCol } = tileCapability;
	const usedTextureLayers = Math.ceil(tiles.size / maxTilesPerTextureLayer);

	if (usedTextureLayers > tileCapability.maxTextureLayers) {
		throw new Error('Fatal: Too many tiles for the available texture layers');
	}

	const tileset = new Uint8Array(maxTextureSize ** 2 * usedTextureLayers);

	let tileIndex = 0;
	let layer = 0;
	let row = 0;
	let column = 0;
	let textureDataOffset = 0;

	for (const tile of tiles.values()) {
		tile.location.textureLayer = layer;
		tile.location.textureX = row;
		tile.location.textureY = column;

		// tileset.data.set(tile.data, 64 ** 2 * tileIndex);
		let n = 0;
		for (let y = 0; y < 64; y++) {
			for (let x = 0; x < 64; x++) {
				tileset[textureDataOffset++] = tile.data[n];
				n++;
			}
		}

		tile.inUse = true;

		tileIndex++;
		row++;
		if (row >= tilesPerRow) {
			row = 0;
			column++;
			if (column >= tilesPerCol) {
				column = 0;
				layer++;
			}
		}
	}

	perf();

	return [tileset, usedTextureLayers];
}
