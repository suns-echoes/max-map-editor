import { Canvas, Div, Pre, Section } from '^reactive/reactive-node.elements.ts';
import { printDebugInfo } from '^lib/debug/debug.ts';
import { AppState } from '^state/app-state.ts';
import { Effect } from '^reactive/effect.ts';
import { loadMapProject } from '^actions/load-map-project/load-map-project.ts';
import { resolveTextResource } from '^tauri-apps/api/path.ts';
import { WglRenderer } from './wgl/wgl-renderer.ts';

import style from './wgl-map.module.css';


export function WGLMap() {
	printDebugInfo('UI::WGLMap');

	const canvas = Canvas('map-canvas');
	const debugInfo = Pre().class(style.debugInfo);

	const component = (
		Section('wgl-map').class(style.wglMap).nodes([
			Div().class(style.canvasContainer).nodes([
				canvas,
				debugInfo,
			]),
		])
	);

	// Initialize WebGL renderer after component is mounted
	let renderer: WglRenderer | null = null;
	let tilesUploaded = false;
	let currentMap: MapProject | null = null;

	// Panning state
	let panX = 0;
	let panY = 0;

	const TILE_SIZE = 64;

	/**
	 * Parse a tile ID with optional transformation flags.
	 * Format: "TILEID" or "TILEID:FLAGS"
	 * Flags: ! = flip horizontal, E = rot 90, S = rot 180, W = rot 270
	 * Returns [baseTileId, transformValue]
	 */
	function parseTileId(tileIdWithFlags: string): [string, number] {
		const colonIdx = tileIdWithFlags.indexOf(':');
		if (colonIdx === -1) {
			return [tileIdWithFlags, 0];
		}

		const baseTileId = tileIdWithFlags.substring(0, colonIdx);
		const flags = tileIdWithFlags.substring(colonIdx + 1);

		let transform = 0;

		// Parse rotation (E=90, S=180, W=270)
		if (flags.includes('E')) {
			transform = 1; // 90 degrees
		} else if (flags.includes('S')) {
			transform = 2; // 180 degrees
		} else if (flags.includes('W')) {
			transform = 3; // 270 degrees
		}

		// Parse flip (! = horizontal flip)
		if (flags.includes('!')) {
			transform |= 4; // Add flip flag
		}

		return [baseTileId, transform];
	}

	/**
	 * Draw a single tile with transformation parsing
	 */
	function drawTile(tileIdWithFlags: string, x: number, y: number) {
		const [baseTileId, transform] = parseTileId(tileIdWithFlags);
		renderer!.drawTileById(baseTileId, x, y, TILE_SIZE, transform);
	}

	function render() {
		if (!renderer) return;
		renderer.clear(0.1, 0.0, 0.1, 1.0); // dark magenta

		if (tilesUploaded && currentMap) {
			// Get canvas dimensions for culling
			const canvas = renderer.getCanvas();
			const canvasWidth = canvas.width;
			const canvasHeight = canvas.height;

			// Calculate visible tile range
			const startCol = Math.max(0, Math.floor(-panX / TILE_SIZE));
			const startRow = Math.max(0, Math.floor(-panY / TILE_SIZE));
			const endCol = Math.min(currentMap.width, Math.ceil((canvasWidth - panX) / TILE_SIZE));
			const endRow = Math.min(currentMap.height, Math.ceil((canvasHeight - panY) / TILE_SIZE));

			// Render visible tiles
			for (let row = startRow; row < endRow; row++) {
				const mapRow = currentMap.map[row];
				if (!mapRow) continue;

				for (let col = startCol; col < endCol; col++) {
					const cell = mapRow[col];
					if (!cell) continue;

					const x = col * TILE_SIZE + panX;
					const y = row * TILE_SIZE + panY;

					// Handle both string (single tile) and array (multiple layers)
					if (Array.isArray(cell)) {
						// Draw all layers from bottom to top
						for (const tileId of cell) {
							drawTile(tileId, x, y);
						}
					} else {
						drawTile(cell, x, y);
					}
				}
			}
		} else {
			// Fallback white square
			renderer.setColor(1.0, 1.0, 1.0, 1.0);
			renderer.drawRect(100 + panX, 100 + panY, 64, 64);
		}
	}

	// Initialize after a small delay to ensure canvas is in DOM
	setTimeout(() => {
		const canvasElement = canvas.element as HTMLCanvasElement;
		const container = canvasElement.parentElement!;
		renderer = new WglRenderer(canvasElement);

		// Use ResizeObserver to always match parent dimensions
		const resizeObserver = new ResizeObserver(() => {
			if (renderer) {
				renderer.resize();
				render();
			}
		});
		resizeObserver.observe(container);

		// Setup panning with right mouse button
		let isPanning = false;
		let lastX = 0;
		let lastY = 0;

		canvasElement.addEventListener('contextmenu', (e) => e.preventDefault());

		canvasElement.addEventListener('mousedown', (e) => {
			if (e.button === 2) { // Right mouse button
				isPanning = true;
				lastX = e.clientX;
				lastY = e.clientY;
				canvasElement.style.cursor = 'grabbing';
			}
		});

		// Use window events for mousemove/mouseup so panning works outside canvas
		window.addEventListener('mousemove', (e) => {
			if (isPanning) {
				const dx = e.clientX - lastX;
				const dy = e.clientY - lastY;
				lastX = e.clientX;
				lastY = e.clientY;

				panX += dx;
				panY += dy;
				render();
			}
		});

		window.addEventListener('mouseup', (e) => {
			if (e.button === 2 && isPanning) {
				isPanning = false;
				canvasElement.style.cursor = 'default';
			}
		});

		render();
	}, 0);

	// Upload palette and tiles when they become available
	new Effect(() => {
		const palette = AppState.palette.value;
		const tiles = AppState.tiles.value;
		const mapProject = AppState.mapProject.value;

		if (renderer && palette && tiles) {
			// Upload palette
			renderer.uploadPalette(palette);

			// Upload all tiles
			renderer.uploadAllTiles(tiles);
			tilesUploaded = true;
			currentMap = mapProject;
			render();
		}
	}).on([AppState.palette, AppState.tiles, AppState.mapProject]);

	// Load map project
	(async () => {
		await loadMapProject(await resolveTextResource('resources/maps/GREEN_1.json'));
	})();

	// Update debug info when state changes
	new Effect(() => {
		const mapProject = AppState.mapProject.value;
		const tiles = AppState.tiles.value;

		let info = '=== MAP DEBUG INFO ===\n\n';

		if (mapProject) {
			info += `Map: ${mapProject.name}\n`;
			info += `Size: ${mapProject.width} x ${mapProject.height}\n`;
			info += `Description: ${mapProject.description?.substring(0, 100)}...\n\n`;

			info += `Tilesets used:\n`;
			for (const asset of mapProject.use) {
				info += `  - ${asset.name} (tileset: ${asset.tileset}, palette: ${asset.palette})\n`;
			}
			info += '\n';
		} else {
			info += 'Map project: NOT LOADED\n\n';
		}

		if (tiles) {
			info += `Tiles loaded: ${tiles.size}\n\n`;

			let totalBytes = 0;
			const tileIds: string[] = [];

			for (const [id, tile] of tiles) {
				tileIds.push(id);
				totalBytes += tile.data.byteLength;
			}

			info += `Total tile data: ${(totalBytes / 1024).toFixed(2)} KB\n\n`;
			info += `Tile IDs (first 50):\n`;
			info += tileIds.slice(0, 50).join(', ');
			if (tileIds.length > 50) {
				info += `\n... and ${tileIds.length - 50} more`;
			}
		} else {
			info += 'Tiles: NOT LOADED\n';
		}

		debugInfo.text(info);
	}).on([AppState.mapProject, AppState.tiles]);

	return component;
}
