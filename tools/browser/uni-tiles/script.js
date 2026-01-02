console.log('uni-Tiles script loaded.');

function getTileDataByName(tileName) {
	const TILE_SIZE = 64 ** 2;
	const tileIdx = this.tileIndex.findIndex(t => t === tileName);
	if (tileIdx === -1) {
		throw new Error(`Tile "${tileName}" not found in this biome.`);
	}
	const tileDataOffset = tileIdx * TILE_SIZE;
	return new Uint8Array(this.tilesData, tileDataOffset, TILE_SIZE);
}

function getTileAsCanvas(tileName, palette = this.paletteData) {
	const tileData = this.getTileDataByName(tileName);
	const canvas = document.createElement('canvas');
	canvas.width = 64;
	canvas.height = 64;
	return drawTileOnCanvas(tileData, palette, canvas);
}

function getColorByIndex(index) {
	const buffer = new Uint8Array(4);
	const paletteOffset = index * 3;
	buffer[0] = this.paletteData[paletteOffset];
	buffer[1] = this.paletteData[paletteOffset + 1];
	buffer[2] = this.paletteData[paletteOffset + 2];
	buffer[3] = 255;
	return buffer;
}

function showThis() {
	console.log('uni-Tiles is working!', this);
}

function paletteToArrayBuffer(palette) {
	const buffer = new ArrayBuffer(256 * 3);
	const uint8View = new Uint8Array(buffer);
	for (let i = 0; i < palette.length; i++) {
		const r = parseInt(palette[i].substring(1, 3), 16);
		const g = parseInt(palette[i].substring(3, 5), 16);
		const b = parseInt(palette[i].substring(5, 7), 16);
		uint8View[i * 3] = r;
		uint8View[i * 3 + 1] = g;
		uint8View[i * 3 + 2] = b;
	}
	return uint8View;
}

function processPaletteText(palette) {
	return {
		cssColors: palette,
		paletteData: paletteToArrayBuffer(palette),
	};
}
function rgbToHex(r, g, b) {
	return `#${r.toString(16).padStart(2, '0')}${g.toString(16).padStart(2, '0')}${b.toString(16).padStart(2, '0')}`;
}
function hexToRgb(hex) {
	const value = hex.replace('#', '');
	return {
		r: parseInt(value.slice(0, 2), 16),
		g: parseInt(value.slice(2, 4), 16),
		b: parseInt(value.slice(4, 6), 16),
	};
}
function setDragPayload(event, color, meta = {}) {
	const payload = JSON.stringify({ r: color.r, g: color.g, b: color.b, ...meta });
	event.dataTransfer?.setData('application/json', payload);
	event.dataTransfer?.setData('text/plain', payload);
	if (event.dataTransfer) {
		event.dataTransfer.effectAllowed = meta.effectAllowed ?? 'copy';
	}
}
function parseDragPayload(event) {
	const raw = event.dataTransfer?.getData('application/json') || event.dataTransfer?.getData('text/plain');
	if (!raw) return null;
	try {
		return JSON.parse(raw);
	} catch {
		return null;
	}
}
function drawTileOnCanvas(tileData, palette, canvas) {
	const paletteView = palette instanceof Uint8Array ? palette : new Uint8Array(palette);
	const ctx = canvas.getContext('2d');
	const imageData = ctx.createImageData(64, 64);
	for (let i = 0; i < tileData.length; i++) {
		const offset = tileData[i] * 3;
		imageData.data[i * 4] = paletteView[offset];
		imageData.data[i * 4 + 1] = paletteView[offset + 1];
		imageData.data[i * 4 + 2] = paletteView[offset + 2];
		imageData.data[i * 4 + 3] = 255;
	}
	ctx.putImageData(imageData, 0, 0);
	return canvas;
}

