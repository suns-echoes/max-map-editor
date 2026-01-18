import { resolveTextResource } from '^tauri-apps/api/path.ts';
import { readTextFile } from '^tauri-apps/plugin-fs.ts';
import { hexToUint8 } from '^lib/array-buffers/hex-to-uint8.ts';
import { Perf } from '^lib/perf/perf.ts';


/**
 * Load and parse a palette file.
 * Returns the palette as a Uint8Array (256 colors × 4 bytes RGBA).
 */
export async function loadPalette(assetName: string): Promise<Uint8Array> {
	const paletteJson = await readTextFile(
		await resolveTextResource(`resources/assets/${assetName}/palette.json`)
	);
	return parsePalette(paletteJson);
}


function parsePalette(paletteData: string): Uint8Array {
	const perf = Perf('parsePalette');

	// TODO: add validation
	const colors = JSON.parse<string[]>(paletteData).map((color: string) => hexToUint8(color.substring(1)));
	const palette = new Uint8Array(256 * 4);

	for (let i = 0, j = 0; i < 256; i++, j += 4) {
		const color = colors[i];
		palette[j] = color[0];
		palette[j + 1] = color[1];
		palette[j + 2] = color[2];
		palette[j + 3] = 255;
	}

	// First color is transparent
	palette[3] = 0;

	perf();

	return palette;
}
