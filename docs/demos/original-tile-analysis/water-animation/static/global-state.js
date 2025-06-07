var state = {
	anim1: createAnimState('1', water1Palette, water1AltPalette, 'Water 1'),
	anim2: createAnimState('2', water2Palette, water2AltPalette, 'Water 2'),
	ready: Promise.withResolvers(),
	animFrameTime: 1000 / 6, // 6 FPS
	frameTicker: null,
	zoom: 4,
};

state.frameTicker = setInterval(() => {
	window.dispatchEvent(new CustomEvent('animTick', {
		detail: {
			anim1Frame: state.anim1.frame,
			anim2Frame: state.anim2.frame,
		},
	}));
}, state.animFrameTime);

state.anim1.animPlay();
state.anim2.animPlay();
