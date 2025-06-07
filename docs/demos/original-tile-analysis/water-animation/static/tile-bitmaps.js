/**
 * Creates an array of ImageBitmap objects, each representing the original tile
 * recolored with the palette shifted by a cycling offset.
 *
 * @param {Uint8ClampedArray} tileData - The pixel data of the tile (RGBA per pixel).
 * @param {Array<Array<number>>} palette - Array of [r, g, b, a] colors.
 * @param {number} paletteOffset - The offset to apply to the palette indices to align it with original palette.
 * @param {boolean} unEvenOdd - If true, will fill out of bounds indices with adjacent color, otherwise will leave them transparent.
 * @returns {Promise<Array<ImageBitmap>>} - Promise resolving to array of ImageBitmap objects.
 */
async function createPaletteCycledBitmaps(tileData, palette, paletteOffset, unEvenOdd, mask) {
	const tileWidth = 64; 
	const tileHeight = 64;
	const bitmaps = [];
	const paletteSize = palette.length;
	const paletteColors = paletteSize / 3;

	for (let offset = 0; offset < paletteColors; offset++) {
		// Create a new ImageData for this offset
		const newImageData = new Uint8ClampedArray(tileData.length * 4);

		const outOfBoundsIndices = new Set();

		for (let i = 0; i < tileData.length; i++) {
			// Assume the original tileData uses palette indices in the red channel
			let paletteIndex = tileData[i] + paletteOffset;
			if ((
				(mask === 'maskEven' && (i + Math.floor(i / 64) % 2) % 2 !== 0) ||
				(mask === 'maskOdd' && (i + Math.floor(i / 64) % 2) % 2 !== 1)
			)) {
				// Skip odd indices if maskEven is true
				paletteIndex = -1; // Set to -1 to indicate out of bounds
			}
			if (paletteIndex < 0 || paletteIndex >= paletteColors) {
				if (!unEvenOdd) continue; // Skip if out of bounds and unEvenOdd is false
				if (outOfBoundsIndices.has(i - 1)) {
					// If we've already processed this index, skip it
					continue;
				}
				// get adjacent color if out of bounds
				const adjacentColor = {
					r: newImageData[(i - 1) * 4],
					g: newImageData[(i - 1) * 4 + 1],
					b: newImageData[(i - 1) * 4 + 2],
					a: newImageData[(i - 1) * 4 + 3],
				}
				newImageData[i * 4] = adjacentColor.r;
				newImageData[i * 4 + 1] = adjacentColor.g;
				newImageData[i * 4 + 2] = adjacentColor.b;
				newImageData[i * 4 + 3] = adjacentColor.a;
				outOfBoundsIndices.add(i);

				// throw new Error("Palette index out of bounds");
				// newImageData[i * 4] = 0;
				continue;
			}
			const cycledIndex = (paletteIndex + offset) % paletteColors;
			const { r, g, b, a } = palette.getColor(cycledIndex);

			newImageData[i * 4] = r;
			newImageData[i * 4 + 1] = g;
			newImageData[i * 4 + 2] = b;
			newImageData[i * 4 + 3] = a;
		}

		// Create ImageData and ImageBitmap
		const imageData = new ImageData(newImageData, tileWidth, tileHeight);
		const bitmap = await createImageBitmap(imageData);
		bitmaps.push(bitmap);
	}

	return bitmaps;
}

function destroyBitmaps(bitmaps) {
	for (const bitmap of bitmaps) {
		if (bitmap.close) {
			bitmap.close();
		}
	}
}


var bitmaps = {
	'1': {
		evenOdd: {
			normalPalette: {
				noMask: [],
				maskEven: [],
				maskOdd: [],
			},
			altPalette: {
				noMask: [],
				maskEven: [],
				maskOdd: [],
			},
			blackPalette: {
				noMask: [],
				maskEven: [],
				maskOdd: [],
			},
		},
		unEvenOdd: {
			normalPalette: {
				noMask: [],
				maskEven: [],
				maskOdd: [],
			},
			altPalette: {
				noMask: [],
				maskEven: [],
				maskOdd: [],
			},
			blackPalette: {
				noMask: [],
				maskEven: [],
				maskOdd: [],
			},
		},
	},
	'2': {
		evenOdd: {
			normalPalette: {
				noMask: [],
				maskEven: [],
				maskOdd: [],
			},
			altPalette: {
				noMask: [],
				maskEven: [],
				maskOdd: [],
			},
			blackPalette: {
				noMask: [],
				maskEven: [],
				maskOdd: [],
			},
		},
		unEvenOdd: {
			normalPalette: {
				noMask: [],
				maskEven: [],
				maskOdd: [],
			},
			altPalette: {
				noMask: [],
				maskEven: [],
				maskOdd: [],
			},
		},
	}
};

