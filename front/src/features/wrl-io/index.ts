/**
 * WRL I/O Feature
 *
 * Import/Export WRL (M.A.X. map) files
 */

// Format types and constants
export { WRL_HEADER, TILE_SIZE, TILE_PIXELS, PALETTE_SIZE } from './wrl-format.ts';
export type { WrlData } from './wrl-format.ts';

// Parser
export { parseWrlFile, validateWrlBuffer, calculateWrlFileSize } from './wrl-parser.ts';

// Builder
export { buildWrlFile, buildWrlDataFromAppState, generateMinimap } from './wrl-builder.ts';

// Import action
export { importWrlFile, importWrlFromFile } from './wrl-import.action.ts';
export type { WrlImportResult } from './wrl-import.action.ts';

// Export action
export { exportToWrl, downloadWrlFile, getWrlBlob } from './wrl-export.action.ts';
export type { WrlExportResult } from './wrl-export.action.ts';
