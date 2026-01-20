/**
 * Pass Table Editor State
 *
 * Reactive state for the pass table editor mode.
 */

import { Value } from '^reactive/value.ts';
import { Expr } from '^reactive/expr.ts';
import { EditorState } from '^state/editor-state.ts';
import { type PassValue, PASS_VALUES } from './pass-table.types.ts';


// ============================================================================
// State Values
// ============================================================================

/** Currently selected pass value for painting */
const selectedPassValue = new Value<PassValue>(PASS_VALUES.LAND);

/** Whether to show pass value overlay on map */
const showOverlay = new Value<boolean>(true);

/** Overlay opacity (0.0 - 1.0) */
const overlayOpacity = new Value<number>(0.5);


// ============================================================================
// Derived State
// ============================================================================

/** True when in pass table editing mode */
const isActive = new Expr(() => EditorState.mode.value === 'passTable');

/** True when user can paint pass values */
const canPaint = new Expr(() =>
	isActive.value &&
	EditorState.tool.value !== 'select' &&
	EditorState.tool.value !== 'eyedropper'
);


// ============================================================================
// Actions
// ============================================================================

/**
 * Select a pass value for painting
 */
function selectPassValue(value: PassValue) {
	selectedPassValue.set(value);
}

/**
 * Toggle overlay visibility
 */
function toggleOverlay() {
	showOverlay.set(!showOverlay.value);
}

/**
 * Set overlay opacity
 */
function setOverlayOpacity(opacity: number) {
	overlayOpacity.set(Math.max(0, Math.min(1, opacity)));
}

/**
 * Enter pass table editor mode
 */
function enterMode() {
	EditorState.mode.set('passTable');
}

/**
 * Exit pass table editor mode (return to ground mode)
 */
function exitMode() {
	EditorState.mode.set('ground');
}

/**
 * Reset state to defaults
 */
function reset() {
	selectedPassValue.set(PASS_VALUES.LAND);
	showOverlay.set(true);
	overlayOpacity.set(0.5);
}


// ============================================================================
// Export
// ============================================================================

export const PassTableState = {
	// Values
	selectedPassValue,
	showOverlay,
	overlayOpacity,

	// Derived
	isActive,
	canPaint,

	// Actions
	selectPassValue,
	toggleOverlay,
	setOverlayOpacity,
	enterMode,
	exitMode,
	reset,
};
