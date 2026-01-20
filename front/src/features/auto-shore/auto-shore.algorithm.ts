import { AppState } from '^state/app-state.ts';


// ============================================================================
// Types
// ============================================================================

/** Neighbor information for a cell */
type NeighborInfo = {
	n: TileType | null;  // north
	w: TileType | null;  // west
	s: TileType | null;  // south
	e: TileType | null;  // east
};

type TileType = 'water' | 'shore' | 'land' | 'obstruction';

/** Direction constants */
const DIRECTIONS = ['N', 'W', 'S', 'E'] as const;


// ============================================================================
// Internal Helpers
// ============================================================================

/**
 * Get map dimensions and data safely.
 */
function getMapContext() {
	const mapProject = AppState.mapProject.value;
	const map = AppState.map.value;
	const tiles = AppState.tiles.value;

	if (!mapProject || !map || !tiles) return null;

	return {
		width: mapProject.width,
		height: mapProject.height,
		map,
		tiles,
	};
}

/**
 * Get tile at coordinates, returns null if out of bounds.
 */
function getTileAt(
	x: number,
	y: number,
	width: number,
	height: number,
	map: Uint16Array,
	tilesArray: [string, Tile][]
): Tile | null {
	if (x < 0 || x >= width || y < 0 || y >= height) return null;
	const index = y * width + x;
	const tileIndex = map[index];
	return tilesArray[tileIndex]?.[1] ?? null;
}

/**
 * Get tile type at coordinates.
 */
function getTileTypeAt(
	x: number,
	y: number,
	width: number,
	height: number,
	map: Uint16Array,
	tilesArray: [string, Tile][]
): TileType | null {
	const tile = getTileAt(x, y, width, height, map, tilesArray);
	return tile?.props.type ?? null;
}

/**
 * Get neighbor types for a cell.
 */
function getNeighborTypes(
	x: number,
	y: number,
	width: number,
	height: number,
	map: Uint16Array,
	tilesArray: [string, Tile][]
): NeighborInfo {
	return {
		n: getTileTypeAt(x, y - 1, width, height, map, tilesArray),
		w: getTileTypeAt(x - 1, y, width, height, map, tilesArray),
		s: getTileTypeAt(x, y + 1, width, height, map, tilesArray),
		e: getTileTypeAt(x + 1, y, width, height, map, tilesArray),
	};
}

/**
 * Check if a cell needs a shore tile.
 * A shore is needed when land/obstruction is adjacent to water.
 */
function needsShore(neighbors: NeighborInfo, currentType: TileType): boolean {
	if (currentType === 'water') return false;
	if (currentType === 'shore') return true; // Already shore, may need re-evaluation

	// Land or obstruction adjacent to water needs shore
	const hasWaterNeighbor =
		neighbors.n === 'water' ||
		neighbors.w === 'water' ||
		neighbors.s === 'water' ||
		neighbors.e === 'water';

	return hasWaterNeighbor;
}

/**
 * Build a pattern string describing water adjacency.
 * e.g., "NW" means water to the north and west.
 */
function buildWaterPattern(neighbors: NeighborInfo): string {
	let pattern = '';
	if (neighbors.n === 'water') pattern += 'N';
	if (neighbors.w === 'water') pattern += 'W';
	if (neighbors.s === 'water') pattern += 'S';
	if (neighbors.e === 'water') pattern += 'E';
	return pattern;
}

/**
 * Check if a tile matches the required adjacency pattern.
 * Uses the tile's match rules to verify compatibility.
 */
function tileMatchesPattern(tile: Tile, neighbors: NeighborInfo): boolean {
	const { match } = tile;

	// Check each direction
	for (const dir of DIRECTIONS) {
		const neighborType = neighbors[dir.toLowerCase() as 'n' | 'w' | 's' | 'e'];

		if (neighborType === 'water') {
			// This direction should allow water
			const allowsWater = match[dir].includes('__WATER__');
			if (!allowsWater) return false;
		} else if (neighborType === 'land' || neighborType === 'obstruction') {
			// This direction should NOT require water
			const requiresWater = match[dir].length === 1 && match[dir][0] === '__WATER__';
			if (requiresWater) return false;
		}
	}

	return true;
}

/**
 * Find all shore tiles that match a given water adjacency pattern.
 */
function findMatchingShoreTiles(
	neighbors: NeighborInfo,
	tiles: Tiles
): [string, Tile][] {
	const matches: [string, Tile][] = [];

	for (const [tileId, tile] of tiles) {
		if (tile.props.type !== 'shore') continue;
		if (tileMatchesPattern(tile, neighbors)) {
			matches.push([tileId, tile]);
		}
	}

	return matches;
}

/**
 * Get tile index by tile ID.
 */
function getTileIndex(tileId: string, tilesArray: [string, Tile][]): number {
	return tilesArray.findIndex(([id]) => id === tileId);
}


// ============================================================================
// Auto Shore Algorithm
// ============================================================================

/**
 * Analyze the map and find all cells that need shore tiles.
 * Returns a list of positions and their required shore patterns.
 */
