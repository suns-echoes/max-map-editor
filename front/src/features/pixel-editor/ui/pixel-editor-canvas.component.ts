/**
 * Pixel Editor Canvas Component
 *
 * Zoomable canvas for pixel-level tile editing.
 */

import { Div, Canvas } from '^reactive/reactive-node.elements.ts';
import { Effect } from '^reactive/effect.ts';
import { AppState } from '^state/app-state.ts';
import { xlog } from '^lib/xlog/xlog.ts';
import { PixelEditorState } from '../pixel-editor.state.ts';
import { TILE_SIZE, getPixelColor } from '../pixel-editor.types.ts';
import {
	pickColorAt,
	drawPixel,
	drawBrush,
	drawLine,
	floodFill,
	flushChanges,
} from '../pixel-editor.actions.ts';

import style from './pixel-editor-canvas.module.css';


/**
 * Pixel Editor Canvas - displays and allows editing of tile pixels.
 */
export function PixelEditorCanvas() {
	xlog.info('UI::PixelEditorCanvas');

	const canvas = Canvas().class(style.canvas);
	const ctx = canvas.element.getContext('2d');

	let isDrawing = false;
	let lastX = -1;
	let lastY = -1;

	/**
	 * Render the tile with current zoom level.
	 */
	function render() {
		if (!ctx) return;

		const tileId = PixelEditorState.editingTileId.value;
		const tiles = AppState.tiles.value;
		const palette = AppState.palette.value;
		const zoom = PixelEditorState.zoom.value;
		const showGrid = PixelEditorState.showGrid.value;

		const canvasSize = TILE_SIZE * zoom;
		canvas.element.width = canvasSize;
		canvas.element.height = canvasSize;

		// Clear canvas
		ctx.fillStyle = '#202020';
		ctx.fillRect(0, 0, canvasSize, canvasSize);

		if (!tileId || !tiles || !palette) {
			// Draw placeholder
			ctx.fillStyle = '#404040';
			ctx.textAlign = 'center';
			ctx.textBaseline = 'middle';
			ctx.font = '14px sans-serif';
			ctx.fillText('No tile selected', canvasSize / 2, canvasSize / 2);
			return;
		}

		const tile = tiles.get(tileId);
		if (!tile) return;

		// Draw pixels
		for (let y = 0; y < TILE_SIZE; y++) {
			for (let x = 0; x < TILE_SIZE; x++) {
				const colorIndex = getPixelColor(tile.data, x, y);
				const paletteOffset = colorIndex * 3;
				const r = palette[paletteOffset];
				const g = palette[paletteOffset + 1];
				const b = palette[paletteOffset + 2];

				ctx.fillStyle = `rgb(${r},${g},${b})`;
				ctx.fillRect(x * zoom, y * zoom, zoom, zoom);
			}
		}

		// Draw grid
		if (showGrid && zoom >= 2) {
			ctx.strokeStyle = 'rgba(255, 255, 255, 0.1)';
			ctx.lineWidth = 1;

			for (let i = 0; i <= TILE_SIZE; i++) {
				const pos = i * zoom;
				ctx.beginPath();
				ctx.moveTo(pos, 0);
				ctx.lineTo(pos, canvasSize);
				ctx.stroke();
				ctx.beginPath();
				ctx.moveTo(0, pos);
				ctx.lineTo(canvasSize, pos);
				ctx.stroke();
			}
		}

		// Highlight pixels to be replaced
		const secondaryColor = PixelEditorState.secondaryColor.value;

		if (PixelEditorState.tool.value === 'replace' && secondaryColor !== null) {
			ctx.strokeStyle = 'rgba(255, 100, 100, 0.8)';
			ctx.lineWidth = 2;

			for (let y = 0; y < TILE_SIZE; y++) {
				for (let x = 0; x < TILE_SIZE; x++) {
					const colorIndex = getPixelColor(tile.data, x, y);
					if (colorIndex === secondaryColor) {
						ctx.strokeRect(x * zoom + 1, y * zoom + 1, zoom - 2, zoom - 2);
					}
				}
			}
		}
	}

	/**
	 * Get pixel coordinates from mouse event.
	 */
	function getPixelCoords(e: MouseEvent): { x: number; y: number } {
		const rect = canvas.element.getBoundingClientRect();
		const zoom = PixelEditorState.zoom.value;
		const x = Math.floor((e.clientX - rect.left) / zoom);
		const y = Math.floor((e.clientY - rect.top) / zoom);
		return { x, y };
	}

	/**
	 * Handle mouse down on canvas.
	 */
	function handleMouseDown(e: MouseEvent) {
		const tileId = PixelEditorState.editingTileId.value;
		if (!tileId) return;

		const { x, y } = getPixelCoords(e);
		const tool = PixelEditorState.tool.value;

		if (tool === 'picker') {
			pickColorAt(tileId, x, y);
		} else if (tool === 'pencil') {
			isDrawing = true;
			lastX = x;
			lastY = y;

			if (e.shiftKey) {
				// Flood fill on shift+click
				floodFill(tileId, x, y);
			} else if (PixelEditorState.brushSize.value > 1) {
				drawBrush(tileId, x, y);
			} else {
				drawPixel(tileId, x, y);
			}
			render();
		} else if (tool === 'replace') {
			// Pick secondary color on click
			const colorIndex = pickColorAt(tileId, x, y);
			if (colorIndex !== null) {
				PixelEditorState.selectSecondaryColor(colorIndex);
			}
		}
	}

	/**
	 * Handle mouse move on canvas.
	 */
	function handleMouseMove(e: MouseEvent) {
		if (!isDrawing) return;

		const tileId = PixelEditorState.editingTileId.value;
		if (!tileId) return;

		const { x, y } = getPixelCoords(e);

		if (x !== lastX || y !== lastY) {
			if (PixelEditorState.brushSize.value > 1) {
				drawBrush(tileId, x, y);
			} else {
				// Draw line for smooth strokes
				drawLine(tileId, lastX, lastY, x, y);
			}
			lastX = x;
			lastY = y;
			render();
		}
	}

	/**
	 * Handle mouse up.
	 */
	function handleMouseUp() {
		if (isDrawing) {
			isDrawing = false;
			lastX = -1;
			lastY = -1;
			flushChanges();
		}
	}

	// Mouse event handlers
	canvas.on('mousedown', handleMouseDown);
	canvas.on('mousemove', handleMouseMove);
	canvas.on('mouseup', handleMouseUp);
	canvas.on('mouseleave', handleMouseUp);

	// Effect: Re-render when tile or state changes
	new Effect(function renderCanvas() {
		render();
	}, { strong: true }).on([
		PixelEditorState.editingTileId,
		PixelEditorState.zoom,
		PixelEditorState.showGrid,
		PixelEditorState.tool,
		PixelEditorState.secondaryColor,
		AppState.tiles,
		AppState.palette,
	]);

	return (
		Div().class(style.canvasContainer).nodes([
			canvas,
		])
	);
}
