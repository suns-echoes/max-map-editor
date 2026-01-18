import { MAP_LAYERS } from '^consts/map-consts.ts';
import { Perf } from '^lib/perf/perf.ts';


/**
 * Parse map data into GPU-ready format.
 * Returns a Uint16Array with tile locations and transformations.
 *
 * The map is stored as 1D array of uint16 quadruples per layer.
 * Each quadruple represents a cell in the map:
 * - First two values: tile XY position in texture
 * - Third value: texture layer index
 * - Fourth value: transformation (0-7)
 *
 * Transformations: N=0, W=1, S=2, E=3, !N=4, !E=5, !S=6, !W=7
 */
export function parseMap(mapProject: MapProject, tiles: Tiles): Uint16Array {
	const perf = Perf('parseMap');

	const mapSize = mapProject.width * mapProject.height * 4;
	const map = new Uint16Array(mapSize * MAP_LAYERS);

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


function populateMapCell(cell: string | null, tiles: Tiles, map: Uint16Array, i: number): void {
	if (!cell) {
		map[i] = 0;
		map[i + 1] = 0;
		map[i + 2] = 0;
		map[i + 3] = 255; // Empty cell marker
		return;
	}

	const [tileId, transformation = 'N'] = cell.split(':') as [string, TileTransformation];
	const tile = tiles.get(tileId);

	if (!tile) {
		throw new Error(`Tile not found: ${tileId}`);
	}

	map[i] = tile.location.textureX;
	map[i + 1] = tile.location.textureY;
	map[i + 2] = tile.location.textureLayer;
	map[i + 3] = TRANSFORM_MAP[transformation];
}


const TRANSFORM_MAP = {
	'N': 0x00,
	'W': 0x01,
	'S': 0x02,
	'E': 0x03,
	'!N': 0x04,
	'!E': 0x05,
	'!S': 0x06,
	'!W': 0x07,
} as const;
