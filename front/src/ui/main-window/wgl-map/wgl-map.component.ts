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

	// Zoom state (1.0 = 100%, 2.0 = 200%, 0.5 = 50%)
	let zoom = 1.0;
	const MIN_ZOOM = 0.05; // Will be dynamically adjusted to fit whole map
	const MAX_ZOOM = 2.0;  // 2:1 zoom

	// Cursor state (tile coordinates)
	let cursorCol = -1;
	let cursorRow = -1;
	let cursorOpacity = 0.5;
	const CURSOR_BLINK_PERIOD = 1500; // ms for full blink cycle

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
	function drawTile(tileIdWithFlags: string, x: number, y: number, tileSize: number) {
		const [baseTileId, transform] = parseTileId(tileIdWithFlags);
		renderer!.drawTileById(baseTileId, x, y, tileSize, transform);
	}

	/**
	 * Calculate minimum zoom to fit whole map on screen
	 */
	function getMinZoom(): number {
		if (!renderer || !currentMap) return MIN_ZOOM;
		const canvas = renderer.getCanvas();
		const mapPixelWidth = currentMap.width * TILE_SIZE;
		const mapPixelHeight = currentMap.height * TILE_SIZE;
		const zoomX = canvas.width / mapPixelWidth;
		const zoomY = canvas.height / mapPixelHeight;
		return Math.min(zoomX, zoomY, 1.0); // Don't go above 1.0 for min
	}

	function render() {
		if (!renderer) return;
		renderer.clear(0.1, 0.0, 0.1, 1.0); // dark magenta

		if (tilesUploaded && currentMap) {
			// Get canvas dimensions for culling
			const canvas = renderer.getCanvas();
			const canvasWidth = canvas.width;
			const canvasHeight = canvas.height;

			// Calculate scaled tile size
			const scaledTileSize = TILE_SIZE * zoom;

			// Calculate visible tile range (accounting for zoom)
			const startCol = Math.max(0, Math.floor(-panX / scaledTileSize));
			const startRow = Math.max(0, Math.floor(-panY / scaledTileSize));
			const endCol = Math.min(currentMap.width, Math.ceil((canvasWidth - panX) / scaledTileSize));
			const endRow = Math.min(currentMap.height, Math.ceil((canvasHeight - panY) / scaledTileSize));

			// Render visible tiles
			for (let row = startRow; row < endRow; row++) {
				const mapRow = currentMap.map[row];
				if (!mapRow) continue;

				for (let col = startCol; col < endCol; col++) {
					const cell = mapRow[col];
					if (!cell) continue;

					const x = col * scaledTileSize + panX;
					const y = row * scaledTileSize + panY;

					// Handle both string (single tile) and array (multiple layers)
					if (Array.isArray(cell)) {
						// Draw all layers from bottom to top
						for (const tileId of cell) {
							drawTile(tileId, x, y, scaledTileSize);
						}
					} else {
						drawTile(cell, x, y, scaledTileSize);
					}
				}
			}

			// Render cursor if within map bounds
			if (cursorCol >= 0 && cursorRow >= 0 && cursorCol < currentMap.width && cursorRow < currentMap.height) {
				const cursorX = cursorCol * scaledTileSize + panX;
				const cursorY = cursorRow * scaledTileSize + panY;

				// Calculate cursor line width: 2px at 1:1, 4px at 2:1, 1px at 1:2, never less than 1px
				const cursorWidth = Math.max(1, Math.round(zoom * 2));

				// Draw cursor rectangle outline with additive blending
				renderer.enableAdditiveBlend();
				renderer.setColor(cursorOpacity, cursorOpacity, cursorOpacity, 1.0); // White with varying intensity

				// Top edge
				renderer.drawRect(cursorX, cursorY, scaledTileSize, cursorWidth);
				// Bottom edge
				renderer.drawRect(cursorX, cursorY + scaledTileSize - cursorWidth, scaledTileSize, cursorWidth);
				// Left edge
				renderer.drawRect(cursorX, cursorY, cursorWidth, scaledTileSize);
				// Right edge
				renderer.drawRect(cursorX + scaledTileSize - cursorWidth, cursorY, cursorWidth, scaledTileSize);

				renderer.disableBlend();
			}
		} else {
			// Fallback white square
			renderer.setColor(1.0, 1.0, 1.0, 1.0);
			renderer.drawRect(100 + panX, 100 + panY, 64 * zoom, 64 * zoom);
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
				const canvas = renderer.getCanvas();
				const oldWidth = canvas.width;
				const oldHeight = canvas.height;

				renderer.resize();

				const newWidth = canvas.width;
				const newHeight = canvas.height;

				// Adjust pan to keep the same map center visible
				panX += (newWidth - oldWidth) / 2;
				panY += (newHeight - oldHeight) / 2;

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

		// Mouse wheel for zooming
		canvasElement.addEventListener('wheel', (e) => {
			e.preventDefault();

			// Get mouse position relative to canvas
			const rect = canvasElement.getBoundingClientRect();
			const mouseX = e.clientX - rect.left;
			const mouseY = e.clientY - rect.top;

			// Calculate zoom factor
			const zoomFactor = e.deltaY > 0 ? 0.9 : 1.1;
			const newZoom = Math.max(getMinZoom(), Math.min(MAX_ZOOM, zoom * zoomFactor));

			if (newZoom !== zoom) {
				// Adjust pan to zoom towards mouse position
				const scale = newZoom / zoom;
				panX = mouseX - (mouseX - panX) * scale;
				panY = mouseY - (mouseY - panY) * scale;
				zoom = newZoom;
				render();
			}
		}, { passive: false });

		// Track mouse position for cursor
		canvasElement.addEventListener('mousemove', (e) => {
			if (isPanning) return; // Don't update cursor while panning

			const rect = canvasElement.getBoundingClientRect();
			const mouseX = e.clientX - rect.left;
			const mouseY = e.clientY - rect.top;

			// Convert to tile coordinates
			const scaledTileSize = TILE_SIZE * zoom;
			const newCursorCol = Math.floor((mouseX - panX) / scaledTileSize);
			const newCursorRow = Math.floor((mouseY - panY) / scaledTileSize);

			if (newCursorCol !== cursorCol || newCursorRow !== cursorRow) {
				cursorCol = newCursorCol;
				cursorRow = newCursorRow;
				render();
			}
		});

		// Hide cursor when mouse leaves canvas
		canvasElement.addEventListener('mouseleave', () => {
			cursorCol = -1;
			cursorRow = -1;
			render();
		});

		// Color cycling animation data
		// Format: { startIndex, endIndex, direction, intervalMs }
		const colorCycleData = [
			{ start: 9, end: 12, direction: 0, fps: 9 },
			{ start: 13, end: 16, direction: 1, fps: 6 },
			{ start: 17, end: 20, direction: 1, fps: 9 },
			{ start: 21, end: 24, direction: 1, fps: 6 },
			{ start: 25, end: 30, direction: 1, fps: 2 },
			{ start: 31, end: 31, direction: 1, fps: 6 },
			{ start: 96, end: 102, direction: 1, fps: 8 },
			{ start: 103, end: 109, direction: 1, fps: 8 },
			{ start: 110, end: 116, direction: 1, fps: 10 },
			{ start: 117, end: 122, direction: 1, fps: 6 },
			{ start: 123, end: 127, direction: 1, fps: 6 },
		];

		// Track last cycle time for each color range
		const lastCycleTime = colorCycleData.map(() => 0);

		// Animation loop for color cycling and cursor blinking
		let animationFrameId: number;
		function animateColorCycling(timestamp: number) {
			if (!renderer) {
				animationFrameId = requestAnimationFrame(animateColorCycling);
				return;
			}

			let needsUpdate = false;

			// Update color cycling
			for (let i = 0; i < colorCycleData.length; i++) {
				const cycle = colorCycleData[i];
				const intervalMs = 1000 / cycle.fps;

				if (timestamp - lastCycleTime[i] >= intervalMs) {
					renderer.cycleColors(cycle.start, cycle.end, cycle.direction);
					lastCycleTime[i] = timestamp;
					needsUpdate = true;
				}
			}

			// Update cursor opacity (smooth fade between 0.3 and 0.7)
			const newCursorOpacity = 0.5 + 0.2 * Math.sin(timestamp / CURSOR_BLINK_PERIOD * Math.PI * 2);
			if (Math.abs(newCursorOpacity - cursorOpacity) > 0.01) {
				cursorOpacity = newCursorOpacity;
				needsUpdate = true;
			}

			if (needsUpdate) {
				renderer.updatePaletteTexture();
				render();
			}

			animationFrameId = requestAnimationFrame(animateColorCycling);
		}

		// Start the animation loop
		animationFrameId = requestAnimationFrame(animateColorCycling);

		render();
	}, 0);

	// Upload palette and tiles when they become available
	new Effect(() => {
		const palette = AppState.palette.value;
		const tiles = AppState.tiles.value;
		const mapProject = AppState.mapProject.value;

		if (renderer && palette && tiles && mapProject) {
			// Upload palette
			renderer.uploadPalette(palette);

			// Upload all tiles
			renderer.uploadAllTiles(tiles);
			tilesUploaded = true;
			currentMap = mapProject;

			// Center the map in the viewport
			const canvas = renderer.getCanvas();
			const mapPixelWidth = mapProject.width * TILE_SIZE;
			const mapPixelHeight = mapProject.height * TILE_SIZE;
			panX = (canvas.width - mapPixelWidth) / 2;
			panY = (canvas.height - mapPixelHeight) / 2;

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
