import { AppState } from '^state/app-state.ts';
import { HistoryState } from '^state/history-state.ts';
import { autoFixShore as runAutoFixShore, autoFixShoreRegion as runAutoFixShoreRegion } from './auto-shore.algorithm.ts';


// ============================================================================
// Types
// ============================================================================

/** Undo data for auto-shore operations */
export type AutoShoreUndoData = {
	/** Original map state before auto-shore */
	originalMap: Uint16Array;
};


// ============================================================================
// Actions
// ============================================================================

/**
 * Apply auto-shore to the entire map with undo support.
 * Returns the number of tiles changed.
 */
export function applyAutoShore(): number {
	const map = AppState.map.value;
	if (!map) return 0;

	// Save original state for undo
	const originalMap = new Uint16Array(map);

	// Run auto-shore algorithm
	const changedCount = runAutoFixShore();

	if (changedCount > 0) {
		// Record in history
		const undoData: AutoShoreUndoData = { originalMap };
		const redoData: AutoShoreUndoData = { originalMap: new Uint16Array(AppState.map.value!) };

		HistoryState.push(
			'auto-shore',
			undoData,
			redoData,
			`Auto-shore: ${changedCount} tiles`
		);
	}

	return changedCount;
}

/**
 * Apply auto-shore to a specific region with undo support.
 */
export function applyAutoShoreRegion(
	startX: number,
	startY: number,
	endX: number,
	endY: number
): number {
	const map = AppState.map.value;
	if (!map) return 0;

	// Save original state for undo
	const originalMap = new Uint16Array(map);

	// Run auto-shore algorithm on region
	const changedCount = runAutoFixShoreRegion(startX, startY, endX, endY);

	if (changedCount > 0) {
		// Record in history
		const undoData: AutoShoreUndoData = { originalMap };
		const redoData: AutoShoreUndoData = { originalMap: new Uint16Array(AppState.map.value!) };

		HistoryState.push(
			'auto-shore-region',
			undoData,
			redoData,
			`Auto-shore region: ${changedCount} tiles`
		);
	}

	return changedCount;
}


// ============================================================================
// Undo/Redo Handlers
// ============================================================================

/**
 * Undo auto-shore operation.
 */
export function undoAutoShore(undoData: AutoShoreUndoData) {
	AppState.map.set(undoData.originalMap);
}

/**
 * Redo auto-shore operation.
 */
export function redoAutoShore(redoData: AutoShoreUndoData) {
	AppState.map.set(redoData.originalMap);
}
