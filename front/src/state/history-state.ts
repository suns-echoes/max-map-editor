import { Value } from '^reactive/value.ts';
import { Expr } from '^reactive/expr.ts';


// ============================================================================
// History Types
// ============================================================================

/** A single history entry representing a reversible action */
export type HistoryEntry = {
	/** Action type identifier */
	type: string;
	/** Data needed to undo this action */
	undoData: unknown;
	/** Data needed to redo this action */
	redoData: unknown;
	/** Timestamp when action was performed */
	timestamp: number;
	/** Optional human-readable description */
	description?: string;
};

/** Maximum number of history entries to keep */
const MAX_HISTORY_SIZE = 100;


// ============================================================================
// History State - Reactive Values
// ============================================================================

/** Stack of past actions (most recent last) */
const past = new Value<HistoryEntry[]>([]);

/** Stack of undone actions (most recent last) */
const future = new Value<HistoryEntry[]>([]);


// ============================================================================
// Derived State
// ============================================================================

/** True when undo is available */
const canUndo = new Expr(() => past.value.length > 0);

/** True when redo is available */
const canRedo = new Expr(() => future.value.length > 0);

/** Number of actions in history */
const historySize = new Expr(() => past.value.length);


// ============================================================================
// Actions
// ============================================================================

/**
 * Push a new action onto the history stack.
 * Clears the redo stack since we're on a new branch.
 */
function push(type: string, undoData: unknown, redoData: unknown, description?: string) {
	const entry: HistoryEntry = {
		type,
		undoData,
		redoData,
		timestamp: Date.now(),
		description,
	};

	const newPast = [...past.value, entry];

	// Trim history if it exceeds max size
	if (newPast.length > MAX_HISTORY_SIZE) {
		newPast.shift();
	}

	past.set(newPast);
	future.set([]); // Clear redo stack
}

/**
 * Pop the most recent action from history.
 * Returns the entry for the caller to apply the undo.
 */
function undo(): HistoryEntry | null {
	const pastEntries = past.value;
	if (pastEntries.length === 0) return null;

	const entry = pastEntries[pastEntries.length - 1];
	past.set(pastEntries.slice(0, -1));
	future.set([...future.value, entry]);

	return entry;
}

/**
 * Redo the most recently undone action.
 * Returns the entry for the caller to apply the redo.
 */
function redo(): HistoryEntry | null {
	const futureEntries = future.value;
	if (futureEntries.length === 0) return null;

	const entry = futureEntries[futureEntries.length - 1];
	future.set(futureEntries.slice(0, -1));
	past.set([...past.value, entry]);

	return entry;
}

/**
 * Clear all history (both past and future).
 */
function clear() {
	past.set([]);
	future.set([]);
}

/**
 * Get the last N entries from history (for UI display).
 */
function getRecentEntries(count: number = 10): HistoryEntry[] {
	const pastEntries = past.value;
	return pastEntries.slice(-count).reverse();
}


// ============================================================================
// Export
// ============================================================================

export const HistoryState = {
	// Values (read-only access recommended)
	past,
	future,

	// Derived
	canUndo,
	canRedo,
	historySize,

	// Actions
	push,
	undo,
	redo,
	clear,
	getRecentEntries,
};
