/**
 * Pixel Editor Types
 *
 * Types and constants for pixel-level tile editing.
 */


// ============================================================================
// Constants
// ============================================================================

/** Tile dimensions */
export const TILE_SIZE = 64;
export const TILE_PIXELS = TILE_SIZE * TILE_SIZE;


// ============================================================================
// Types
// ============================================================================

/** Pixel editor tool types */
export type PixelTool = 'picker' | 'pencil' | 'replace';

/** Scope for color replace operation */
export type ReplaceScope = 'tile' | 'selected' | 'all';

/** A single pixel change */
export interface PixelChange {
	tileId: string;
	pixelIndex: number;
	oldColor: number;  // palette index
	newColor: number;  // palette index
}

/** Undo data for pixel operations */
export interface PixelUndoData {
	changes: PixelChange[];
}

/** Color replace operation parameters */
export interface ColorReplaceParams {
	fromColor: number;  // palette index to replace
	toColor: number;    // palette index to use
	scope: ReplaceScope;
	tileIds?: string[]; // for 'selected' scope
}

/** Pixel editor state snapshot */
export interface PixelEditorSnapshot {
	tool: PixelTool;
	selectedColor: number | null;
	secondaryColor: number | null;
	replaceScope: ReplaceScope;
	brushSize: number;
	editingTileId: string | null;
}


// ============================================================================
// Helpers
// ============================================================================

/**
 * Convert pixel x,y to linear index
 */
export function pixelToIndex(x: number, y: number): number {
	return y * TILE_SIZE + x;
}

/**
 * Convert linear index to x,y
 */
export function indexToPixel(index: number): { x: number; y: number } {
	return {
		x: index % TILE_SIZE,
		y: Math.floor(index / TILE_SIZE),
	};
}

/**
 * Check if pixel coordinates are valid
 */
export function isValidPixel(x: number, y: number): boolean {
	return x >= 0 && x < TILE_SIZE && y >= 0 && y < TILE_SIZE;
}

/**
 * Get pixel color from tile data
 */
export function getPixelColor(tileData: Uint8Array, x: number, y: number): number {
	if (!isValidPixel(x, y)) return 0;
	return tileData[pixelToIndex(x, y)];
}

/**
 * Set pixel color in tile data (mutates array)
 */
export function setPixelColor(tileData: Uint8Array, x: number, y: number, colorIndex: number): void {
	if (!isValidPixel(x, y)) return;
	tileData[pixelToIndex(x, y)] = colorIndex;
}

/**
 * Get all pixel indices with a specific color
 */
export function findPixelsWithColor(tileData: Uint8Array, colorIndex: number): number[] {
	const result: number[] = [];
	for (let i = 0; i < tileData.length; i++) {
		if (tileData[i] === colorIndex) {
			result.push(i);
		}
	}
	return result;
}

/**
 * Count occurrences of each color in tile
 */
export function countColors(tileData: Uint8Array): Map<number, number> {
	const counts = new Map<number, number>();
	for (const colorIndex of tileData) {
		counts.set(colorIndex, (counts.get(colorIndex) ?? 0) + 1);
	}
	return counts;
}

/**
 * Get unique colors used in tile
 */
export function getUsedColors(tileData: Uint8Array): number[] {
	return [...new Set(tileData)].sort((a, b) => a - b);
}
