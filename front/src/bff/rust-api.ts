import { invoke as _invoke, isTauri } from '@tauri-apps/api/core';


/**
 * Whether we're running in Tauri (native) or browser (dev/preview) mode.
 * Cached at module load for performance.
 */
const IS_TAURI = isTauri();

/**
 * Invoke wrapper that provides safe fallbacks when running outside Tauri.
 * In browser mode, commands either return sensible defaults or reject with clear errors.
 */
const invoke = IS_TAURI
	? _invoke
	: async <T>(cmd: string, _data?: unknown): Promise<T> => {
		console.warn(`[RustAPI] Command "${cmd}" called outside Tauri - using fallback`);

		// Provide sensible fallbacks for each command
		switch (cmd) {
			case 'open_devtools':
				// No-op in browser
				return undefined as T;

			case 'validate_max_dir':
				// Always valid in browser (for testing UI flow)
				return true as T;

			case 'xlog_command':
				// No-op in browser (console.log already happens in xlog)
				return undefined as T;

			case 'image_to_wrl':
				// This actually needs Rust, so reject with clear error
				throw new Error('image_to_wrl requires Tauri runtime - not available in browser mode');

			default:
				throw new Error(`Unknown command "${cmd}" - no browser fallback available`);
		}
	};


export const RustAPI = {
	/** Open DevTools window (Tauri only, no-op in browser) */
	openDevTools: (): Promise<void> => invoke('open_devtools'),

	/** Validate that a path contains a valid M.A.X. installation */
	validateMAXDir: (path: string): Promise<boolean> => invoke('validate_max_dir', { path }),

	/** Convert an image file to WRL format (Tauri only) */
	imageToWRL: (path: string): Promise<[palette: Uint8Array, indexedImage: Uint8Array]> => invoke('image_to_wrl', { path }),

	/** Log message to Rust backend (no-op in browser) */
	xlog: (level: string, message: string): Promise<void> => invoke('xlog_command', { level, message }),
};
