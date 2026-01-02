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
	const paletteView = palette instanceof Uint8Array ? palette : new Uint8Array(palette);

	const canvas = document.createElement('canvas');
	canvas.width = 64;
	canvas.height = 64;
	const ctx = canvas.getContext('2d');
	const imageData = ctx.createImageData(64, 64);

	for (let i = 0; i < tileData.length; i++) {
		const colorIndex = tileData[i];
		const paletteOffset = colorIndex * 3;
		imageData.data[i * 4] = paletteView[paletteOffset];
		imageData.data[i * 4 + 1] = paletteView[paletteOffset + 1];
		imageData.data[i * 4 + 2] = paletteView[paletteOffset + 2];
		imageData.data[i * 4 + 3] = 255;
	}
	ctx.putImageData(imageData, 0, 0);
	return canvas;
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


const baselinePalette = data.snow.paletteData.slice(0, 256 * 3);

['crater', 'desert', 'green', 'snow', 'snow_dark'].forEach(biomeName => {
	const snowTileCanvases = data[biomeName].tileIndex
		.filter(tileName => /.[LS].001/.test(tileName))
		.map(tileName => {
			const canvas = data[biomeName].getTileAsCanvas(tileName);
			canvas.dataset.tileName = tileName;
			canvas.dataset.biomeName = biomeName;
			canvas.addEventListener('click', () => {
				showTileInfo(tileName, biomeName);
			});
			return canvas;
		});


	document.body.append(...snowTileCanvases);
	document.body.append(document.createElement('hr'));
});



function renderColorBox(colorIndices, paletteData) {
	const container = document.createElement('div');
	container.className = 'color-box-container';

	for (let y = 0; y < 16; y++) {
		for (let x = 0; x < 16; x++) {
			const i = y * 16 + x;
			const colorBox = document.createElement('div');
			const paletteOffset = i * 3;
			const r = paletteData[paletteOffset];
			const g = paletteData[paletteOffset + 1];
			const b = paletteData[paletteOffset + 2];
			colorBox.className = 'color-box';
			colorBox.style.borderColor = '#00F';
			if (colorIndices.includes(i)) {
				colorBox.style.backgroundColor = `rgb(${r}, ${g}, ${b})`;
			}
			container.appendChild(colorBox);
		}
	}

	return container;
}

function showTileInfo(tileName, biomeName) {
	const biome = data[biomeName];
	const tileData = biome.getTileDataByName(tileName);
	const paletteIndicesInUse = [...new Set(tileData)];
	const colorsInUse = Array.from(paletteIndicesInUse).map(index => {
		const colorBuffer = biome.getColorByIndex(index);
		return `#${colorBuffer[0].toString(16).padStart(2, '0')}${colorBuffer[1].toString(16).padStart(2, '0')}${colorBuffer[2].toString(16).padStart(2, '0')}`;
	});
	document.getElementById('tile-info').innerText = `Tile: ${tileName}, colors in use (${colorsInUse.length})`;

	document.getElementById('palette-in-use').replaceChildren(renderColorBox(paletteIndicesInUse, biome.paletteData));
}


console.log('uni-Tiles data loaded:', window._ = data);
