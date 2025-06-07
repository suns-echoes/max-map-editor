function setupPaletteControls({
	subpaletteId,
	animState,
}) {
	const canvas = document.getElementById(`subpalette-${subpaletteId}-canvas`);
	canvas.width = 42 * 7;
	canvas.height = 42;
	const prefix = `subpalette-${subpaletteId}-ctrl-`;
	const stopButton = document.getElementById(`${prefix}stop`);
	const playpauseButton = document.getElementById(`${prefix}playpause`);
	const framebackButton = document.getElementById(`${prefix}frameback`);
	const frameForwardButton = document.getElementById(`${prefix}frameForward`);
	const switchpaletteButton = document.getElementById(`${prefix}switchpalette`);
	const onoffButton = document.getElementById(`${prefix}onoff`);
	const unEvenOdd = document.getElementById(`${prefix}unEvenOdd`);
	const maskEven = document.getElementById(`${prefix}maskEven`);
	const maskOdd = document.getElementById(`${prefix}maskOdd`);
	const noMask = document.getElementById(`${prefix}noMask`);

	stopButton.addEventListener('click', () => {
		animState.animStop();
	});

	playpauseButton.addEventListener('click', () => {
		animState.animPlayPause();
	});

	framebackButton.addEventListener('click', () => {
		animState.animFrameBack();
	});

	frameForwardButton.addEventListener('click', () => {
		animState.animFrameForward();
	});

	switchpaletteButton.addEventListener('click', () => {
		animState.switchPalette();
	});

	onoffButton.addEventListener('click', () => {
		animState.toggleOnOff();
	});

	unEvenOdd.addEventListener('click', () => {
		animState.toggleUnEvenOdd();
	});

	maskEven.addEventListener('click', () => {
		animState.setMask('even');
	});

	maskOdd.addEventListener('click', () => {
		animState.setMask('odd');
	});

	noMask.addEventListener('click', () => {
		animState.setMask('noMask');
	});
}


setupPaletteControls({
	subpaletteId: '1',
	animState: state.anim1,
});

setupPaletteControls({
	subpaletteId: '2',
	animState: state.anim2,
});
