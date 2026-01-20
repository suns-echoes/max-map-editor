/**
 * Pixel Editor Toolbar Component
 *
 * Tool selection and options for pixel editing.
 */

import { Div, Button, Input } from '^reactive/reactive-node.elements.ts';
import { Effect } from '^reactive/effect.ts';
import { xlog } from '^lib/xlog/xlog.ts';
import { PixelEditorState } from '../pixel-editor.state.ts';
import { type PixelTool, type ReplaceScope } from '../pixel-editor.types.ts';
import { replaceColor } from '../pixel-editor.actions.ts';

import style from './pixel-editor-toolbar.module.css';


/**
 * Pixel Editor Toolbar - tool selection and options.
 */
export function PixelEditorToolbar() {
	xlog.info('UI::PixelEditorToolbar');

	// Tool buttons
	const tools: Array<{ id: PixelTool; label: string; title: string }> = [
		{ id: 'picker', label: '💧', title: 'Color Picker (pick color from tile)' },
		{ id: 'pencil', label: '✏️', title: 'Pencil (draw pixels)' },
		{ id: 'replace', label: '🔄', title: 'Replace Color' },
	];

	const toolButtonsContainer = Div().class(style.toolButtons);
	const toolButtons = new Map<PixelTool, ReturnType<typeof Button>>();

	for (const { id, label, title } of tools) {
		const btn = Button(label)
			.class(style.toolButton)
			.attr('title', title);

		btn.on('click', () => PixelEditorState.setTool(id));
		toolButtons.set(id, btn);
		toolButtonsContainer.element.appendChild(btn.element);
	}

	// Brush size control
	const brushSizeInput = Input('range')
		.class(style.brushSizeInput)
		.attr('min', '1')
		.attr('max', '8')
		.attr('title', 'Brush Size');
	brushSizeInput.element.value = '1';
	brushSizeInput.on('input', () => {
		PixelEditorState.setBrushSize(parseInt(brushSizeInput.element.value));
	});

	const brushSizeLabel = Div().class(style.brushSizeLabel).text('Size: 1');

	const brushSizeContainer = Div().class(style.brushSizeContainer).nodes([
		brushSizeLabel,
		brushSizeInput,
	]);

	// Replace scope selector
	const scopes: Array<{ id: ReplaceScope; label: string }> = [
		{ id: 'tile', label: 'This Tile' },
		{ id: 'selected', label: 'Selected' },
		{ id: 'all', label: 'All Tiles' },
	];

	const scopeButtonsContainer = Div().class(style.scopeButtons);
	const scopeButtons = new Map<ReplaceScope, ReturnType<typeof Button>>();

	for (const { id, label } of scopes) {
		const btn = Button(label).class(style.scopeButton);
		btn.on('click', () => PixelEditorState.setReplaceScope(id));
		scopeButtons.set(id, btn);
		scopeButtonsContainer.element.appendChild(btn.element);
	}

	// Replace action button
	const replaceButton = Button('Replace').class(style.replaceButton);
	replaceButton.on('click', () => {
		const count = replaceColor();
		xlog.info(`Replaced ${count} pixels`);
	});

	const replaceContainer = Div().class(style.replaceContainer).nodes([
		Div().class(style.sectionLabel).text('Replace Scope:'),
		scopeButtonsContainer,
		replaceButton,
	]);

	// Zoom controls
	const zoomOutBtn = Button('-').class(style.zoomButton);
	const zoomInBtn = Button('+').class(style.zoomButton);
	const zoomLabel = Div().class(style.zoomLabel).text('4x');

	zoomOutBtn.on('click', () => PixelEditorState.setZoom(PixelEditorState.zoom.value - 1));
	zoomInBtn.on('click', () => PixelEditorState.setZoom(PixelEditorState.zoom.value + 1));

	const zoomContainer = Div().class(style.zoomContainer).nodes([
		Div().class(style.sectionLabel).text('Zoom:'),
		zoomOutBtn,
		zoomLabel,
		zoomInBtn,
	]);

	// Grid toggle
	const gridToggle = Button('Grid').class(style.gridToggle);
	gridToggle.on('click', () => PixelEditorState.toggleGrid());

	// Effect: Update tool button states
	new Effect(function updateToolButtons() {
		const currentTool = PixelEditorState.tool.value;
		for (const [id, btn] of toolButtons) {
			if (id === currentTool) {
				btn.element.classList.add(style.active);
			} else {
				btn.element.classList.remove(style.active);
			}
		}

		// Show/hide replace options based on tool
		if (currentTool === 'replace') {
			replaceContainer.element.style.display = 'flex';
			brushSizeContainer.element.style.display = 'none';
		} else {
			replaceContainer.element.style.display = 'none';
			brushSizeContainer.element.style.display = currentTool === 'pencil' ? 'flex' : 'none';
		}
	}, { strong: true }).on([PixelEditorState.tool]);

	// Effect: Update scope button states
	new Effect(function updateScopeButtons() {
		const currentScope = PixelEditorState.replaceScope.value;
		for (const [id, btn] of scopeButtons) {
			if (id === currentScope) {
				btn.element.classList.add(style.active);
			} else {
				btn.element.classList.remove(style.active);
			}
		}
	}, { strong: true }).on([PixelEditorState.replaceScope]);

	// Effect: Update brush size label
	new Effect(function updateBrushSize() {
		const size = PixelEditorState.brushSize.value;
		brushSizeLabel.text(`Size: ${size}`);
		brushSizeInput.element.value = String(size);
	}, { strong: true }).on([PixelEditorState.brushSize]);

	// Effect: Update zoom label
	new Effect(function updateZoom() {
		const zoom = PixelEditorState.zoom.value;
		zoomLabel.text(`${zoom}x`);
	}, { strong: true }).on([PixelEditorState.zoom]);

	// Effect: Update grid toggle state
	new Effect(function updateGridToggle() {
		if (PixelEditorState.showGrid.value) {
			gridToggle.element.classList.add(style.active);
		} else {
			gridToggle.element.classList.remove(style.active);
		}
	}, { strong: true }).on([PixelEditorState.showGrid]);

	return (
		Div().class(style.pixelEditorToolbar).nodes([
			Div().class(style.section).nodes([
				Div().class(style.sectionLabel).text('Tools:'),
				toolButtonsContainer,
			]),
			brushSizeContainer,
			replaceContainer,
			zoomContainer,
			gridToggle,
		])
	);
}