const data = {
	crater: {
		tilesData: await fetch('/~res/assets/CRATER/tiles-data.bin').then(res => res.arrayBuffer()),
		tileIndex: await fetch('/~res/assets/CRATER/tiles-data.json').then(res => res.json()),
		...processPaletteText(await fetch('/~res/assets/CRATER/palette.json').then(res => res.json())),
	},
	desert: {
		tilesData: await fetch('/~res/assets/DESERT/tiles-data.bin').then(res => res.arrayBuffer()),
		tileIndex: await fetch('/~res/assets/DESERT/tiles-data.json').then(res => res.json()),
		...processPaletteText(await fetch('/~res/assets/DESERT/palette.json').then(res => res.json())),
	},
	green: {
		tilesData: await fetch('/~res/assets/GREEN/tiles-data.bin').then(res => res.arrayBuffer()),
		tileIndex: await fetch('/~res/assets/GREEN/tiles-data.json').then(res => res.json()),
		...processPaletteText(await fetch('/~res/assets/GREEN/palette.json').then(res => res.json())),

	},
	snow: {
		tilesData: await fetch('/~res/assets/SNOW/tiles-data.bin').then(res => res.arrayBuffer()),
		tileIndex: await fetch('/~res/assets/SNOW/tiles-data.json').then(res => res.json()),
		...processPaletteText(await fetch('/~res/assets/SNOW/palette.json').then(res => res.json())),

	},
	snow_dark: {
		tilesData: await fetch('/~res/assets/SNOW_DARK/tiles-data.bin').then(res => res.arrayBuffer()),
		tileIndex: await fetch('/~res/assets/SNOW_DARK/tiles-data.json').then(res => res.json()),
		...processPaletteText(await fetch('/~res/assets/SNOW_DARK/palette.json').then(res => res.json())),

	},
	water: {
		tilesData: await fetch('/~res/assets/WATER/tiles-data.bin').then(res => res.arrayBuffer()),
		tileIndex: await fetch('/~res/assets/WATER/tiles-data.json').then(res => res.json()),

	},
};

const biomePrototype = {
	getColorByIndex,
	getTileDataByName,
	getTileAsCanvas,
	showThis,
};

Object.setPrototypeOf(data.crater, biomePrototype);
Object.setPrototypeOf(data.desert, biomePrototype);
Object.setPrototypeOf(data.green, biomePrototype);
Object.setPrototypeOf(data.snow, biomePrototype);
Object.setPrototypeOf(data.snow_dark, biomePrototype);
Object.setPrototypeOf(data.water, biomePrototype);

const paletteEnabledBiomes = Object.entries(data).filter(([, biome]) => biome.paletteData);
const customPaletteState = new Array(256).fill(null);
let selectedBiome = paletteEnabledBiomes[0]?.[0] ?? 'snow';
let activePaletteHighlight = null;
const tileCanvasRegistry = [];

const customPaletteSection = document.createElement('section');
customPaletteSection.id = 'custom-palette-section';
const customPaletteTitle = document.createElement('h2');
customPaletteTitle.textContent = 'Custom Palette';
const customPaletteHint = document.createElement('p');
customPaletteHint.textContent = 'Drag colors from the biome palette into these slots.';
const customPaletteGrid = document.createElement('div');
customPaletteGrid.className = 'color-box-container custom-palette';
customPaletteSection.append(customPaletteTitle, customPaletteHint, customPaletteGrid);

const biomePaletteSection = document.createElement('section');
biomePaletteSection.id = 'biome-palette-section';
const biomePaletteHeader = document.createElement('div');
const biomePaletteLabel = document.createElement('label');
biomePaletteLabel.htmlFor = 'biome-palette-select';
biomePaletteLabel.textContent = 'Biome palette: ';
const biomeSelect = document.createElement('select');
biomeSelect.id = 'biome-palette-select';
const biomePaletteGrid = document.createElement('div');
biomePaletteGrid.className = 'color-box-container biome-palette';

const tileRenderControls = document.createElement('div');
tileRenderControls.id = 'tile-render-controls';
const renderWithCustomButton = document.createElement('button');
renderWithCustomButton.textContent = 'Render tiles with custom palette';
renderWithCustomButton.addEventListener('click', () => {
	const customPalette = buildCustomPaletteData();
	renderTilesWithPalette(() => customPalette);
});
const renderWithSelectedButton = document.createElement('button');
renderWithSelectedButton.textContent = 'Render tiles with selected biome palette';
renderWithSelectedButton.addEventListener('click', () => {
	renderTilesWithPalette(() => data[selectedBiome]?.paletteData);
});
const resetTilesButton = document.createElement('button');
resetTilesButton.textContent = 'Reset tile rendering';
resetTilesButton.addEventListener('click', () => {
	renderTilesWithPalette(entry => data[entry.biomeName].paletteData);
});
tileRenderControls.append(renderWithCustomButton, renderWithSelectedButton, resetTilesButton);

