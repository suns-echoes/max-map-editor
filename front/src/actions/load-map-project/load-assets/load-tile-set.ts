import { resolveTextResource } from '^tauri-apps/api/path.ts';
import { readFile, readTextFile } from '^tauri-apps/plugin-fs.ts';
import { TILE_LENGTH } from '^consts/tile-consts.ts';
import { Perf } from '^lib/perf/perf.ts';


// TODO: Implement variants


export async function loadTileSet(outTiles: Tiles, assetName: string) {
	const [dataIndex, dataBin, match, props, /*variants*/] = await Promise.all([
		readTextFile(await resolveTextResource(`resources/assets/${assetName}/tiles-data.json`)),
		readFile(await resolveTextResource(`resources/assets/${assetName}/tiles-data.bin`)),
		readTextFile(await resolveTextResource(`resources/assets/${assetName}/tiles.match.json`)),
		readTextFile(await resolveTextResource(`resources/assets/${assetName}/tiles.props.json`)),
		// await readTextFile(await resolveResource(`resources/assets/${assetName}/tiles.variants.json`)),
	]);

	const tilesData = parseTilesData(dataIndex, dataBin);
	const tilesMatch = parseTilesMatch(match);
	const tilesProps = parseTilesProps(props);
	// const tilesVariants = parseTilesVariants(variants);

	buildTiles(outTiles, assetName, tilesData, tilesMatch, tilesProps, /*tilesVariants*/);
}


function parseTilesData(indexJson: string, dataBin: Uint8Array): TilesData {
	const perf = Perf('parseTilesData');

	// Parse index file (array of tile IDs in order)
	const tileIds = JSON.parse<string[]>(indexJson);
	const tilesData: TilesData = {};

	// Each tile is TILE_LENGTH bytes (64x64 = 4096)
	for (let i = 0; i < tileIds.length; i++) {
		const offset = i * TILE_LENGTH;
		tilesData[tileIds[i]] = dataBin.subarray(offset, offset + TILE_LENGTH);
	}

	perf();

	return tilesData;
}

function parseTilesMatch(match: string): TilesMatch {
	// TODO: Add validation
	return JSON.parse(match);
}

function parseTilesProps(props: string): TilesProps {
	// TODO: Add validation
	return JSON.parse(props);
}

// function parseTilesVariants(variants: string): TilesVariants {
// 	// TODO: Add validation
// 	return JSON.parse(variants);
// }


function buildTiles<T extends string>(outTiles: Tiles, assetName: string, data: TilesData<T>, match: TilesMatch<T>, props: TilesProps<T>, /*variants: TilesVariants*/) {
	const tileIds = Object.keys(data) as T[];
	for (let i = 0; i < tileIds.length; i++) {
		const tileId = tileIds[i];
		const tileData = data[tileId];
		const tileMatch = match[tileId];
		const tileProps = props[tileId];

		outTiles.set(tileId, {
			data: tileData,
			match: tileMatch,
			props: tileProps,
			transformation: 'N',
			variantsName: null,
			assetInfo: {
				assetName,
				tileId,
			},
			inUse: false,
			location: {
				dataOffset: 0,
				textureLayer: 2,
				textureX: 0,
				textureY: 0,
			},
		});
	}
}
