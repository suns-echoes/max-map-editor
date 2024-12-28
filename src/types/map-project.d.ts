declare type MapProject = {
	version: 0.1,
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