paletteEnabledBiomes.forEach(([biomeName]) => {
	const option = document.createElement('option');
	option.value = biomeName;
	option.textContent = biomeName;
	if (biomeName === selectedBiome) {
		option.selected = true;
	}
	biomeSelect.appendChild(option);
});

biomeSelect.addEventListener('change', event => {
	selectedBiome = event.target.value;
	activePaletteHighlight = null;
	renderCurrentBiomePalette();
});

biomePaletteHeader.append(biomePaletteLabel, biomeSelect);
biomePaletteSection.append(biomePaletteHeader, biomePaletteGrid);

let shouldCopyColor = false;
window.addEventListener('keydown', event => {
	if (event.key === 'Control') {
		shouldCopyColor = true;
	}
});
window.addEventListener('keyup', event => {
	if (event.key === 'Control') {
		shouldCopyColor = false;
	}
});

function renderCustomPaletteGrid() {
	customPaletteGrid.replaceChildren();
	for (let i = 0; i < 256; i++) {
		const slot = document.createElement('div');
		slot.className = 'color-box custom-palette-slot';
		slot.dataset.index = i;
		const color = customPaletteState[i];
		if (color) {
			const hex = rgbToHex(color.r, color.g, color.b);
			slot.style.backgroundColor = hex;
			// slot.style.borderColor = '#0a0';
			slot.title = `Slot ${i}: ${hex}`;
			slot.draggable = true;
			slot.addEventListener('dragstart', event => {
				setDragPayload(event, color, { sourceType: 'customSlot', sourceIndex: i, effectAllowed: 'copyMove' });
			});
		} else {
			slot.classList.add('empty');
			// slot.style.borderColor = '#999';
			slot.title = `Slot ${i}: empty`;
			slot.draggable = false;
		}
		slot.addEventListener('dragover', event => {
			event.preventDefault();
			slot.classList.add('drop-ready');
		});
		slot.addEventListener('dragleave', () => slot.classList.remove('drop-ready'));
		slot.addEventListener('drop', event => {
			event.preventDefault();
			slot.classList.remove('drop-ready');
			const payload = parseDragPayload(event);
			if (!payload) return;
			if (payload.sourceType === 'customSlot' && typeof payload.sourceIndex === 'number') {
				const from = payload.sourceIndex;
				if (from === i) return;
				const sourceColor = customPaletteState[from] ?? { r: payload.r, g: payload.g, b: payload.b };
				if (shouldCopyColor) {
					customPaletteState[i] = { r: sourceColor.r, g: sourceColor.g, b: sourceColor.b };
				} else {
					[customPaletteState[i], customPaletteState[from]] = [customPaletteState[from], customPaletteState[i]];
				}
				renderCustomPaletteGrid();
				return;
			}
			if (typeof payload.r === 'number') {
				customPaletteState[i] = { r: payload.r, g: payload.g, b: payload.b };
				renderCustomPaletteGrid();
			}
		});
		slot.addEventListener('dblclick', () => {
			if (!color) return;
			const picker = document.createElement('input');
			picker.type = 'color';
			picker.value = rgbToHex(color.r, color.g, color.b);
			picker.style.position = 'fixed';
			picker.style.left = '-9999px';
			picker.addEventListener('input', e => {
				customPaletteState[i] = hexToRgb(e.target.value);
				renderCustomPaletteGrid();
			}, { once: true });
			picker.addEventListener('blur', () => picker.remove(), { once: true });
			document.body.appendChild(picker);
			picker.click();
		});
		customPaletteGrid.appendChild(slot);
	}
}

