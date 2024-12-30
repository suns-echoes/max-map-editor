import { effect } from '^utils/reactive/effect.ts';
import { Perf } from '^utils/perf/perf.ts';
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


effect.onNonNullValues([AppState.wglMap, AppState.mapProject, AppState.tiles], function ([wglMap, mapProject, tiles]) {
	const tileset = arrangeTilesData(tiles, wglMap.getTileCapability());
	wglMap.initTilesets([tileset]);
	loadMap(mapProject, tiles);
});


function arrangeTilesData(tiles: Tiles, tileCapability: WglTileCapability): Uint8Array {
	const perf = Perf('arrangeTilesData');

	const { maxTileTextures, maxTilesPerTexture, tilesPerRow, tilesPerCol } = tileCapability;

	const tileset = new Uint8Array(64 ** 2 * maxTilesPerTexture * 4);

	let tileIndex = 0;
	let row = 0;
	let column = 0;
	let textureDataOffset = 0;
	let textureUnit = 0;
	for (const tile of tiles.values()) {
		tile.location.textureIndex = 0;
		tile.location.textureX = row;
		tile.location.textureY = column;

		// tileset.data.set(tile.data, 64 ** 2 * tileIndex);
		let n = 0;
		let w = 64 * 63;
		let s = 64 ** 2 - 1;
		let e = 63;
		for (let y = 0; y < 64; y++) {
			for (let x = 0; x < 64; x++) {
				// N
				tileset[textureDataOffset++] = tile.data[n];
				n++;
				// W
				tileset[textureDataOffset++] = tile.data[w];
				w -= 64;
				// S
				tileset[textureDataOffset++] = tile.data[s];
				s--;
				// E
				tileset[textureDataOffset++] = tile.data[e];
				e += 64;
			}
			w += 64 ** 2 + 1;
			e -= 64 ** 2 + 1;
		}

		tile.inUse = true;

		tileIndex++;
		row++;
		if (row >= tilesPerRow) {
			row = 0;
			column++;
			if (column >= tilesPerCol) {
				column = 0;
				textureUnit++;
				textureDataOffset = 0;

				throw new Error('Too many tiles for one texture, implement more!');

				if (textureUnit >= maxTileTextures) {
					throw new Error('Too many tiles');
				}
			}
		}
	}

	perf();

	return tileset;
}
