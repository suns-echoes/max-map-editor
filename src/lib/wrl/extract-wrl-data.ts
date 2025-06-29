import { ArrayBufferReader } from '^lib/array-buffers/array-buffer-reader.ts';


export function extractWrlData(wrl: Uint8Array): WRLData {
	const reader = new ArrayBufferReader(wrl.buffer);

	reader.seek(5); // Skip header.

	const width = reader.readUInt16LE();
	const height = reader.readUInt16LE();
	const minimap = reader.readUint8Array(width * height);
	const bigmap = reader.readUint8Array(width * height * 2);
	const tileCount = reader.readUInt16LE();
	const tiles = new Array(tileCount);
	for (let i = 0; i < tileCount; i++) tiles[i] = reader.readUint8Array(64 * 64);

	const palette = reader.readUint8Array(256 * 3);
	const passtab = reader.readUint8Array(tileCount);

	return {
		width,
		height,
		minimap,
		bigmap,
		tileCount,
		tiles,
		palette,
		passtab,
	};
}


interface WRLData {
	width: number;
	height: number;
	minimap: Uint8Array;
	bigmap: Uint8Array;
	tileCount: number;
	tiles: Uint8Array[];
	palette: Uint8Array;
	passtab: Uint8Array;
}
