/**
 * Pass Table Panel Component
 *
 * UI panel for editing pass values of tiles.
 * Shows pass value selector and displays current tile pass info.
 */

import { AppState } from '^state/app-state.ts';
import { Effect } from '^reactive/effect.ts';
import { Section, Div, Button, Canvas } from '^reactive/reactive-node.elements.ts';
import { xlog } from '^lib/xlog/xlog.ts';
import { PassTableState } from '../pass-table.state.ts';
import {
	type PassValue,
	PASS_VALUES,
	PASS_INFO,
	getPassValueFromType,
} from '../pass-table.types.ts';
import { cycleTilePassValue, getPassStats } from '../pass-table.actions.ts';

import style from './pass-table-panel.module.css';


/**
 * Render a single tile onto a canvas with pass value overlay.
 */
function renderTileWithPassOverlay(
	canvas: HTMLCanvasElement,
	tileData: Uint8Array,
	palette: Uint8Array,
	passValue: PassValue,
	overlayOpacity: number
): void {
	const ctx = canvas.getContext('2d');
	if (!ctx) return;

	// Render tile
	const imageData = ctx.createImageData(64, 64);
	for (let i = 0; i < tileData.length; i++) {
		const paletteIndex = tileData[i];
		const paletteOffset = paletteIndex * 3;
		imageData.data[i * 4] = palette[paletteOffset];
		imageData.data[i * 4 + 1] = palette[paletteOffset + 1];
		imageData.data[i * 4 + 2] = palette[paletteOffset + 2];
		imageData.data[i * 4 + 3] = 255;
	}
	ctx.putImageData(imageData, 0, 0);

	// Draw pass value overlay
	const info = PASS_INFO[passValue];
	ctx.fillStyle = info.color;
	ctx.globalAlpha = overlayOpacity;
	ctx.fillRect(0, 0, 64, 64);
	ctx.globalAlpha = 1;
}


/**
 * Pass Table Panel component - displays pass value editor.
 */
