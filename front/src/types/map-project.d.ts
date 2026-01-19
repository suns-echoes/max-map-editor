declare type MapProject = {
	version: string,
	name: string,
	description: string,
	width: number,
	height: number,
	use: {
		name: string,
		tileset?: boolean,
		palette?: boolean,
		version: number,
	}[],
	map: (string | string[])[],
};
