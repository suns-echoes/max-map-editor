/**
 * Error Handling Utilities
 *
 * Re-exports all error handling utilities from a single entry point.
 */

export {
	initErrorBoundary,
	disposeErrorBoundary,
	type ErrorBoundaryConfig,
} from './error-boundary.ts';

export {
	tryCatch,
	tryCatchAsync,
	tryCatchFn,
	toError,
	unwrap,
	type Result,
} from './try-catch.ts';
