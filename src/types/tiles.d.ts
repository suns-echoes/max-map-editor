declare type Tiles = Map<string, Tile>;
declare type TilesData<T extends string = string> = Record<T, Uint8Array>;
declare type TilesMatch<T extends string = string> = Record<T, TileMatch>;
declare type TilesProps<T extends string = string> = Record<T, TileProps>;

declare type Tile = {
	data: Uint8Array,
	match: TileMatch,
	props: TileProps,
	transformation: TileTransformation,
	variantsName: string | null,
	assetInfo: TileAssetInfo,
	inUse: boolean,
	location: {
		dataOffset: number,
		textureIndex: number,
		textureX: number,
		textureY: number,
	},
};

declare type TileHexData = string;

declare type TileMatch = {
	N: string[],
	W: string[],
	S: string[],
	E: string[],
	'!N': string[],
	'!W': string[],
	'!S': string[],
	'!E': string[],
};

declare type TileProps = {
	"type": "water" | "shore" | "land" | "obstruction",
	"hasVariants": boolean,
	"useMaskColor": boolean,
	"transformable": boolean,
};

declare type TileTransformation = 'N' | 'W' | 'S' | 'E' | '!N' | '!W' | '!S' | '!E';

declare type TileAssetInfo = {
	assetName: string,
	tileId: string,
};
