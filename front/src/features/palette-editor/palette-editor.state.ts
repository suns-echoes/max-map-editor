/**
 * Palette Editor State
 *
 * Reactive state for the palette editor.
 */

import { Value } from '^reactive/value.ts';
import { Expr } from '^reactive/expr.ts';


// ============================================================================
// State Values
// ============================================================================

/** Whether the palette panel is visible */
const isPanelVisible = new Value<boolean>(false);

/** Currently selected palette index (0-255) */
const selectedIndex = new Value<number | null>(null);

/** Secondary selected index for range operations */
const secondaryIndex = new Value<number | null>(null);

/** Zoom level for palette grid (1 = 16x16, 2 = 32x32 cells) */
const zoom = new Value<number>(1);

/** Show color usage overlay */
const showUsage = new Value<boolean>(false);

/** Currently copied color for paste */
const copiedColor = new Value<{ r: number; g: number; b: number } | null>(null);


// ============================================================================
// Derived State
// ============================================================================

/** True when a color is selected */
const hasSelection = new Expr(() => selectedIndex.value !== null);

/** True when range is selected (for gradient, etc.) */
const hasRange = new Expr(() =>
	selectedIndex.value !== null && secondaryIndex.value !== null
);

/** Get the selected range (sorted) */
const selectedRange = new Expr(() => {
	const a = selectedIndex.value;
	const b = secondaryIndex.value;
	if (a === null) return null;
	if (b === null) return { start: a, end: a };
	return { start: Math.min(a, b), end: Math.max(a, b) };
});


// ============================================================================
// Actions
// ============================================================================

/**
 * Select a palette index
 */
function select(index: number) {
	selectedIndex.set(index);
	secondaryIndex.set(null);
}

/**
 * Extend selection to index (for shift-click range select)
 */
function extendSelection(index: number) {
	if (selectedIndex.value === null) {
		selectedIndex.set(index);
	} else {
		secondaryIndex.set(index);
	}
}

/**
 * Clear selection
 */
function clearSelection() {
	selectedIndex.set(null);
	secondaryIndex.set(null);
}

/**
 * Set zoom level
 */
function setZoom(level: number) {
	zoom.set(Math.max(1, Math.min(3, level)));
}

/**
 * Toggle usage overlay
 */
function toggleUsage() {
	showUsage.set(!showUsage.value);
}

/**
 * Copy selected color
 */
function copyColor(color: { r: number; g: number; b: number }) {
	copiedColor.set({ ...color });
}

/**
 * Reset state
 */
function reset() {
	selectedIndex.set(null);
	secondaryIndex.set(null);
	zoom.set(1);
	showUsage.set(false);
	copiedColor.set(null);
}


// ============================================================================
// Export
// ============================================================================

export const PaletteEditorState = {
	// Values
	isPanelVisible,
	selectedIndex,
	secondaryIndex,
	zoom,
	showUsage,
	copiedColor,

	// Derived
	hasSelection,
	hasRange,
	selectedRange,

	// Actions
	select,
	extendSelection,
	clearSelection,
	setZoom,
	toggleUsage,
	copyColor,
	reset,

	// Panel visibility
	showPanel: () => isPanelVisible.set(true),
	hidePanel: () => isPanelVisible.set(false),
	togglePanel: () => isPanelVisible.set(!isPanelVisible.value),
};
