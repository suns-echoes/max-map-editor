/**
 * Image Import Types
 */

export interface ImageImportOptions {
	/** Image file path (for Tauri) or File object (for browser) */
	source: string | File;
	/** Name for the new map (defaults to filename) */
	mapName?: string;
}

export interface ImageImportResult {
	success: boolean;
	error?: string;
	mapName?: string;
	width?: number;
	height?: number;
	tileCount?: number;
}

export interface ImageImportState {
	/** Whether import is in progress */
	isImporting: boolean;
	/** Progress percentage (0-100) */
	progress: number;
	/** Current step description */
	step: string;
}

/**
 * Tile size in pixels (M.A.X. uses 64x64 tiles)
 */
export const TILE_SIZE = 64;
export const TILE_PIXELS = TILE_SIZE * TILE_SIZE;
