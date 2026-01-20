/**
 * Palette Editor Actions
 *
 * Actions for editing palette colors with undo/redo support.
 */

import { xlog } from '^lib/xlog/xlog.ts';
import { AppState } from '^state/app-state.ts';
import { HistoryState } from '^state/history-state.ts';
import { PaletteEditorState } from './palette-editor.state.ts';
import {
	type RgbColor,
	type PaletteChange,
	type PaletteUndoData,
	type PaletteColorInfo,
	getPaletteColor,
	setPaletteColor,
	colorsEqual,
	blendColors,
} from './palette-editor.types.ts';


// ============================================================================
// Palette Read Operations
// ============================================================================

/**
 * Get color at palette index
 */
export function getColorAt(index: number): RgbColor | null {
	const palette = AppState.palette.value;
	if (!palette || index < 0 || index > 255) return null;
	return getPaletteColor(palette, index);
}

/**
 * Get all palette colors with usage info
 */
export function getPaletteWithUsage(): PaletteColorInfo[] {
	const palette = AppState.palette.value;
	const tiles = AppState.tiles.value;
	if (!palette) return [];

	const result: PaletteColorInfo[] = [];
	const usageCounts = new Uint32Array(256);
	const tileCounts = new Uint16Array(256);

	// Count usage in tiles
	if (tiles) {
		for (const tile of tiles.values()) {
			const usedInTile = new Set<number>();
			for (const colorIndex of tile.data) {
				usageCounts[colorIndex]++;
				usedInTile.add(colorIndex);
			}
			for (const colorIndex of usedInTile) {
				tileCounts[colorIndex]++;
			}
		}
	}

	// Build result
	for (let i = 0; i < 256; i++) {
		result.push({
			index: i,
			color: getPaletteColor(palette, i),
			usageCount: usageCounts[i],
			tileCount: tileCounts[i],
		});
	}

	return result;
}

/**
 * Find unused palette indices
 */
export function findUnusedColors(): number[] {
	const info = getPaletteWithUsage();
	return info.filter(c => c.usageCount === 0).map(c => c.index);
}

/**
 * Find similar colors in palette
 */
export function findSimilarColors(targetColor: RgbColor, threshold: number = 30): number[] {
	const palette = AppState.palette.value;
	if (!palette) return [];

	const result: number[] = [];

	for (let i = 0; i < 256; i++) {
		const color = getPaletteColor(palette, i);
		const dr = color.r - targetColor.r;
		const dg = color.g - targetColor.g;
		const db = color.b - targetColor.b;
		const distance = Math.sqrt(dr * dr + dg * dg + db * db);

		if (distance <= threshold) {
			result.push(i);
		}
	}

	return result;
}


// ============================================================================
// Palette Write Operations
// ============================================================================

/**
 * Set color at palette index
 */
export function setColorAt(index: number, newColor: RgbColor): boolean {
	const palette = AppState.palette.value;
	if (!palette || index < 0 || index > 255) return false;

	const oldColor = getPaletteColor(palette, index);
	if (colorsEqual(oldColor, newColor)) return false;

	// Apply change
	setPaletteColor(palette, index, newColor);

	// Record for undo
	recordChange([{ index, oldColor, newColor }]);

	// Trigger reactive update
	AppState.palette.set(new Uint8Array(palette));

	xlog.info('PaletteEditor', `Set color ${index} to rgb(${newColor.r},${newColor.g},${newColor.b})`);

	return true;
}

/**
 * Set multiple colors at once
 */
export function setColorsAt(changes: Array<{ index: number; color: RgbColor }>): number {
	const palette = AppState.palette.value;
	if (!palette) return 0;

	const paletteChanges: PaletteChange[] = [];

	for (const { index, color } of changes) {
		if (index < 0 || index > 255) continue;

		const oldColor = getPaletteColor(palette, index);
		if (colorsEqual(oldColor, color)) continue;

		setPaletteColor(palette, index, color);
		paletteChanges.push({ index, oldColor, newColor: color });
	}

	if (paletteChanges.length === 0) return 0;

	// Record for undo
	recordChange(paletteChanges);

	// Trigger reactive update
	AppState.palette.set(new Uint8Array(palette));

	xlog.info('PaletteEditor', `Updated ${paletteChanges.length} colors`);

	return paletteChanges.length;
}

/**
 * Swap two palette colors
 */
export function swapColors(indexA: number, indexB: number): boolean {
	const palette = AppState.palette.value;
	if (!palette) return false;
	if (indexA < 0 || indexA > 255 || indexB < 0 || indexB > 255) return false;
	if (indexA === indexB) return false;

	const colorA = getPaletteColor(palette, indexA);
	const colorB = getPaletteColor(palette, indexB);

	if (colorsEqual(colorA, colorB)) return false;

	// Apply swap
	setPaletteColor(palette, indexA, colorB);
	setPaletteColor(palette, indexB, colorA);

	// Record for undo
	recordChange([
		{ index: indexA, oldColor: colorA, newColor: colorB },
		{ index: indexB, oldColor: colorB, newColor: colorA },
	]);

	// Trigger reactive update
	AppState.palette.set(new Uint8Array(palette));

	xlog.info('PaletteEditor', `Swapped colors ${indexA} and ${indexB}`);

	return true;
}

