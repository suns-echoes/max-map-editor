/**
 * HMR (Hot Module Replacement) Utilities
 *
 * Provides utilities for properly cleaning up side effects during HMR.
 * When Vite hot-replaces a module, effects from the old module need to be
 * disposed to prevent duplicates and memory leaks.
 *
 * @example
 * ```ts
 * // In a module with global effects:
 * import { hmrDispose, hmrAccept } from '^lib/hmr/hmr.ts';
 *
 * const effect1 = new Effect(() => { ... }, { strong: true });
 * const effect2 = new Effect(() => { ... }, { strong: true });
 *
 * // Register cleanup for HMR
 * hmrDispose(import.meta, () => {
 *   effect1.dispose();
 *   effect2.dispose();
 * });
 *
 * // Accept HMR updates
 * hmrAccept(import.meta);
 * ```
 */


// ============================================================================
// Types
// ============================================================================

interface ImportMeta {
	hot?: {
		accept: (callback?: (newModule: unknown) => void) => void;
		dispose: (callback: (data: Record<string, unknown>) => void) => void;
		data: Record<string, unknown>;
	};
}

interface Disposable {
	dispose(): void;
}


// ============================================================================
// HMR Utilities
// ============================================================================

/**
 * Check if we're in DEV mode.
 * import.meta.env is Vite-specific and may not exist in Node.js test environment.
 */
function isDev(): boolean {
	try {
		return import.meta.env?.DEV === true;
	} catch {
		return false;
	}
}


/**
 * Internal implementation that always runs (for testing).
 * Public API wraps this with isDev() check.
 */
function hmrDisposeInternal(importMeta: ImportMeta, cleanup: () => void): void {
	if (importMeta.hot) {
		importMeta.hot.dispose(() => {
			cleanup();
		});
	}
}


/**
 * Register a cleanup function to run when the module is hot-replaced.
 * The callback will only run during HMR in development mode.
 *
 * @param importMeta Pass `import.meta` from the module
 * @param cleanup Function to run before the module is replaced
 */
export function hmrDispose(importMeta: ImportMeta, cleanup: () => void): void {
	if (isDev()) {
		hmrDisposeInternal(importMeta, cleanup);
	}
}


/**
 * Internal implementation for testing.
 */
function hmrDisposeAllInternal(importMeta: ImportMeta, disposables: Disposable[]): void {
	if (importMeta.hot) {
		importMeta.hot.dispose(() => {
			for (const d of disposables) {
				d.dispose();
			}
		});
	}
}


/**
 * Register multiple disposables for HMR cleanup.
 * Convenience function for disposing multiple effects/subscriptions.
 *
 * @param importMeta Pass `import.meta` from the module
 * @param disposables Array of objects with dispose() method
 */
export function hmrDisposeAll(importMeta: ImportMeta, disposables: Disposable[]): void {
	if (isDev()) {
		hmrDisposeAllInternal(importMeta, disposables);
	}
}


/**
 * Internal implementation for testing.
 */
function hmrAcceptInternal(importMeta: ImportMeta): void {
	if (importMeta.hot) {
		importMeta.hot.accept();
	}
}


/**
 * Accept HMR updates for this module.
 * Call this at the end of modules that have global effects.
 *
 * @param importMeta Pass `import.meta` from the module
 */
export function hmrAccept(importMeta: ImportMeta): void {
	if (isDev()) {
		hmrAcceptInternal(importMeta);
	}
}


/**
 * Internal implementation for testing.
 */
function hmrEffectInternal<T extends Disposable>(
	importMeta: ImportMeta,
	createEffect: () => T
): T {
	const effect = createEffect();

	if (importMeta.hot) {
		// Store effects in hot.data to accumulate them
		const effects = (importMeta.hot.data.effects ??= []) as Disposable[];
		effects.push(effect);

		// Only register dispose once per module
		if (!importMeta.hot.data.disposeRegistered) {
			importMeta.hot.data.disposeRegistered = true;
			importMeta.hot.dispose((data) => {
				const storedEffects = data.effects as Disposable[] | undefined;
				if (storedEffects) {
					for (const e of storedEffects) {
						e.dispose();
					}
				}
			});
		}
	}

	return effect;
}


/**
 * Create an effect that is automatically disposed on HMR.
 * This is a higher-order function that wraps effect creation with HMR cleanup.
 *
 * @param importMeta Pass `import.meta` from the module
 * @param createEffect Function that creates and returns the effect
 * @returns The created effect
 */
export function hmrEffect<T extends Disposable>(
	importMeta: ImportMeta,
	createEffect: () => T
): T {
	if (isDev() && importMeta.hot) {
		return hmrEffectInternal(importMeta, createEffect);
	}
	// In production or without HMR, just create the effect
	return createEffect();
}


// ============================================================================
// Testing Utilities
// ============================================================================

/**
 * @internal - Exported only for testing purposes.
 * These bypass the isDev() check so HMR logic can be tested in Node.js.
 */
export const _testing = {
	hmrDisposeInternal,
	hmrDisposeAllInternal,
	hmrAcceptInternal,
	hmrEffectInternal,
};
