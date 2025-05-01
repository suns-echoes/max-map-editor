import { MAP_LAYERS } from '^consts/map-consts.ts';
import { AppState } from '^state/app-state.ts';
import { Effect } from '^utils/reactive/effect.class.ts';
import { Perf } from '^utils/perf/perf.ts';


export async function loadMap(mapProject: MapProject, tiles: Tiles) {
	const map = parseMap(mapProject, tiles);
	AppState.mapSize.set({ width: mapProject.width, height: mapProject.height });
	AppState.map.set(map);
}


/**
 * The map is stored as 1D array of uint8 quadruples.
 * Each quadruple represents a cell in the map.
 * The first two bytes represent the tile XY position in texture.
 * The third byte represents the texture index where the tile is located.
 * The fourth byte represents the transformation of the tile.
 *
 * The transformation is as follows:
 * 0: N, 1: W, 2: S, 3: E, 4: !N, 5: !W, 6: !S, 7: !E
 */

function parseMap(mapProject: MapProject, tiles: Tiles) {
	const perf = Perf('parseMap');

	// TODO: add validation
	const mapSize = mapProject.width * mapProject.height * 4;
	const map = new Uint8Array(mapSize * MAP_LAYERS);

	let i = 0;
	for (let y = 0; y < mapProject.height; y++) {
		const row = mapProject.map[y];
		for (let x = 0; x < mapProject.width; x++) {
			const cell = row[x];
			if (Array.isArray(cell)) {
				for (let layer = 0; layer < cell.length; layer++) {
					populateMapCell(cell[layer], tiles, map, mapSize * layer + i);
				}
				for (let layer = cell.length; layer < MAP_LAYERS; layer++) {
					populateMapCell(null, tiles, map, mapSize * layer + i);
				}
			} else {
				populateMapCell(cell, tiles, map, i);
				for (let layer = 1; layer < MAP_LAYERS; layer++) {
					populateMapCell(null, tiles, map, mapSize * layer + i);
				}
			}
			i += 4;
		}
	}

	perf();

	return map;
}

function populateMapCell(cell: string | null, tiles: Tiles, map: Uint8Array, i: number) {
	if (!cell) {
		map[i++] = 0;
		map[i++] = 0;
		map[i++] = 0;
		map[i++] = 255;
		return;
	}

	const [tileId, transformation = 'N'] = cell.split(':') as [string, TileTransformation];
	const tile = tiles.get(tileId);

	if (!tile) {
		// throw new Error(`Tile not found: ${tileId}`);
		console.error(`Tile not found: ${tileId}`);
		map[i++] = 0;
		map[i++] = 0;
		map[i++] = 0;
		map[i++] = 0;
		return;
	}

	map[i++] = tile.location.textureX;
	map[i++] = tile.location.textureY;
	map[i++] = tile.location.textureIndex;
	map[i++] = transformMap[transformation];
}

const transformMap = { 'N': 0, 'W': 1, 'S': 2, 'E': 3, '!N': 4, '!W': 5, '!S': 6, '!E': 7 };


new Effect(function () {
	const wglMap = AppState.wglMap.value;
	const mapSize = AppState.mapSize.value;
	const map = AppState.map.value;
	if (!wglMap || !mapSize || !map) return;
	wglMap.initMap(map, mapSize.width, mapSize.height);
}).watch([AppState.wglMap, AppState.mapSize, AppState.map]);
