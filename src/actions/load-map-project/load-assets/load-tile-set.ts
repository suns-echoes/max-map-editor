import { resolveResource } from '^tauri-apps/api/path.ts';
import { readTextFile } from '^tauri-apps/plugin-fs.ts';
import { hexToUint8 } from '^utils/array-buffers/hex-to-uint8.ts';
import { Perf } from '^utils/perf/perf.ts';


// TODO: Implement variants


export async function loadTileSet(outTiles: Tiles, assetName: string) {
	const [data, match, props, /*variants*/] = await Promise.all([
		await readTextFile(await resolveResource(`resources/assets/${assetName}/tiles.data.json`)),
		await readTextFile(await resolveResource(`resources/assets/${assetName}/tiles.match.json`)),
		await readTextFile(await resolveResource(`resources/assets/${assetName}/tiles.props.json`)),
		// await readTextFile(await resolveResource(`resources/assets/${assetName}/tiles.variants.json`)),
	]);

	const tilesData = parseTilesData(data);
	const tilesMatch = parseTilesMatch(match);
	const tilesProps = parseTilesProps(props);
	// const tilesVariants = parseTilesVariants(variants);

	buildTiles(outTiles, assetName, tilesData, tilesMatch, tilesProps, /*tilesVariants*/);
}


function parseTilesData(data: string): TilesData {
	const perf = Perf('parseTilesData');

	// TODO: Add validation
	const tilesData = JSON.parse<Record<string, any>>(data);
	Object.keys(tilesData).forEach((tileId) => {
		tilesData[tileId] = hexToUint8(tilesData[tileId]);
	});

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
				textureIndex: 2,
				textureX: 0,
				textureY: 0,
			},
		});
	}
}
