import { Canvas, Div, Pre, Section } from '^reactive/reactive-node.elements.ts';
import { printDebugInfo } from '^lib/debug/debug.ts';
import { AppState } from '^state/app-state.ts';
import { Effect } from '^reactive/effect.ts';
import { loadMapProject } from '^actions/load-map-project/load-map-project.ts';
import { resolveTextResource } from '^tauri-apps/api/path.ts';
import { WglMap } from './wgl/wgl-map.ts';

import style from './wgl-map.module.css';

// Default map to load on startup
const DEFAULT_MAP_PATH = 'resources/maps/GREEN_1.json';


export function WGLMap() {
	printDebugInfo('UI::WGLMap');

	const canvas = Canvas('map-canvas');
	const debugInfo = import.meta.env.DEV ? Pre().class(style.debugInfo) : null;

	const component = (
		Section('wgl-map').class(style.wglMap).nodes([
			Div().class(style.canvasContainer).nodes(
				[canvas, debugInfo].filter(Boolean) as [typeof canvas, ...typeof canvas[]]
			),
		])
	);

	let wglMap: WglMap | null = null;

	// Initialize after component is mounted
	setTimeout(() => {
		const canvasElement = canvas.element as HTMLCanvasElement;
		const container = canvasElement.parentElement;
		if (!container) {
			console.error('WGLMap: Canvas has no parent element');
			return;
		}

		// Create the optimized WebGL map renderer
		wglMap = new WglMap(canvasElement);
		AppState.wglMap.value = wglMap;

		// ResizeObserver for canvas resizing
		let canvasRect = canvasElement.getBoundingClientRect();  // Cache for mouse events
		const resizeObserver = new ResizeObserver(() => {
			if (wglMap) {
				wglMap.onCanvasResize();
				canvasRect = canvasElement.getBoundingClientRect();  // Update cache on resize
			}
		});
		resizeObserver.observe(container);

		// Panning with right mouse button
		let isPanning = false;
		let lastX = 0;
		let lastY = 0;

		canvasElement.addEventListener('contextmenu', (e) => e.preventDefault());

		canvasElement.addEventListener('mousedown', (e) => {
			if (e.button === 2) {
				isPanning = true;
				lastX = e.clientX;
				lastY = e.clientY;
				canvasElement.style.cursor = 'grabbing';
			} else if (e.button === 1) {
				// Middle click: reset zoom to 1:1
				e.preventDefault();
				if (wglMap) {
					// Use moveCamera with a zoom reset (handled internally)
					wglMap.moveCamera(0, 0, 0);
				}
			}
		});

		const handleMouseMove = (e: MouseEvent) => {
			if (isPanning && wglMap) {
				const dx = e.clientX - lastX;
				const dy = e.clientY - lastY;
				lastX = e.clientX;
				lastY = e.clientY;
				wglMap.moveCamera(dx, dy, 0);
			}
		};

		const handleMouseUp = (e: MouseEvent) => {
			if (e.button === 2 && isPanning) {
				isPanning = false;
				canvasElement.style.cursor = 'default';
			}
		};

		window.addEventListener('mousemove', handleMouseMove);
		window.addEventListener('mouseup', handleMouseUp);

		// Mouse wheel for zooming
		canvasElement.addEventListener('wheel', (e) => {
			e.preventDefault();
			if (!wglMap) return;

			const cursorX = e.clientX - canvasRect.left;
			const cursorY = e.clientY - canvasRect.top;

			// Positive deltaY = scroll down = zoom out
			const dz = e.deltaY > 0 ? -1 : 1;
			wglMap.moveCamera(0, 0, dz, cursorX, cursorY);
		}, { passive: false });

		// Track mouse for cursor highlight
		canvasElement.addEventListener('mousemove', (e) => {
			if (isPanning || !wglMap) return;

			const screenX = e.clientX - canvasRect.left;
			const screenY = e.clientY - canvasRect.top;
			wglMap.moveCursor(screenX, screenY);
		});

		canvasElement.addEventListener('mouseleave', () => {
			if (wglMap) {
				wglMap.moveCursor(-1000, -1000); // Move cursor off-screen
			}
		});

		// Enable animation (water effects, etc.)
		wglMap.enableAnimation();

		// Cleanup
		component.cleanup(() => {
			resizeObserver.disconnect();
			window.removeEventListener('mousemove', handleMouseMove);
			window.removeEventListener('mouseup', handleMouseUp);

			if (wglMap) {
				wglMap.disableAnimation();
				wglMap.cleanup();
				wglMap = null;
				AppState.wglMap.value = null;
			}
		});
	}, 0);

	// Load map project on startup
	(async () => {
		try {
			await loadMapProject(await resolveTextResource(DEFAULT_MAP_PATH));
		} catch (error) {
			console.error('Failed to load map project:', error);
		}
	})();

	// Debug info panel (dev mode only)
	if (import.meta.env.DEV && debugInfo) {
		const debugEffect = new Effect(() => {
			const mapProject = AppState.mapProject.value;
			const tiles = AppState.tiles.value;

			let info = '=== MAP DEBUG INFO ===\n\n';

			if (mapProject) {
				info += `Map: ${mapProject.name}\n`;
				info += `Size: ${mapProject.width} x ${mapProject.height}\n`;
				info += `Description: ${mapProject.description?.substring(0, 100)}...\n\n`;
			} else {
				info += 'Map project: NOT LOADED\n\n';
			}

			if (tiles) {
				info += `Tiles loaded: ${tiles.size}\n`;
				let totalBytes = 0;
				for (const [, tile] of tiles) {
					totalBytes += tile.data.byteLength;
				}
				info += `Total tile data: ${(totalBytes / 1024).toFixed(2)} KB\n`;
			} else {
				info += 'Tiles: NOT LOADED\n';
			}

			debugInfo.text(info);
		}).on([AppState.mapProject, AppState.tiles]);

		component.cleanup(() => {
			debugEffect.dispose();
		});
	}

	return component;
}