(async () => {
	for (const tile of tiles) {
		bitmaps[1].evenOdd.normalPalette.noMask.push(await createPaletteCycledBitmaps(tile, water1Palette, -water1SubPaletteStart, false, 'noMask'));
		bitmaps[1].evenOdd.normalPalette.maskEven.push(await createPaletteCycledBitmaps(tile, water1Palette, -water1SubPaletteStart, false, 'maskEven'));
		bitmaps[1].evenOdd.normalPalette.maskOdd.push(await createPaletteCycledBitmaps(tile, water1Palette, -water1SubPaletteStart, false, 'maskOdd'));
		bitmaps[1].evenOdd.altPalette.noMask.push(await createPaletteCycledBitmaps(tile, water1AltPalette, -water1SubPaletteStart, false, 'noMask'));
		bitmaps[1].evenOdd.altPalette.maskEven.push(await createPaletteCycledBitmaps(tile, water1AltPalette, -water1SubPaletteStart, false, 'maskEven'));
		bitmaps[1].evenOdd.altPalette.maskOdd.push(await createPaletteCycledBitmaps(tile, water1AltPalette, -water1SubPaletteStart, false, 'maskOdd'));
		bitmaps[1].unEvenOdd.normalPalette.noMask.push(await createPaletteCycledBitmaps(tile, water1Palette, -water1SubPaletteStart, true, 'noMask'));
		bitmaps[1].unEvenOdd.normalPalette.maskEven.push(await createPaletteCycledBitmaps(tile, water1Palette, -water1SubPaletteStart, true, 'maskEven'));
		bitmaps[1].unEvenOdd.normalPalette.maskOdd.push(await createPaletteCycledBitmaps(tile, water1Palette, -water1SubPaletteStart, true, 'maskOdd'));
		bitmaps[1].unEvenOdd.altPalette.noMask.push(await createPaletteCycledBitmaps(tile, water1AltPalette, -water1SubPaletteStart, true, 'noMask'));
		bitmaps[1].unEvenOdd.altPalette.maskEven.push(await createPaletteCycledBitmaps(tile, water1AltPalette, -water1SubPaletteStart, true, 'maskEven'));
		bitmaps[1].unEvenOdd.altPalette.maskOdd.push(await createPaletteCycledBitmaps(tile, water1AltPalette, -water1SubPaletteStart, true, 'maskOdd'));
		
		bitmaps[2].evenOdd.normalPalette.noMask.push(await createPaletteCycledBitmaps(tile, water2Palette, -water2SubPaletteStart, false, 'noMask'));
		bitmaps[2].evenOdd.normalPalette.maskEven.push(await createPaletteCycledBitmaps(tile, water2Palette, -water2SubPaletteStart, false, 'maskEven'));
		bitmaps[2].evenOdd.normalPalette.maskOdd.push(await createPaletteCycledBitmaps(tile, water2Palette, -water2SubPaletteStart, false, 'maskOdd'));
		bitmaps[2].evenOdd.altPalette.noMask.push(await createPaletteCycledBitmaps(tile, water2AltPalette, -water2SubPaletteStart, false, 'noMask'));
		bitmaps[2].evenOdd.altPalette.maskEven.push(await createPaletteCycledBitmaps(tile, water2AltPalette, -water2SubPaletteStart, false, 'maskEven'));
		bitmaps[2].evenOdd.altPalette.maskOdd.push(await createPaletteCycledBitmaps(tile, water2AltPalette, -water2SubPaletteStart, false, 'maskOdd'));
		bitmaps[2].unEvenOdd.normalPalette.noMask.push(await createPaletteCycledBitmaps(tile, water2Palette, -water2SubPaletteStart, true, 'noMask'));
		bitmaps[2].unEvenOdd.normalPalette.maskEven.push(await createPaletteCycledBitmaps(tile, water2Palette, -water2SubPaletteStart, true, 'maskEven'));
		bitmaps[2].unEvenOdd.normalPalette.maskOdd.push(await createPaletteCycledBitmaps(tile, water2Palette, -water2SubPaletteStart, true, 'maskOdd'));
		bitmaps[2].unEvenOdd.altPalette.noMask.push(await createPaletteCycledBitmaps(tile, water2AltPalette, -water2SubPaletteStart, true, 'noMask'));
		bitmaps[2].unEvenOdd.altPalette.maskEven.push(await createPaletteCycledBitmaps(tile, water2AltPalette, -water2SubPaletteStart, true, 'maskEven'));
		bitmaps[2].unEvenOdd.altPalette.maskOdd.push(await createPaletteCycledBitmaps(tile, water2AltPalette, -water2SubPaletteStart, true, 'maskOdd'));
		
	}

	state.ready.resolve();
})();
