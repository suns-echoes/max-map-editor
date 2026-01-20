/**
 * Pass Table Editor Feature
 *
 * Edit pass values (passability) for tiles.
 */

// Types
export {
	PASS_VALUES,
	PASS_INFO,
	getPassValueFromType,
	getTypeFromPassValue,
	cyclePassValue,
} from './pass-table.types.ts';
export type { PassValue, PassChange, PassTableUndoData, PassStats } from './pass-table.types.ts';

// State
export { PassTableState } from './pass-table.state.ts';

// Actions
export {
	getTilePassValue,
	getAllPassValues,
	getPassStats,
	setTilePassValue,
	setMultipleTilePassValues,
	cycleTilePassValue,
	setPassValueForType,
	autoDetectPassValues,
	paintPassValueAt,
	pickPassValueAt,
	applyPassTableUndo,
	applyPassTableRedo,
} from './pass-table.actions.ts';

// UI
export { PassTablePanel } from './ui/pass-table-panel.component.ts';
