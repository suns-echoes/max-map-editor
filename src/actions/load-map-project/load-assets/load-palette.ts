import { resolveResource } from '^tauri-apps/api/path.ts';
import { readTextFile } from '^tauri-apps/plugin-fs.ts';
import { AppState } from '^state/app-state.ts';
import { hexToUint8 } from '^utils/array-buffers/hex-to-uint8.ts';
import { Perf } from '^utils/perf/perf.ts';
import { effect } from '^utils/reactive/effect.ts';


export async function loadPalette(assetName: string) {
	const palette = parsePalette(await readTextFile(await resolveResource(`resources/assets/${assetName}/palette.json`)));
	AppState.palette.set(palette);
}


function parsePalette(paletteData: string) {
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

	palette[3] = 0;

	perf();

	return palette;
}


effect.onNonNullValues([AppState.wglMap, AppState.palette], function ([wglMap, palette]) {
	wglMap.initPalette(palette);
});