function renderCurrentBiomePalette() {
	const biome = data[selectedBiome];
	if (!biome?.paletteData) return;
	const { paletteData } = biome;
	const highlightActive = activePaletteHighlight && activePaletteHighlight.biomeName === selectedBiome;
	biomePaletteGrid.replaceChildren();
	for (let i = 0; i < 256; i++) {
		const paletteOffset = i * 3;
		const r = paletteData[paletteOffset];
		const g = paletteData[paletteOffset + 1];
		const b = paletteData[paletteOffset + 2];
		const colorBox = document.createElement('div');
		colorBox.className = 'color-box biome-color';
		const hex = rgbToHex(r, g, b);
		colorBox.style.backgroundColor = hex;
		colorBox.title = `${selectedBiome}[${i}] ${hex}`;
		colorBox.draggable = true;
		if (highlightActive) {
			colorBox.style.opacity = activePaletteHighlight.indices.has(i) ? '1' : '0.25';
		}
		colorBox.addEventListener('dragstart', event => {
			setDragPayload(event, { r, g, b });
		});
		colorBox.addEventListener('click', event => {
			if (!event.shiftKey) return;
			event.preventDefault();
			customPaletteState[i] = { r, g, b };
			renderCustomPaletteGrid();
		});
		biomePaletteGrid.appendChild(colorBox);
	}
}

function initializePaletteBuilderUI() {
	if (!paletteEnabledBiomes.length) return;
	document.body.prepend(biomePaletteSection);
	document.body.prepend(customPaletteSection);
	renderCustomPaletteGrid();
	renderCurrentBiomePalette();
}

initializePaletteBuilderUI();
if (paletteEnabledBiomes.length) {
	customPaletteSection.after(tileRenderControls);
} else {
	document.body.prepend(tileRenderControls);
}

const baselinePalette = data.snow.paletteData.slice(0, 256 * 3);

['crater', 'desert', 'green', 'snow', 'snow_dark'].forEach(biomeName => {
	const snowTileCanvases = data[biomeName].tileIndex
		// .filter(tileName => /.[LS].001/.test(tileName))
		.filter(tileName => /.[LS]..../.test(tileName))
		.map(tileName => {
			const canvas = data[biomeName].getTileAsCanvas(tileName);
			canvas.dataset.tileName = tileName;
			canvas.dataset.biomeName = biomeName;
			canvas.addEventListener('click', () => {
				showTileInfo(tileName, biomeName);
			});
			tileCanvasRegistry.push({ canvas, biomeName, tileName });
			return canvas;
		});


	document.body.append(...snowTileCanvases);
	document.body.append(document.createElement('hr'));
});



function showTileInfo(tileName, biomeName) {
	const biome = data[biomeName];
	const tileData = biome.getTileDataByName(tileName);
	const paletteIndexSet = new Set(tileData);
	const paletteIndicesInUse = [...paletteIndexSet];
	const colorsInUse = paletteIndicesInUse.map(index => {
		const colorBuffer = biome.getColorByIndex(index);
		return `#${colorBuffer[0].toString(16).padStart(2, '0')}${colorBuffer[1].toString(16).padStart(2, '0')}${colorBuffer[2].toString(16).padStart(2, '0')}`;
	});
	const togglingSameTile = activePaletteHighlight
		&& activePaletteHighlight.tileName === tileName
		&& activePaletteHighlight.biomeName === biomeName;
	activePaletteHighlight = togglingSameTile ? null : {
		biomeName,
		tileName,
		indices: paletteIndexSet,
	};
	selectedBiome = biomeName;
	biomeSelect.value = biomeName;
	renderCurrentBiomePalette();
	document.getElementById('tile-info').innerText = `Tile: ${tileName}, colors in use (${colorsInUse.length})`;
	const paletteInUseEl = document.getElementById('palette-in-use');
	if (paletteInUseEl) {
		paletteInUseEl.textContent = 'Colors highlighted in biome palette above.';
	}
}


console.log('uni-Tiles data loaded:', window._ = data);

function buildCustomPaletteData() {
	const palette = new Uint8Array(256 * 3);
	customPaletteState.forEach((color, index) => {
		const offset = index * 3;
		if (!color) {
			palette[offset] = palette[offset + 1] = palette[offset + 2] = 0;
		} else {
			palette[offset] = color.r ?? 0;
			palette[offset + 1] = color.g ?? 0;
			palette[offset + 2] = color.b ?? 0;
		}
	});
	return palette;
}
function renderTilesWithPalette(resolver) {
	tileCanvasRegistry.forEach(entry => {
		const palette = typeof resolver === 'function' ? resolver(entry) : resolver;
		if (!palette) return;
		const tileData = data[entry.biomeName].getTileDataByName(entry.tileName);
		drawTileOnCanvas(tileData, palette, entry.canvas);
	});
}
