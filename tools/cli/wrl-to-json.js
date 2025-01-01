#!/usr/bin/env node

//
// ▰▰▰ CLI HELP ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰
//

import { CliLog } from '../_common/node/cli-log.js';

const cliLog = new CliLog();

if (process.argv[2] === '-h' || process.argv[2] === '--help') {
	cliLog.bgYellow().black()
		.ln('')
		.ln('')
		.ln(' ▟██▙  ██▙           ▟██▘      ▜█▙   ▟█▛')
		.ln('  ███▙ ███▙         ▟███▙       ▜█▙ ▟█▛')
		.ln('  ██▜█▙██▜█▙       ▟█▛ ▜█▙       🬸███🬴')
		.ln('  ██ ▜███ ▜█▙ ▜█▛ ▟███████▙ ▜█▛ ▟█▛ ▜█▙ ▜█▛')
		.ln('  ██  ▜██  ▜█▙ ▀ ▟█▛     ▜█▙ ▀ ▟█▛   ▜█▙ ▀')
		.ln('', true)
		.ln('')
		.bold().green().ln('WRL to JSON converter and extractor (v 1.0.0)', true)
		.ln('')
		.ln('Convert WRL map file into JSON format file.')
		.ln('')
		.ln('Usage:')
		.ln('    ./wrl-to-json.js input_file [output_file] [options]')
		.ln('')
		.ln('Arguments:')
		.ln('    options     - see options list below')
		.ln('    input_file  - path to the WRL map file')
		.ln('    output_file - optional path for JSON output file')
		.ln('                  default is: input_file.json')
		.ln('')
		.ln('Options:')
		.ln('	-e, --export  - export all resources to binary files')
		.ln('	-p, --preview - export all images to bitmap files')
		.ln('')
		.ln('')
		.lightGreen('origin: ').underline('https://github.com/suns-echoes/max-game-map-editor')
		.ln('')
		.reset().log();

	process.exit(0);
}


// ▰▰▰ JAVASCRIPT HELPERS ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰
//

Uint8Array.prototype.toJSON = function () {
	return Buffer.from(this).toString('hex');
};

Uint16Array.prototype.toJSON = function () {
	return Buffer.from(this).toString('hex');
};


//
// ▰▰▰ INIT PROGRAM ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰
//

import { Buffer } from 'node:buffer';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { BufferReader } from '../_common/node/buffer-reader.class.js';
import { Bitmap } from '../_common/node/bitmap.class.js';

const argv = process.argv.slice(2);
let $;

const extractBinaryChunks = (
	$ = argv.indexOf('-e'),
	$ === -1 && ($ = argv.indexOf('--extract')),
	$ === -1 ? false : !!argv.splice($, 1)
);

const extractPreviewBitmaps = (
	$ = argv.indexOf('-p'),
	$ === -1 && ($ = argv.indexOf('--preview')),
	$ === -1 ? false : !!argv.splice($, 1)
);

const inputFilePath = argv[0];
const outputFilePath = argv[1] ?? inputFilePath + '.json';


/**
 * 1. load all WRL files and extract data
 *
 * The WRL file structure:
 *
 *  Content   | Size in bytes       | Data type
 * -----------+---------------------+-----------
 *  header    | 5                   | uint8
 *  width     | 2                   | uint16
 *  height    | 2                   | uint16
 *  minimap   | width * height      | uint8
 *  bigmap    | width * height * 2  | uint8
 *  tileCount | 2                   | uint16
 *  tiles     | 64 * 64 * tileCount | uint8
 *  palette   | 256 * 3             | uint8
 *  passtab   | tileCount           | uint8
 */

/**
 * @param {string} filePath
 */
