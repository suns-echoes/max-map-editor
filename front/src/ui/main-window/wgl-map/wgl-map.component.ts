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
	let tileUploaded = false;

	// Panning state
	let panX = 0;
	let panY = 0;

	function render() {
		if (!renderer) return;
		renderer.clear(0.1, 0.0, 0.1, 1.0); // dark magenta

		if (tileUploaded) {
			renderer.drawTile(100 + panX, 100 + panY, 64); // Draw tile with pan offset
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

		canvasElement.addEventListener('mousemove', (e) => {
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

		canvasElement.addEventListener('mouseup', (e) => {
			if (e.button === 2) {
				isPanning = false;
				canvasElement.style.cursor = 'default';
			}
		});

		canvasElement.addEventListener('mouseleave', () => {
			isPanning = false;
			canvasElement.style.cursor = 'default';
		});

		render();
	}, 0);

	// Upload palette and tile when they become available
	new Effect(() => {
		const palette = AppState.palette.value;
		const tiles = AppState.tiles.value;

		if (renderer && palette && tiles) {
			// Upload palette
			renderer.uploadPalette(palette);

			// Get first tile and upload it
			const firstTileId = tiles.keys().next().value;
			if (firstTileId) {
				const firstTile = tiles.get(firstTileId);
				if (firstTile) {
					console.log(`Uploading tile: ${firstTileId}`);
					renderer.uploadTile(firstTile.data);
					tileUploaded = true;
					render();
				}
			}
		}
	}).on([AppState.palette, AppState.tiles]);

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