export function PassTablePanel() {
	xlog.info('UI::PassTablePanel');

	// UI references
	const statsContainer = Div().class(style.stats);
	const tileGridContainer = Div().class(style.tileGrid);

	/**
	 * Create pass value selector buttons.
	 */
	function createPassValueSelector() {
		const buttons: ReturnType<typeof Button>[] = [];

		const passValues: PassValue[] = [
			PASS_VALUES.LAND,
			PASS_VALUES.WATER,
			PASS_VALUES.SHORE,
			PASS_VALUES.OBSTRUCTION,
		];

		for (const value of passValues) {
			const info = PASS_INFO[value];
			const btn = Button(info.label)
				.class(style.passButton)
				.attr('title', info.description);

			btn.element.style.backgroundColor = info.color;

			btn.on('click', () => {
				PassTableState.selectPassValue(value);
			});

			buttons.push(btn);
		}

		return Div().class(style.passSelector).nodes(buttons);
	}

	/**
	 * Create overlay controls.
	 */
	function createOverlayControls() {
		const toggleBtn = Button('Toggle Overlay').class(style.controlButton);
		toggleBtn.on('click', () => PassTableState.toggleOverlay());

		return Div().class(style.overlayControls).nodes([toggleBtn]);
	}

	/**
	 * Update stats display.
	 */
	function updateStats() {
		const stats = getPassStats();
		statsContainer.nodes([
			Div().class(style.statItem).nodes([
				Div().class(style.statLabel).text('Land:'),
				Div().class(style.statValue).text(String(stats.land)),
			]),
			Div().class(style.statItem).nodes([
				Div().class(style.statLabel).text('Water:'),
				Div().class(style.statValue).text(String(stats.water)),
			]),
			Div().class(style.statItem).nodes([
				Div().class(style.statLabel).text('Shore:'),
				Div().class(style.statValue).text(String(stats.shore)),
			]),
			Div().class(style.statItem).nodes([
				Div().class(style.statLabel).text('Obstruction:'),
				Div().class(style.statValue).text(String(stats.obstruction)),
			]),
			Div().class(style.statItem).nodes([
				Div().class(style.statLabel).text('Total:'),
				Div().class(style.statValue).text(String(stats.total)),
			]),
		]);
	}

	/**
	 * Render the tile grid with pass value overlays.
	 */
	function renderTileGrid() {
		const tiles = AppState.tiles.value;
		const palette = AppState.palette.value;
		const showOverlay = PassTableState.showOverlay.value;
		const overlayOpacity = PassTableState.overlayOpacity.value;
		const selectedPassValue = PassTableState.selectedPassValue.value;

		if (!tiles || !palette) {
			tileGridContainer.nodes([
				Div().class(style.noTiles).text('No tiles loaded')
			]);
			return;
		}

		const tileNodes: ReturnType<typeof Div>[] = [];

		for (const [tileId, tile] of tiles) {
			const passValue = getPassValueFromType(tile.props.type);
			const info = PASS_INFO[passValue];

			const canvas = Canvas();
			canvas.element.width = 64;
			canvas.element.height = 64;

			if (showOverlay) {
				renderTileWithPassOverlay(
					canvas.element,
					tile.data,
					palette,
					passValue,
					overlayOpacity
				);
			} else {
				// Just render tile without overlay
				const ctx = canvas.element.getContext('2d');
				if (ctx) {
					const imageData = ctx.createImageData(64, 64);
					for (let i = 0; i < tile.data.length; i++) {
						const paletteIndex = tile.data[i];
						const paletteOffset = paletteIndex * 3;
						imageData.data[i * 4] = palette[paletteOffset];
						imageData.data[i * 4 + 1] = palette[paletteOffset + 1];
						imageData.data[i * 4 + 2] = palette[paletteOffset + 2];
						imageData.data[i * 4 + 3] = 255;
					}
					ctx.putImageData(imageData, 0, 0);
				}
			}

			const isSelected = passValue === selectedPassValue;
			const tileItem = Div().class(style.tileItem).nodes([
				canvas,
				Div()
					.class(style.passLabel)
					.text(info.label)
					.attr('style', `background-color: ${info.color}`)
			]);

			if (isSelected) {
				tileItem.element.classList.add(style.selected);
			}

			// Click to cycle pass value
			tileItem.on('click', () => {
				cycleTilePassValue(tileId);
			});

			tileItem.element.dataset.tileId = tileId;
			tileNodes.push(tileItem);
		}

		tileGridContainer.nodes(tileNodes);
		updateStats();
	}

	// Effect: Re-render when tiles, palette, or overlay settings change
	new Effect(function onDataChange() {
		renderTileGrid();
	}, { strong: true }).on([
		AppState.tiles,
		AppState.palette,
		PassTableState.showOverlay,
		PassTableState.overlayOpacity,
	]);

	// Effect: Update selection when pass value changes
	new Effect(function onPassValueChange() {
		const selectedPassValue = PassTableState.selectedPassValue.value;
		const items = tileGridContainer.element.querySelectorAll(`.${style.tileItem}`);

		items.forEach(item => {
			const el = item as HTMLElement;
			const tileId = el.dataset.tileId;
			if (!tileId) return;

			const tiles = AppState.tiles.value;
			if (!tiles) return;

			const tile = tiles.get(tileId);
			if (!tile) return;

			const passValue = getPassValueFromType(tile.props.type);
			if (passValue === selectedPassValue) {
				el.classList.add(style.selected);
			} else {
				el.classList.remove(style.selected);
			}
		});
	}, { strong: true }).on([PassTableState.selectedPassValue]);

	return (
		Section('pass-table-panel').class(style.passTablePanel).nodes([
			Div().class(style.header).text('Pass Table Editor'),
			createPassValueSelector(),
			createOverlayControls(),
			statsContainer,
			tileGridContainer,
		])
	);
}