export function analyzeShoreNeeds(): Array<{
	x: number;
	y: number;
	pattern: string;
	neighbors: NeighborInfo;
}> {
	const ctx = getMapContext();
	if (!ctx) return [];

	const { width, height, map, tiles } = ctx;
	const tilesArray = [...tiles.entries()];
	const needs: Array<{ x: number; y: number; pattern: string; neighbors: NeighborInfo }> = [];

	for (let y = 0; y < height; y++) {
		for (let x = 0; x < width; x++) {
			const tile = getTileAt(x, y, width, height, map, tilesArray);
			if (!tile) continue;

			const neighbors = getNeighborTypes(x, y, width, height, map, tilesArray);

			if (needsShore(neighbors, tile.props.type)) {
				const pattern = buildWaterPattern(neighbors);
				if (pattern) {
					needs.push({ x, y, pattern, neighbors });
				}
			}
		}
	}

	return needs;
}

/**
 * Apply auto-shore to the entire map.
 * Finds cells that need shore tiles and replaces them with appropriate shore tiles.
 * Returns the number of tiles changed.
 */
export function autoFixShore(): number {
	const ctx = getMapContext();
	if (!ctx) return 0;

	const { width, height, map, tiles } = ctx;
	const tilesArray = [...tiles.entries()];
	let changedCount = 0;

	for (let y = 0; y < height; y++) {
		for (let x = 0; x < width; x++) {
			const currentTile = getTileAt(x, y, width, height, map, tilesArray);
			if (!currentTile) continue;

			const neighbors = getNeighborTypes(x, y, width, height, map, tilesArray);
			const currentType = currentTile.props.type;

			// Skip water tiles
			if (currentType === 'water') continue;

			// Check if shore is needed
			if (!needsShore(neighbors, currentType)) continue;

			// Find matching shore tiles
			const matchingShores = findMatchingShoreTiles(neighbors, tiles);

			if (matchingShores.length > 0) {
				// Pick a random matching shore tile for variety
				const [newTileId] = matchingShores[Math.floor(Math.random() * matchingShores.length)];
				const newTileIndex = getTileIndex(newTileId, tilesArray);

				if (newTileIndex >= 0) {
					const mapIndex = y * width + x;
					const currentTileIndex = map[mapIndex];

					if (currentTileIndex !== newTileIndex) {
						map[mapIndex] = newTileIndex;
						changedCount++;
					}
				}
			}
		}
	}

	// Trigger reactive update if changes were made
	if (changedCount > 0) {
		AppState.map.set(map);
	}

	return changedCount;
}

/**
 * Apply auto-shore to a specific region of the map.
 * Useful for fixing shore around recently painted areas.
 */
export function autoFixShoreRegion(
	startX: number,
	startY: number,
	endX: number,
	endY: number
): number {
	const ctx = getMapContext();
	if (!ctx) return 0;

	const { width, height, map, tiles } = ctx;
	const tilesArray = [...tiles.entries()];
	let changedCount = 0;

	// Expand region by 1 to catch edge cases
	const minX = Math.max(0, Math.min(startX, endX) - 1);
	const maxX = Math.min(width - 1, Math.max(startX, endX) + 1);
	const minY = Math.max(0, Math.min(startY, endY) - 1);
	const maxY = Math.min(height - 1, Math.max(startY, endY) + 1);

	for (let y = minY; y <= maxY; y++) {
		for (let x = minX; x <= maxX; x++) {
			const currentTile = getTileAt(x, y, width, height, map, tilesArray);
			if (!currentTile) continue;

			const neighbors = getNeighborTypes(x, y, width, height, map, tilesArray);
			const currentType = currentTile.props.type;

			if (currentType === 'water') continue;
			if (!needsShore(neighbors, currentType)) continue;

			const matchingShores = findMatchingShoreTiles(neighbors, tiles);

			if (matchingShores.length > 0) {
				const [newTileId] = matchingShores[Math.floor(Math.random() * matchingShores.length)];
				const newTileIndex = getTileIndex(newTileId, tilesArray);

				if (newTileIndex >= 0) {
					const mapIndex = y * width + x;
					const currentTileIndex = map[mapIndex];

					if (currentTileIndex !== newTileIndex) {
						map[mapIndex] = newTileIndex;
						changedCount++;
					}
				}
			}
		}
	}

	if (changedCount > 0) {
		AppState.map.set(map);
	}

	return changedCount;
}

/**
 * Validate shore placement on the map.
 * Returns cells with invalid shore tiles (tiles that don't match their neighbors).
 */
export function validateShores(): Array<{
	x: number;
	y: number;
	tileId: string;
	issue: string;
}> {
	const ctx = getMapContext();
	if (!ctx) return [];

	const { width, height, map, tiles } = ctx;
	const tilesArray = [...tiles.entries()];
	const issues: Array<{ x: number; y: number; tileId: string; issue: string }> = [];

	for (let y = 0; y < height; y++) {
		for (let x = 0; x < width; x++) {
			const index = y * width + x;
			const tileIndex = map[index];
			const [tileId, tile] = tilesArray[tileIndex] ?? [];

			if (!tile || tile.props.type !== 'shore') continue;

			const neighbors = getNeighborTypes(x, y, width, height, map, tilesArray);

			if (!tileMatchesPattern(tile, neighbors)) {
				issues.push({
					x,
					y,
					tileId,
					issue: `Shore tile doesn't match neighbors: N=${neighbors.n}, W=${neighbors.w}, S=${neighbors.s}, E=${neighbors.e}`,
				});
			}
		}
	}

	return issues;
}
