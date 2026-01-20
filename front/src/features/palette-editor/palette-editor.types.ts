/**
 * Palette Editor Types
 *
 * Types and helpers for the palette editor feature.
 */


// ============================================================================
// Types
// ============================================================================

/**
 * RGB color
 */
export interface RgbColor {
	r: number;
	g: number;
	b: number;
}

/**
 * A change to a palette color
 */
export interface PaletteChange {
	index: number;
	oldColor: RgbColor;
	newColor: RgbColor;
}

/**
 * Undo data for palette changes
 */
export interface PaletteUndoData {
	changes: PaletteChange[];
}

/**
 * Palette color info with usage data
 */
export interface PaletteColorInfo {
	index: number;
	color: RgbColor;
	usageCount: number;  // How many pixels use this color
	tileCount: number;   // How many tiles use this color
}


// ============================================================================
// Color Conversion Helpers
// ============================================================================

/**
 * Convert RGB to hex string (#RRGGBB)
 */
export function rgbToHex(r: number, g: number, b: number): string {
	return `#${r.toString(16).padStart(2, '0')}${g.toString(16).padStart(2, '0')}${b.toString(16).padStart(2, '0')}`;
}

/**
 * Convert hex string to RGB
 */
export function hexToRgb(hex: string): RgbColor {
	const value = hex.replace('#', '');
	return {
		r: parseInt(value.slice(0, 2), 16),
		g: parseInt(value.slice(2, 4), 16),
		b: parseInt(value.slice(4, 6), 16),
	};
}

/**
 * Get RGB color from palette at index
 */
export function getPaletteColor(palette: Uint8Array, index: number): RgbColor {
	const offset = index * 3;
	return {
		r: palette[offset],
		g: palette[offset + 1],
		b: palette[offset + 2],
	};
}

/**
 * Set RGB color in palette at index
 */
export function setPaletteColor(palette: Uint8Array, index: number, color: RgbColor): void {
	const offset = index * 3;
	palette[offset] = color.r;
	palette[offset + 1] = color.g;
	palette[offset + 2] = color.b;
}

/**
 * Compare two colors for equality
 */
export function colorsEqual(a: RgbColor, b: RgbColor): boolean {
	return a.r === b.r && a.g === b.g && a.b === b.b;
}

/**
 * Calculate color distance (Euclidean in RGB space)
 */
export function colorDistance(a: RgbColor, b: RgbColor): number {
	const dr = a.r - b.r;
	const dg = a.g - b.g;
	const db = a.b - b.b;
	return Math.sqrt(dr * dr + dg * dg + db * db);
}

/**
 * Blend two colors
 */
export function blendColors(a: RgbColor, b: RgbColor, t: number): RgbColor {
	return {
		r: Math.round(a.r + (b.r - a.r) * t),
		g: Math.round(a.g + (b.g - a.g) * t),
		b: Math.round(a.b + (b.b - a.b) * t),
	};
}

/**
 * Adjust brightness of a color
 */
export function adjustBrightness(color: RgbColor, factor: number): RgbColor {
	return {
		r: Math.min(255, Math.max(0, Math.round(color.r * factor))),
		g: Math.min(255, Math.max(0, Math.round(color.g * factor))),
		b: Math.min(255, Math.max(0, Math.round(color.b * factor))),
	};
}