function readWRLFile(filePath) {
	/** The buffer with original WRL file. */
	const wrl = readFileSync(filePath);

	/**
	 * UNPACKING SECTION
	 */
	const buf = new BufferReader(wrl);

	const headerBuffer = buf.readBytes(5, 'header');
	const header = new Uint8Array(headerBuffer, headerBuffer.byteOffset, headerBuffer.byteLength);

	const width = buf.readUInt16LE('width');

	const height = buf.readUInt16LE('height');

	const minimapBuffer = buf.readBytes(width * height, 'minimap');
	const minimap = new Uint8Array(minimapBuffer, minimapBuffer.byteOffset, minimapBuffer.byteLength);

	const bigmapBuffer = buf.readBytes(width * height * 2, 'bigmap');
	const bigmap = new Uint16Array(bigmapBuffer, bigmapBuffer.byteOffset, bigmapBuffer.byteLength / Uint16Array.BYTES_PER_ELEMENT);

	const tileCount = buf.readUInt16LE('tileCount');

	const tiles = new Array(tileCount);
	for (let i = 0; i < tileCount; i++) {
		const tileBuffer = buf.readBytes(64 * 64, `tile[${i}]`);
		tiles[i] = new Uint8Array(tileBuffer, tileBuffer.byteOffset, tileBuffer.byteLength);
	}

	const paletteBuffer = buf.readBytes(256 * 3, 'palette');
	const palette = new Uint8Array(paletteBuffer, paletteBuffer.byteOffset, paletteBuffer.byteLength);

	const passtabBuffer = buf.readBytes(tileCount, 'passtab');
	const passtab = new Uint8Array(passtabBuffer, passtabBuffer.byteOffset, passtabBuffer.byteLength);;

	return {
		header,
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

/**
 * @param {string} filePath
 */
function writeJSONFile(filePath, data) {
	mkdirSync(dirname(outputFilePath), { recursive: true });

	writeFileSync(filePath, JSON.stringify(data, null, '\t'), { encoding: 'utf8' });
}


//
// ▰▰▰ MAIN PROGRAM ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰
//

const wrlData = readWRLFile(inputFilePath);

writeJSONFile(outputFilePath, wrlData);

/**
 * Write binary files.
 */
if (extractBinaryChunks) {
	const outDir = join(dirname(outputFilePath), 'binary');
	mkdirSync(outDir, { recursive: true });
	const tileOutDir = join(dirname(outputFilePath), 'binary/tiles');
	mkdirSync(tileOutDir, { recursive: true });

	writeFileSync(join(outDir, 'header.bin'), wrlData.header);

	writeFileSync(join(outDir, 'minimap.bin'), wrlData.minimap);

	writeFileSync(join(outDir, 'bigmap.bin'), wrlData.bigmap);

	writeFileSync(join(outDir, 'palette.bin'), wrlData.palette);

	writeFileSync(join(outDir, 'tiles.bin'), new Uint16Array([wrlData.tileCount]));

	for (let i = 0; i < wrlData.tileCount; i++)
		writeFileSync(join(tileOutDir, `tile[${i.toString(10).padStart(4, '0')}].bin`), wrlData.tiles[i]);

	writeFileSync(join(outDir, 'palette.bin'), wrlData.palette);

	writeFileSync(join(outDir, 'passtab.bin'), wrlData.passtab);
}

if (extractPreviewBitmaps) {
	const outDir = join(dirname(outputFilePath), 'preview');
	mkdirSync(outDir, { recursive: true });
	const tileOutDir = join(dirname(outputFilePath), 'preview/tiles');
	mkdirSync(tileOutDir, { recursive: true });

	/**
	 * PALETTE PREVIEW GENERATOR
	 */
	{
		const paletteWidth = 16;
		const paletteHeight = 16;
		const paletteColorPixelSize = 16;
		const paletteColorPixelCount = paletteColorPixelSize * paletteColorPixelSize;
		const pixelBuffer = Buffer.alloc(256 * paletteColorPixelCount * 4, 128);
		for (let colorY = 0; colorY < paletteHeight; colorY++) {
			for (let colorX = 0; colorX < paletteWidth; colorX++) {
				const offsetX = colorX * paletteColorPixelSize;
				const offsetY = colorY * paletteColorPixelSize;
				const paletteColorIndex = (colorY * paletteWidth + colorX) * 3;
				const color = wrlData.palette[paletteColorIndex] * 0x10000
					+ wrlData.palette[paletteColorIndex + 1] * 0x100
					+ wrlData.palette[paletteColorIndex + 2]
					+ 0xff000000;
				for (let y = 0; y < paletteColorPixelSize; y++) {
					for (let x = 0; x < paletteColorPixelSize; x++) {
						const pixelOffset = ((paletteWidth * paletteColorPixelSize) * (offsetY + y) + (offsetX + x)) * 4;
						pixelBuffer.writeUInt32LE(color, pixelOffset);
					}
				}
			}
		}
		const palettePreview = new Bitmap(pixelBuffer, 16 * 16, 16 * 16);
		palettePreview.save(join(outDir, 'palette.bmp'));
		palettePreview.destroy();
	}

	/**
	 * MINIMAP PREVIEW GENERATOR
	 */
	{
		const pixelBuffer = Buffer.alloc(wrlData.width * wrlData.height * 4);
		for (let pixelIndex = 0; pixelIndex < wrlData.width * wrlData.height; pixelIndex++) {
			const offset = pixelIndex * 4;
			const paletteColorIndex = wrlData.minimap[pixelIndex] * 3;
			const color = wrlData.palette[paletteColorIndex] * 0x10000
				+ wrlData.palette[paletteColorIndex + 1] * 0x100
				+ wrlData.palette[paletteColorIndex + 2]
				+ 0xff000000;
			pixelBuffer.writeUInt32LE(color, offset);
		}
		const tilesPreview = new Bitmap(pixelBuffer, wrlData.width, wrlData.height);
		tilesPreview.save(join(outDir, 'minimap.bmp'));
		tilesPreview.destroy();
	}

	/**
	 * TILES PREVIEW GENERATOR
	 */
	{
		const outputTilesBitmaps = true;

		const tileSize = 64;
		const tilePixelCount = tileSize * tileSize;
		const pixelBuffer = Buffer.alloc(tilePixelCount * 4 * wrlData.tileCount);
		for (let tileIndex = 0; tileIndex < wrlData.tileCount; tileIndex++) {
			const tilePixelBuffer = outputTilesBitmaps ? new Uint32Array(64 * 64) : null;
			const offset = tileIndex * tilePixelCount * 4;
			const tile = wrlData.tiles[tileIndex];
			for (let i = 0; i < tilePixelCount; i++) {
				const paletteColorIndex = tile[i] * 3;
				const color = wrlData.palette[paletteColorIndex] * 0x10000
					+ wrlData.palette[paletteColorIndex + 1] * 0x100
					+ wrlData.palette[paletteColorIndex + 2]
					+ 0xff000000;
				const pixelOffset = offset + i * 4;
				pixelBuffer.writeUInt32LE(color, pixelOffset);
				if (outputTilesBitmaps) tilePixelBuffer[i] = color;
			}
			if (outputTilesBitmaps) {
				const tilesPreview = new Bitmap(Buffer.from(tilePixelBuffer.buffer), 64, 64);
				tilesPreview.save(join(tileOutDir, `tile[${tileIndex.toString(10).padStart(4, '0')}].bmp`));
				tilesPreview.destroy();
			}
		}
		const tilesPreview = new Bitmap(pixelBuffer, tileSize, tileSize * wrlData.tileCount);
		tilesPreview.save(join(outDir, 'tiles.bmp'));
		tilesPreview.destroy();
	}
}
