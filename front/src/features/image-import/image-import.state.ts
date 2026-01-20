/**
 * Image Import State
 *
 * Reactive state for tracking image import progress
 */

import { Value } from '^reactive/value.ts';


export const ImageImportState = {
	isImporting: new Value<boolean>(false),
	progress: new Value<number>(0),
	step: new Value<string>(''),

	/**
	 * Update progress
	 */
	setProgress(progress: number, step: string) {
		this.progress.set(progress);
		this.step.set(step);
	},

	/**
	 * Start import
	 */
	start() {
		this.isImporting.set(true);
		this.progress.set(0);
		this.step.set('Starting...');
	},

	/**
	 * Complete import
	 */
	complete() {
		this.isImporting.set(false);
		this.progress.set(100);
		this.step.set('Complete');
	},

	/**
	 * Reset state
	 */
	reset() {
		this.isImporting.set(false);
		this.progress.set(0);
		this.step.set('');
	},
};