/**
 * Create gradient between two palette indices
 */
export function createGradient(startIndex: number, endIndex: number): boolean {
	const palette = AppState.palette.value;
	if (!palette) return false;

	const minIdx = Math.min(startIndex, endIndex);
	const maxIdx = Math.max(startIndex, endIndex);

	if (minIdx < 0 || maxIdx > 255 || minIdx === maxIdx) return false;

	const startColor = getPaletteColor(palette, minIdx);
	const endColor = getPaletteColor(palette, maxIdx);
	const steps = maxIdx - minIdx;

	const changes: PaletteChange[] = [];

	for (let i = minIdx; i <= maxIdx; i++) {
		const t = (i - minIdx) / steps;
		const newColor = blendColors(startColor, endColor, t);
		const oldColor = getPaletteColor(palette, i);

		if (!colorsEqual(oldColor, newColor)) {
			setPaletteColor(palette, i, newColor);
			changes.push({ index: i, oldColor, newColor });
		}
	}

	if (changes.length === 0) return false;

	// Record for undo
	recordChange(changes);

	// Trigger reactive update
	AppState.palette.set(new Uint8Array(palette));

	xlog.info('PaletteEditor', `Created gradient from ${minIdx} to ${maxIdx}`);

	return true;
}

/**
 * Shift colors in a range (cycle)
 */
export function shiftColors(startIndex: number, endIndex: number, direction: 1 | -1): boolean {
	const palette = AppState.palette.value;
	if (!palette) return false;

	const minIdx = Math.min(startIndex, endIndex);
	const maxIdx = Math.max(startIndex, endIndex);

	if (minIdx < 0 || maxIdx > 255 || minIdx === maxIdx) return false;

	const changes: PaletteChange[] = [];

	// Save all old colors first
	const oldColors: RgbColor[] = [];
	for (let i = minIdx; i <= maxIdx; i++) {
		oldColors.push(getPaletteColor(palette, i));
	}

	// Apply shift
	const count = maxIdx - minIdx + 1;
	for (let i = 0; i < count; i++) {
		const srcIdx = direction === 1
			? (i + 1) % count
			: (i - 1 + count) % count;

		const targetIndex = minIdx + i;
		const newColor = oldColors[srcIdx];
		const oldColor = oldColors[i];

		if (!colorsEqual(oldColor, newColor)) {
			setPaletteColor(palette, targetIndex, newColor);
			changes.push({ index: targetIndex, oldColor, newColor });
		}
	}

	if (changes.length === 0) return false;

	// Record for undo
	recordChange(changes);

	// Trigger reactive update
	AppState.palette.set(new Uint8Array(palette));

	xlog.info('PaletteEditor', `Shifted colors ${minIdx}-${maxIdx} ${direction === 1 ? 'forward' : 'backward'}`);

	return true;
}


// ============================================================================
// Clipboard Operations
// ============================================================================

/**
 * Copy color at selected index
 */
export function copySelectedColor(): boolean {
	const index = PaletteEditorState.selectedIndex.value;
	if (index === null) return false;

	const color = getColorAt(index);
	if (!color) return false;

	PaletteEditorState.copyColor(color);
	xlog.info('PaletteEditor', `Copied color ${index}`);
	return true;
}

/**
 * Paste color to selected index
 */
export function pasteColor(): boolean {
	const index = PaletteEditorState.selectedIndex.value;
	const color = PaletteEditorState.copiedColor.value;

	if (index === null || !color) return false;

	return setColorAt(index, color);
}


// ============================================================================
// History Integration
// ============================================================================

/**
 * Record palette changes for undo/redo
 */
function recordChange(changes: PaletteChange[]) {
	const undoData: PaletteUndoData = { changes };

	const description = changes.length === 1
		? `Change palette color ${changes[0].index}`
		: `Change ${changes.length} palette colors`;

	HistoryState.push('palette', undoData, undoData, description);
}

/**
 * Apply undo for palette changes
 */
export function applyPaletteUndo(data: PaletteUndoData) {
	const palette = AppState.palette.value;
	if (!palette) return;

	for (const change of data.changes) {
		setPaletteColor(palette, change.index, change.oldColor);
	}

	AppState.palette.set(new Uint8Array(palette));
}

/**
 * Apply redo for palette changes
 */
export function applyPaletteRedo(data: PaletteUndoData) {
	const palette = AppState.palette.value;
	if (!palette) return;

	for (const change of data.changes) {
		setPaletteColor(palette, change.index, change.newColor);
	}

	AppState.palette.set(new Uint8Array(palette));
}
