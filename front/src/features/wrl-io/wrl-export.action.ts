/**
 * WRL Export Action
 *
 * Exports the current map to WRL format
 */

import { xlog } from '^lib/xlog/xlog.ts';
import { AppState } from '^state/app-state.ts';
import { buildWrlFile, buildWrlDataFromAppState } from './wrl-builder.ts';


export interface WrlExportResult {
	success: boolean;
	error?: string;
	buffer?: ArrayBuffer;
	fileName?: string;
}


/**
 * Export current map to WRL format
 */
export function exportToWrl(): WrlExportResult {
	xlog.info('exportToWrl');

	const mapProject = AppState.mapProject.value;
	const map = AppState.map.value;
	const tiles = AppState.tiles.value;
	const palette = AppState.palette.value;

	// Validate state
	if (!mapProject) {
		return { success: false, error: 'No map loaded' };
	}
	if (!map) {
		return { success: false, error: 'No map data available' };
	}
	if (!tiles || tiles.size === 0) {
		return { success: false, error: 'No tiles available' };
	}
	if (!palette) {
		return { success: false, error: 'No palette available' };
	}

	try {
		// Build WRL data
		const wrlData = buildWrlDataFromAppState(mapProject, map, tiles, palette);

		// Build binary file
		const buffer = buildWrlFile(wrlData);

		// Generate filename
		const fileName = `${mapProject.name.replace(/[^a-zA-Z0-9_-]/g, '_')}.wrl`;

		xlog.info(`WRL exported: ${mapProject.width}x${mapProject.height}, ${tiles.size} tiles, ${buffer.byteLength} bytes`);

		return {
			success: true,
			buffer,
			fileName,
		};
	} catch (err) {
		const message = err instanceof Error ? err.message : 'Unknown error';
		xlog.error('WRL export failed:', message);
		return { success: false, error: message };
	}
}


/**
 * Export and download WRL file
 */
export function downloadWrlFile(): WrlExportResult {
	const result = exportToWrl();

	if (!result.success || !result.buffer || !result.fileName) {
		return result;
	}

	// Create blob and download
	const blob = new Blob([result.buffer], { type: 'application/octet-stream' });
	const url = URL.createObjectURL(blob);

	const link: HTMLAnchorElement = document.createElement('a');
	link.href = url;
	link.download = result.fileName;
	link.click();

	URL.revokeObjectURL(url);

	xlog.info(`Downloaded: ${result.fileName}`);
	return result;
}


/**
 * Get WRL file as Blob (for external use)
 */
export function getWrlBlob(): { blob: Blob; fileName: string } | null {
	const result = exportToWrl();

	if (!result.success || !result.buffer || !result.fileName) {
		return null;
	}

	return {
		blob: new Blob([result.buffer], { type: 'application/octet-stream' }),
		fileName: result.fileName,
	};
}
