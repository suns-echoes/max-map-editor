/**
 * MapProject Validation
 *
 * Runtime validation for MapProject JSON files.
 * Provides detailed error messages with field paths.
 */


// ============================================================================
// Validation Error
// ============================================================================

export class MapProjectValidationError extends Error {
	constructor(field: string, message: string) {
		super(`MapProject validation failed at '${field}': ${message}`);
		this.name = 'MapProjectValidationError';
	}
}


// ============================================================================
// Main Validation
// ============================================================================

/**
 * Parse and validate a map project JSON string.
 * Throws detailed errors if validation fails.
 */
export function parseMapProject(jsonString: string): MapProject {
	let data: unknown;
	try {
		data = JSON.parse(jsonString);
	} catch (e) {
		throw new MapProjectValidationError('Invalid JSON', e instanceof Error ? e.message : String(e));
	}

	validateMapProject(data);
	return data;
}


/**
 * Validates data matches the MapProject type.
 * Uses assertion function to narrow type.
 */
export function validateMapProject(data: unknown): asserts data is MapProject {
	if (data === null || typeof data !== 'object') {
		throw new MapProjectValidationError('root', 'Expected an object');
	}

	const obj = data as Record<string, unknown>;

	// version: must be 1
	if (obj.version !== 1) {
		throw new MapProjectValidationError('version', `Expected 1, got ${obj.version}`);
	}

	// name: required string
	if (typeof obj.name !== 'string' || obj.name.length === 0) {
		throw new MapProjectValidationError('name', 'Expected non-empty string');
	}

	// description: required string (can be empty)
	if (typeof obj.description !== 'string') {
		throw new MapProjectValidationError('description', 'Expected string');
	}

	// width: positive integer
	if (typeof obj.width !== 'number' || !Number.isInteger(obj.width) || obj.width <= 0) {
		throw new MapProjectValidationError('width', 'Expected positive integer');
	}

	// height: positive integer
	if (typeof obj.height !== 'number' || !Number.isInteger(obj.height) || obj.height <= 0) {
		throw new MapProjectValidationError('height', 'Expected positive integer');
	}

	// use: array of asset references
	if (!Array.isArray(obj.use)) {
		throw new MapProjectValidationError('use', 'Expected array');
	}
	if (obj.use.length === 0) {
		throw new MapProjectValidationError('use', 'Expected at least one asset');
	}
	for (let i = 0; i < obj.use.length; i++) {
		validateAssetReference(obj.use[i], `use[${i}]`);
	}

	// map: array of rows (string or string[])
	if (!Array.isArray(obj.map)) {
		throw new MapProjectValidationError('map', 'Expected array');
	}
	if (obj.map.length !== obj.height) {
		throw new MapProjectValidationError('map', `Expected ${obj.height} rows, got ${obj.map.length}`);
	}
	for (let y = 0; y < obj.map.length; y++) {
		validateMapRow(obj.map[y], obj.width, `map[${y}]`);
	}
}


// ============================================================================
// Helper Validators
// ============================================================================

/**
 * Validates an asset reference in the 'use' array.
 */
function validateAssetReference(data: unknown, path: string): void {
	if (data === null || typeof data !== 'object') {
		throw new MapProjectValidationError(path, 'Expected an object');
	}

	const obj = data as Record<string, unknown>;

	// name: required string
	if (typeof obj.name !== 'string' || obj.name.length === 0) {
		throw new MapProjectValidationError(`${path}.name`, 'Expected non-empty string');
	}

	// version: required number
	if (typeof obj.version !== 'number') {
		throw new MapProjectValidationError(`${path}.version`, 'Expected number');
	}

	// tileset: optional boolean
	if (obj.tileset !== undefined && typeof obj.tileset !== 'boolean') {
		throw new MapProjectValidationError(`${path}.tileset`, 'Expected boolean');
	}

	// palette: optional boolean
	if (obj.palette !== undefined && typeof obj.palette !== 'boolean') {
		throw new MapProjectValidationError(`${path}.palette`, 'Expected boolean');
	}
}

/**
 * Validates a map row. Each cell is either a string or string[].
 */
function validateMapRow(row: unknown, expectedWidth: number, path: string): void {
	if (!Array.isArray(row)) {
		throw new MapProjectValidationError(path, 'Expected array');
	}
	if (row.length !== expectedWidth) {
		throw new MapProjectValidationError(path, `Expected ${expectedWidth} cells, got ${row.length}`);
	}
	for (let x = 0; x < row.length; x++) {
		const cell = row[x];
		if (typeof cell === 'string') {
			continue; // valid: single tile ID
		}
		if (Array.isArray(cell)) {
			// valid: array of tile IDs (layered)
			for (let layer = 0; layer < cell.length; layer++) {
				if (typeof cell[layer] !== 'string') {
					throw new MapProjectValidationError(
						`${path}[${x}][${layer}]`,
						`Expected string, got ${typeof cell[layer]}`
					);
				}
			}
			continue;
		}
		throw new MapProjectValidationError(`${path}[${x}]`, `Expected string or string[], got ${typeof cell}`);
	}
}
