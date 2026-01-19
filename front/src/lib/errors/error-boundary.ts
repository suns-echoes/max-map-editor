/**
 * Global Error Boundary
 *
 * Catches unhandled errors and promise rejections at the application level.
 * Logs them via xlog and optionally shows a user-facing error modal.
 *
 * This should be initialized once at application startup.
 */

import { xlog } from '^lib/xlog/xlog.ts';


// ============================================================================
// Types
// ============================================================================

export interface ErrorBoundaryConfig {
	/** Called when an error is caught. Can return false to prevent default logging. */
	onError?: (error: Error, source: 'sync' | 'async') => boolean | void;
	/** Whether to show errors in console even if onError is provided */
	alwaysLog?: boolean;
}


// ============================================================================
// State
// ============================================================================

let isInitialized = false;
let config: ErrorBoundaryConfig = {};


// ============================================================================
// Public API
// ============================================================================

/**
 * Initialize the global error boundary.
 * Call this once at application startup.
 */
export function initErrorBoundary(options: ErrorBoundaryConfig = {}): void {
	if (isInitialized) {
		xlog.warn('Error boundary already initialized');
		return;
	}

	config = options;
	isInitialized = true;

	// Catch synchronous errors
	window.onerror = function (message, source, lineno, colno, error) {
		handleError(error ?? new Error(String(message)), 'sync', {
			source,
			line: lineno,
			column: colno,
		});
		return true; // Prevents default browser error handling
	};

	// Catch unhandled promise rejections
	window.onunhandledrejection = function (event) {
		const error = event.reason instanceof Error
			? event.reason
			: new Error(String(event.reason));

		handleError(error, 'async');
		event.preventDefault(); // Prevents default browser error handling
	};

	xlog.info('Error boundary initialized');
}


/**
 * Dispose the global error boundary.
 * Useful for testing or cleanup.
 */
export function disposeErrorBoundary(): void {
	if (!isInitialized) return;

	window.onerror = null;
	window.onunhandledrejection = null;
	isInitialized = false;
	config = {};
}


// ============================================================================
// Internal
// ============================================================================

interface ErrorContext {
	source?: string;
	line?: number;
	column?: number;
}

function handleError(error: Error, type: 'sync' | 'async', context?: ErrorContext): void {
	const shouldLog = config.alwaysLog || !config.onError;

	// Call custom handler if provided
	const preventLog = config.onError?.(error, type) === false;

	// Log to xlog unless prevented
	if (shouldLog && !preventLog) {
		const prefix = type === 'sync' ? 'Uncaught error' : 'Unhandled promise rejection';
		const location = context?.source
			? ` at ${context.source}:${context.line}:${context.column}`
			: '';

		xlog.error(`${prefix}${location}:`, error.message);

		if (error.stack) {
			xlog.error('Stack trace:', error.stack);
		}
	}
}
