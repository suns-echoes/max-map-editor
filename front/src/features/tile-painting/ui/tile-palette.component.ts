import { AppState } from '^state/app-state.ts';
import { EditorState } from '^state/editor-state.ts';
import { Effect } from '^reactive/effect.ts';
import { Section, Div, Canvas, Button } from '^reactive/reactive-node.elements.ts';
import { xlog } from '^lib/xlog/xlog.ts';

import style from './tile-palette.module.css';


/** Tile type filter options */
type TileFilter = 'all' | 'inUse' | 'water' | 'shore' | 'land' | 'obstruction';


/**
 * Render a single tile onto a canvas.
 */
function renderTileToCanvas(
	canvas: HTMLCanvasElement,
	tileData: Uint8Array,
	palette: Uint8Array
): void {
	const ctx = canvas.getContext('2d');
	if (!ctx) return;

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
}


/**
 * Tile Palette component - displays available tiles for selection.
 */
export function TilePalette() {
	xlog.info('UI::TilePalette');

	// Filter state
	let currentFilter: TileFilter = 'all';

	// UI references
	const recentTilesContainer = Div().class(style.recentTiles);
	const tileGridContainer = Div().class(style.tileGrid);
	const filterButtons: Map<TileFilter, ReturnType<typeof Button>> = new Map();

	// Create filter buttons
	const filters: TileFilter[] = ['all', 'inUse', 'water', 'shore', 'land', 'obstruction'];
	const filterButtonNodes = filters.map(filter => {
		const btn = Button(filter).class(style.filterButton);
		if (filter === 'all') {
			btn.element.classList.add(style.active);
		}
		btn.on('click', () => setFilter(filter));
		filterButtons.set(filter, btn);
		return btn;
	});
	const filterButtonsContainer = Div().class(style.filterButtons).nodes(filterButtonNodes);

	function setFilter(filter: TileFilter) {
		currentFilter = filter;
		// Update button states
		filterButtons.forEach((btn, key) => {
			if (key === filter) {
				btn.element.classList.add(style.active);
			} else {
				btn.element.classList.remove(style.active);
			}
		});
		// Re-render tiles
		renderTileGrid();
	}

	/**
	 * Render the tile grid based on current filter.
	 * Only shows tiles that are used in the current map.
	 */
	function renderTileGrid() {
		const tiles = AppState.tiles.value;
		const palette = AppState.palette.value;
		const mapProject = AppState.mapProject.value;
		const selectedTileId = EditorState.selectedTile.value;

		if (!tiles || !palette) {
			tileGridContainer.nodes([
				Div().class(style.noTiles).text('No tiles loaded')
			]);
			return;
		}

		// Get unique tile IDs used in the map from mapProject.map
		const usedTileIds = new Set<string>();
		if (mapProject?.map) {
			for (const row of mapProject.map) {
				for (const cell of row) {
					if (Array.isArray(cell)) {
						// Layered cell - each layer is a tile reference
						for (const layerCell of cell) {
							const tileId = layerCell.split(':')[0]; // Remove transformation suffix
							usedTileIds.add(tileId);
						}
					} else if (typeof cell === 'string') {
						const tileId = cell.split(':')[0]; // Remove transformation suffix
						usedTileIds.add(tileId);
					}
				}
			}
		}

		const tileNodes: ReturnType<typeof Div>[] = [];

		// Only show tiles that are used in the map
		for (const tileId of usedTileIds) {
			const tile = tiles.get(tileId);
			console.log(tileId, tile);
			if (!tile?.data) continue;

			// Apply filter
			if (currentFilter !== 'all' && tile.props.type !== currentFilter) {
				continue;
			}

			const canvas = Canvas();
			canvas.element.width = 64;
			canvas.element.height = 64;
			renderTileToCanvas(canvas.element, tile.data, palette);

			const isSelected = tileId === selectedTileId;
			const tileItem = Div().class(style.tileItem).nodes([
				canvas,
				Div()
					.class(style.tileTypeLabel)
					.text(tile.)
			]);

			if (isSelected) {
				tileItem.element.classList.add(style.selected);
			}

			tileItem.on('click', () => {
				EditorState.selectTile(tileId);
			});

			// Store tile ID for later reference
			tileItem.element.dataset.tileId = tileId;

			tileNodes.push(tileItem);
		}

		tileGridContainer.nodes(tileNodes);
	}

	/**
	 * Render recent tiles.
	 */
	function renderRecentTiles() {
		const tiles = AppState.tiles.value;
		const palette = AppState.palette.value;
		const recentIds = EditorState.recentTiles.value;
		const selectedTileId = EditorState.selectedTile.value;

		if (!tiles || !palette || recentIds.length === 0) {
			recentTilesContainer.nodes([]);
			return;
		}

		const recentNodes: ReturnType<typeof Div>[] = [];
		for (const tileId of recentIds) {
			const tile = tiles.get(tileId);
			if (!tile) continue;

			const canvas = Canvas();
			canvas.element.width = 64;
			canvas.element.height = 64;
			renderTileToCanvas(canvas.element, tile.data, palette);

			const isSelected = tileId === selectedTileId;
			const node = Div().class(style.tileItem).nodes([canvas]);
			if (isSelected) {
				node.element.classList.add(style.selected);
			}
			node.on('click', () => {
				EditorState.selectTile(tileId);
			});
			recentNodes.push(node);
		}

		recentTilesContainer.nodes(recentNodes);
	}

	/**
	 * Update selection highlighting without full re-render.
	 */
	function updateSelectionHighlight() {
		const selectedTileId = EditorState.selectedTile.value;

		// Update tile grid
		const gridItems = tileGridContainer.element.querySelectorAll(`.${style.tileItem}`);
		gridItems.forEach(item => {
			const el = item as HTMLElement;
			if (el.dataset.tileId === selectedTileId) {
				el.classList.add(style.selected);
			} else {
				el.classList.remove(style.selected);
			}
		});

		// Re-render recent tiles (small set, OK to re-render)
		renderRecentTiles();
	}

	// Effect: Re-render when tiles, palette, or mapProject change
	new Effect(function onTilesChange() {
		renderTileGrid();
		renderRecentTiles();
	}, { strong: true }).on([AppState.tiles, AppState.palette, AppState.mapProject]);

	// Effect: Update selection highlight when selected tile changes
	new Effect(function onSelectionChange() {
		updateSelectionHighlight();
	}, { strong: true }).on([EditorState.selectedTile, EditorState.recentTiles]);

	return (
		Section('tile-palette').class(style.tilePalette).nodes([
			Div().class(style.header).text('Tiles'),
			filterButtonsContainer,
			recentTilesContainer,
			tileGridContainer,
		])
	);
}
