/**
 * Pixel Editor Color Bar Component
 *
 * Color selection from palette for pixel editing.
 */

import { Div } from '^reactive/reactive-node.elements.ts';
import { Effect } from '^reactive/effect.ts';
import { AppState } from '^state/app-state.ts';
import { xlog } from '^lib/xlog/xlog.ts';
import { PixelEditorState } from '../pixel-editor.state.ts';
import { getPaletteColor, rgbToHex } from '^src/features/palette-editor/index.ts';

import style from './pixel-editor-colorbar.module.css';


/**
 * Pixel Editor Color Bar - palette-based color selection.
 */
export function PixelEditorColorBar() {
	xlog.info('UI::PixelEditorColorBar');

	// Color preview boxes
	const primaryPreview = Div().class(style.colorPreview).class(style.primary);
	const secondaryPreview = Div().class(style.colorPreview).class(style.secondary);

	const colorPreviewContainer = Div().class(style.colorPreviewContainer).nodes([
		Div().class(style.previewLabel).text('Primary'),
		primaryPreview,
		Div().class(style.previewLabel).text('Secondary'),
		secondaryPreview,
		Div().class(style.swapButton).text('⇄').attr('title', 'Swap colors'),
	]);

	// Swap button action
	colorPreviewContainer.element.querySelector(`.${style.swapButton}`)?.addEventListener('click', () => {
		PixelEditorState.swapColors();
	});

	// Palette mini-grid
	const paletteGrid = Div().class(style.paletteGrid);

	/**
	 * Render palette color cells.
	 */
	function renderPalette() {
		const palette = AppState.palette.value;
		const selectedColor = PixelEditorState.selectedColor.value;
		const secondaryColor = PixelEditorState.secondaryColor.value;

		if (!palette) {
			paletteGrid.nodes([
				Div().class(style.noPalette).text('No palette')
			]);
			return;
		}

		const cells: ReturnType<typeof Div>[] = [];

		for (let i = 0; i < 256; i++) {
			const color = getPaletteColor(palette, i);
			const hex = rgbToHex(color.r, color.g, color.b);

			const cell = Div().class(style.colorCell);
			cell.element.style.backgroundColor = hex;
			cell.element.dataset.index = String(i);
			cell.element.title = `${i}: ${hex}`;

			if (i === selectedColor) {
				cell.element.classList.add(style.selected);
			}
			if (i === secondaryColor) {
				cell.element.classList.add(style.secondarySelected);
			}

			cell.on('click', (e: MouseEvent) => {
				if (e.shiftKey || e.ctrlKey) {
					PixelEditorState.selectSecondaryColor(i);
				} else {
					PixelEditorState.selectColor(i);
				}
			});

			cell.on('contextmenu', (e: MouseEvent) => {
				e.preventDefault();
				PixelEditorState.selectSecondaryColor(i);
			});

			cells.push(cell);
		}

		paletteGrid.nodes(cells);
	}

	/**
	 * Update color previews.
	 */
	function updatePreviews() {
		const palette = AppState.palette.value;
		if (!palette) return;

		const primaryIdx = PixelEditorState.selectedColor.value;
		const secondaryIdx = PixelEditorState.secondaryColor.value;

		const primaryColor = getPaletteColor(palette, primaryIdx);
		primaryPreview.element.style.backgroundColor = rgbToHex(primaryColor.r, primaryColor.g, primaryColor.b);
		primaryPreview.element.title = `Primary: ${primaryIdx}`;

		if (secondaryIdx !== null) {
			const secondaryColor = getPaletteColor(palette, secondaryIdx);
			secondaryPreview.element.style.backgroundColor = rgbToHex(secondaryColor.r, secondaryColor.g, secondaryColor.b);
			secondaryPreview.element.title = `Secondary: ${secondaryIdx}`;
		} else {
			secondaryPreview.element.style.backgroundColor = 'transparent';
			secondaryPreview.element.title = 'Secondary: none';
		}
	}

	/**
	 * Update palette cell selection states.
	 */
	function updateSelection() {
		const selectedColor = PixelEditorState.selectedColor.value;
		const secondaryColor = PixelEditorState.secondaryColor.value;

		const cells = paletteGrid.element.querySelectorAll(`.${style.colorCell}`);
		cells.forEach(cell => {
			const el = cell as HTMLElement;
			const idx = parseInt(el.dataset.index ?? '-1');

			el.classList.remove(style.selected, style.secondarySelected);

			if (idx === selectedColor) {
				el.classList.add(style.selected);
			}
			if (idx === secondaryColor) {
				el.classList.add(style.secondarySelected);
			}
		});

		updatePreviews();
	}

	// Effect: Render palette when loaded
	new Effect(function onPaletteLoad() {
		renderPalette();
		updatePreviews();
	}, { strong: true }).on([AppState.palette]);

	// Effect: Update selection highlights
	new Effect(function onSelectionChange() {
		updateSelection();
	}, { strong: true }).on([
		PixelEditorState.selectedColor,
		PixelEditorState.secondaryColor,
	]);

	return (
		Div().class(style.colorBar).nodes([
			colorPreviewContainer,
			paletteGrid,
		])
	);
}
