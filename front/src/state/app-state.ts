import type { WglMap } from '^src/ui/main-window/wgl-map/wgl/wgl-map';
import { Value } from '^reactive/value.ts';
import { Effect } from '^reactive/effect.ts';
import { Perf } from '^lib/perf/perf.ts';
import { TILE_LENGTH } from '^consts/tile-consts.ts';


export const AppState = {
	mapProject: new Value<MapProject | null>(null),
	palette: new Value<Uint8Array | null>(null),
	map: new Value<Uint16Array | null>(null),
	tiles: new Value<Tiles | null>(null),

	wglMap: new Value<WglMap | null>(null),

	reset() {
		this.mapProject.set(null);
		this.palette.set(null);
		this.map.set(null);
		this.tiles.set(null);
	}
};


// ============================================================================
// WebGL Map Initialization Effects
// These Effects initialize the WebGL renderer when both wglMap and data are ready.
// They handle the case where wglMap may be created before or after data loads.
// ============================================================================

/**
 * Initialize palette in WebGL when wglMap + palette are ready.
 */
new Effect(function initWglPalette() {
	const wglMap = AppState.wglMap.value;
	const palette = AppState.palette.value;
	if (!wglMap || !palette) return;

	wglMap.initPalette(palette);
}, { strong: true }).on([AppState.wglMap, AppState.palette]);


/**
 * Initialize tilesets in WebGL when wglMap + tiles are ready.
 */
new Effect(function initWglTilesets() {
	const wglMap = AppState.wglMap.value;
	const tiles = AppState.tiles.value;
	if (!wglMap || !tiles) return;

	const [tileset, layers] = arrangeTilesData(tiles, wglMap.getTileCapability());
	wglMap.initTilesets(tileset, layers);
}, { strong: true }).on([AppState.wglMap, AppState.tiles]);


/**
 * Initialize map in WebGL when wglMap + mapProject + map are ready.
 */
new Effect(function initWglMap() {
	const wglMap = AppState.wglMap.value;
	const mapProject = AppState.mapProject.value;
	const map = AppState.map.value;
	if (!wglMap || !mapProject || !map) return;

	wglMap.initMap(map, mapProject.width, mapProject.height);
}, { strong: true }).on([AppState.wglMap, AppState.mapProject, AppState.map]);


/**
 * Arrange tile data into GPU-ready texture format.
 */
export function arrangeTilesData(tiles: Tiles, tileCapability: WglTileCapability): [tileset: Uint8Array, layers: number] {
	const perf = Perf('arrangeTilesData');

	const { maxTextureSize, maxTilesPerTextureLayer, tilesPerRow, tilesPerCol } = tileCapability;
	const usedTextureLayers = Math.ceil(tiles.size / maxTilesPerTextureLayer);

	if (usedTextureLayers > tileCapability.maxTextureLayers) {
		throw new Error('Fatal: Too many tiles for the available texture layers');
	}

	const tileset = new Uint8Array(maxTextureSize ** 2 * usedTextureLayers);

	let layer = 0;
	let row = 0;
	let column = 0;
	let textureDataOffset = 0;

	for (const tile of tiles.values()) {
		tile.location.textureLayer = layer;
		tile.location.textureX = row;
		tile.location.textureY = column;

		// Copy tile data to tileset
		for (let i = 0; i < TILE_LENGTH; i++) {
			tileset[textureDataOffset++] = tile.data[i];
		}

		tile.inUse = true;

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
