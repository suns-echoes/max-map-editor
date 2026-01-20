/**
 * Palette Panel Component
 *
 * UI panel for editing palette colors.
 */

import { AppState } from '^state/app-state.ts';
import { Effect } from '^reactive/effect.ts';
import { Section, Div, Button, Input } from '^reactive/reactive-node.elements.ts';
import { xlog } from '^lib/xlog/xlog.ts';
import { PaletteEditorState } from '../palette-editor.state.ts';
import {
	rgbToHex,
	hexToRgb,
	getPaletteColor,
} from '../palette-editor.types.ts';
import {
	getColorAt,
	setColorAt,
	getPaletteWithUsage,
	copySelectedColor,
	pasteColor,
	createGradient,
} from '../palette-editor.actions.ts';

import style from './palette-panel.module.css';


/**
 * Palette Panel component - displays and edits palette colors.
 */
export function PalettePanel() {
	xlog.info('UI::PalettePanel');

	// UI references
	const colorPreview = Div().class(style.colorPreview);
	const colorInfo = Div().class(style.colorInfo);
	const colorInput = Input('color').class(style.colorPicker);
	const paletteGrid = Div().class(style.paletteGrid);

	/**
	 * Update color preview and info for selected index.
	 */
	function updateColorPreview() {
		const index = PaletteEditorState.selectedIndex.value;

		if (index === null) {
			colorPreview.element.style.backgroundColor = 'transparent';
			colorInfo.text('No color selected');
			return;
		}

		const color = getColorAt(index);
		if (!color) {
			colorInfo.text(`Index ${index}: invalid`);
			return;
		}

		const hex = rgbToHex(color.r, color.g, color.b);
		colorPreview.element.style.backgroundColor = hex;
		colorInfo.nodes([
			Div().text(`Index: ${index}`),
			Div().text(`RGB: ${color.r}, ${color.g}, ${color.b}`),
			Div().text(`Hex: ${hex}`),
		]);
		colorInput.element.value = hex;
	}

	/**
	 * Render the 16x16 palette grid.
	 */
	function renderPaletteGrid() {
		const palette = AppState.palette.value;
		const selectedIndex = PaletteEditorState.selectedIndex.value;
		const secondaryIndex = PaletteEditorState.secondaryIndex.value;
		const showUsage = PaletteEditorState.showUsage.value;

		if (!palette) {
			paletteGrid.nodes([
				Div().class(style.noPalette).text('No palette loaded')
			]);
			return;
		}

		const usageInfo = showUsage ? getPaletteWithUsage() : null;
		const colorNodes: ReturnType<typeof Div>[] = [];

		for (let i = 0; i < 256; i++) {
			const color = getPaletteColor(palette, i);
			const hex = rgbToHex(color.r, color.g, color.b);

			const cell = Div().class(style.colorCell);
			cell.element.style.backgroundColor = hex;
			cell.element.dataset.index = String(i);
			cell.element.title = `${i}: ${hex}`;

			// Selection state
			if (i === selectedIndex) {
				cell.element.classList.add(style.selected);
			} else if (i === secondaryIndex) {
				cell.element.classList.add(style.secondary);
			}

			// Range highlight
			if (selectedIndex !== null && secondaryIndex !== null) {
				const minIdx = Math.min(selectedIndex, secondaryIndex);
				const maxIdx = Math.max(selectedIndex, secondaryIndex);
				if (i >= minIdx && i <= maxIdx) {
					cell.element.classList.add(style.inRange);
				}
			}

			// Usage overlay
			if (usageInfo) {
				const usage = usageInfo[i];
				if (usage.usageCount === 0) {
					cell.element.classList.add(style.unused);
				}
			}

			// Click handlers
			cell.on('click', (e: MouseEvent) => {
				if (e.shiftKey) {
					PaletteEditorState.extendSelection(i);
				} else {
					PaletteEditorState.select(i);
				}
			});

			// Double-click to edit
			cell.on('dblclick', () => {
				colorInput.element.click();
			});

			colorNodes.push(cell);
		}

		paletteGrid.nodes(colorNodes);
	}

	/**
	 * Create action buttons.
	 */
	function createActionButtons() {
		const copyBtn = Button('Copy').class(style.actionButton);
		copyBtn.on('click', () => copySelectedColor());

		const pasteBtn = Button('Paste').class(style.actionButton);
		pasteBtn.on('click', () => pasteColor());

		const gradientBtn = Button('Gradient').class(style.actionButton);
		gradientBtn.on('click', () => {
			const range = PaletteEditorState.selectedRange.value;
			if (range && range.start !== range.end) {
				createGradient(range.start, range.end);
			}
		});

		const usageBtn = Button('Usage').class(style.actionButton);
		usageBtn.on('click', () => PaletteEditorState.toggleUsage());

		return Div().class(style.actionButtons).nodes([
			copyBtn,
			pasteBtn,
			gradientBtn,
			usageBtn,
		]);
	}

	// Color picker input handler
	colorInput.on('input', () => {
		const index = PaletteEditorState.selectedIndex.value;
		if (index === null) return;

		const hex = colorInput.element.value;
		const color = hexToRgb(hex);
		setColorAt(index, color);
	});

	// Effect: Update when selection changes
	new Effect(function onSelectionChange() {
		updateColorPreview();
		// Update selection highlights
		const cells = paletteGrid.element.querySelectorAll(`.${style.colorCell}`);
		const selectedIndex = PaletteEditorState.selectedIndex.value;
		const secondaryIndex = PaletteEditorState.secondaryIndex.value;

		cells.forEach(cell => {
			const el = cell as HTMLElement;
			const idx = parseInt(el.dataset.index ?? '-1');

			el.classList.remove(style.selected, style.secondary, style.inRange);

			if (idx === selectedIndex) {
				el.classList.add(style.selected);
			} else if (idx === secondaryIndex) {
				el.classList.add(style.secondary);
			}

			if (selectedIndex !== null && secondaryIndex !== null) {
				const minIdx = Math.min(selectedIndex, secondaryIndex);
				const maxIdx = Math.max(selectedIndex, secondaryIndex);
				if (idx >= minIdx && idx <= maxIdx) {
					el.classList.add(style.inRange);
				}
			}
		});
	}, { strong: true }).on([
		PaletteEditorState.selectedIndex,
		PaletteEditorState.secondaryIndex,
	]);

	// Effect: Re-render when palette changes
	new Effect(function onPaletteChange() {
		renderPaletteGrid();
		updateColorPreview();
	}, { strong: true }).on([AppState.palette]);

	// Effect: Update usage overlay
	new Effect(function onUsageToggle() {
		renderPaletteGrid();
	}, { strong: true }).on([PaletteEditorState.showUsage]);

	return (
		Section('palette-panel').class(style.palettePanel).nodes([
			Div().class(style.header).text('Palette Editor'),
			Div().class(style.previewRow).nodes([
				colorPreview,
				colorInfo,
				colorInput,
			]),
			createActionButtons(),
			paletteGrid,
		])
	);
}
