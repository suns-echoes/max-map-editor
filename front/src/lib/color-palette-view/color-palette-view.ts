/**
 * A view over a color palette that provides methods to get colors in different formats,
 * with RGBA caching for performance.
 */
export class ColorPaletteView {
	public constructor(palette: Uint8Array) {
		this._palette = palette;
		this._paletteRGBA = new Uint8Array(256 * 4);
		for (let i = 0, j = 0; i < 256 * 3; i += 3, j += 4) {
			this._paletteRGBA[j] = palette[i];
			this._paletteRGBA[j + 1] = palette[i + 1];
			this._paletteRGBA[j + 2] = palette[i + 2];
			this._paletteRGBA[j + 3] = 255;
		}
	}

	public getRGB(colorIndex: number): Uint8Array {
		if (colorIndex < 0 || colorIndex > 255) {
			throw new Error('Invalid color index, must be between 0 and 255');
		};
		const colorOffset = colorIndex * 4;
		return this._palette.subarray(colorOffset, colorOffset + 3);
	}

	public getRGBA(colorIndex: number): Uint8Array {
		if (colorIndex < 0 || colorIndex > 255) {
			throw new Error('Invalid color index, must be between 0 and 255');
		};
		const colorOffset = colorIndex * 4;
		return this._palette.subarray(colorOffset, colorOffset + 4);
	}

	public getCSSColor(colorIndex: number): string {
		if (colorIndex < 0 || colorIndex > 255) {
			throw new Error('Invalid color index, must be between 0 and 255');
		};
		const rgba = this.getRGB(colorIndex);
		return `#${this._toHex(rgba![0])}${this._toHex(rgba![1])}${this._toHex(rgba![2])}`;
	}

	private _palette: Uint8Array;
	private _paletteRGBA: Uint8Array;

	private _toHex(value: number): string {
		return value.toString(16).padStart(2, '0').toUpperCase();
	}
}
