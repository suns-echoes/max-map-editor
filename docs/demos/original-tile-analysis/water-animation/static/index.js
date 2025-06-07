document.addEventListener('contextmenu', event => event.preventDefault());


state.ready.promise.then(() => {
	const tileCanvasArray1 = [];
	const tileCanvasArray2 = [];

	let useBitmapSet1 = bitmaps[1].evenOdd.normalPalette.noMask;
	let useBitmapSet2 = bitmaps[2].evenOdd.normalPalette.noMask;
	let anim1Frame = 0;
	let anim2Frame = 0;
	let isUnEvenOdd1 = false;
	let isUnEvenOdd2 = false;
	let isAltPalette1 = false;
	let isAltPalette2 = false;
	let isOff1 = false;
	let isOff2 = false;
	let mask1 = 'noMask';
	let mask2 = 'noMask';

	const tilesPerLine = 4;
	let tileWidth = 64 * 4;
	let tileHeight = 64 * 4;

	function createTileCanvases(useBitmapSet, tileCanvasArray) {
		let line = 0;
		let column = 0;
		for (const bitmap of useBitmapSet.map(row => row[0])) {
			const canvas = document.createElement('canvas');
			canvas.className = 'tile-canvas';
			canvas.style.top = `${line * tileWidth + 180}px`;
			canvas.style.left = `${column * tileHeight}px`;
			document.body.appendChild(canvas);
			canvas.width = bitmap.width;
			canvas.height = bitmap.height;
			const ctx = canvas.getContext('2d');
			ctx.clearRect(0, 0, canvas.width, canvas.height);
			ctx.drawImage(bitmap, 0, 0);
			tileCanvasArray.push(canvas);
			column++;
			if (column >= tilesPerLine) {
				column = 0;
				line++;
			}
		}
	}

	createTileCanvases(useBitmapSet1, tileCanvasArray1);
	createTileCanvases(useBitmapSet2, tileCanvasArray2);

	function repaintTiles() {
		tileCanvasArray1.forEach((canvas, index) => {
			const ctx = canvas.getContext('2d');
			ctx.clearRect(0, 0, canvas.width, canvas.height);
			ctx.drawImage(useBitmapSet1[index][anim1Frame], 0, 0);
		});
		tileCanvasArray2.forEach((canvas, index) => {
			const ctx = canvas.getContext('2d');
			ctx.clearRect(0, 0, canvas.width, canvas.height);
			ctx.drawImage(useBitmapSet2[index][anim2Frame], 0, 0);
		});
	}

	window.addEventListener('animTick', (event) => {
		const { anim1Frame: a1f, anim2Frame: a2f } = event.detail;
		anim1Frame = a1f;
		anim2Frame = a2f;
		repaintTiles();
	});

	function hideOffTiles() {
		tileCanvasArray1.forEach((canvas) => {
			canvas.style.display = isOff1 ? 'none' : 'block';
		});
		tileCanvasArray2.forEach((canvas) => {
			canvas.style.display = isOff2 ? 'none' : 'block';
		});
	}

	function switchBitmapSet() {
		const palette1 = isAltPalette1 ? 'altPalette' : 'normalPalette';
		const evenOdd1 = isUnEvenOdd1 ? 'unEvenOdd' : 'evenOdd';
		useBitmapSet1 = bitmaps[1][evenOdd1][palette1][mask1];
		const palette2 = isAltPalette2 ? 'altPalette' : 'normalPalette';
		const evenOdd2 = isUnEvenOdd2 ? 'unEvenOdd' : 'evenOdd';
		useBitmapSet2 = bitmaps[2][evenOdd2][palette2][mask2];
		hideOffTiles();
		repaintTiles();
	}

	window.addEventListener('unEvenOddToggle', (event) => {
		const { id, unEvenOdd } = event.detail;
		if (id === '1') {
			isUnEvenOdd1 = unEvenOdd;
		} else if (id === '2') {
			isUnEvenOdd2 = unEvenOdd;
		}
		switchBitmapSet();
		repaintTiles();
	});

	window.addEventListener('changePalette', (event) => {
		const { id, isAltPalette, isOff } = event.detail;
		if (id === '1') {
			isAltPalette1 = isAltPalette;
			isOff1 = isOff;
		} else if (id === '2') {
			isAltPalette2 = isAltPalette;
			isOff2 = isOff;
		}
		switchBitmapSet();
		repaintTiles();
	});

	window.addEventListener('changeMask', (event) => {
		const { id, mask } = event.detail;
		if (id === '1') {
			mask1 = mask;
		} else if (id === '2') {
			mask2 = mask;
		}
		switchBitmapSet();
		repaintTiles();
	});

	makeDraggable(document.querySelectorAll('.tile-canvas'));
});
