function paletteAnimationControl(animState, animCurrentState) {
	animState.state = animCurrentState;
	if (animState.state === 'playing') {
		animState.eventHandler = () => {
			animState.frame = (animState.frame - 1 + animState.frames) % animState.frames;
			animState.draw();
		};
		window.addEventListener('animTick', animState.eventHandler);
	} else {
		window.removeEventListener('animTick', animState.eventHandler);
	}
	animState.draw();
}


function createAnimState(id, palette, altPalette, name) {
	return {
		id,
		name: name,
		state: 'playing',
		frame: 0,
		frames: 7,
		fps: 6,
		animInterval: null,
		palette,
		altPalette,
		legend: document.getElementById(`subpalette-${id}-legend`),
		ctx: document.getElementById(`subpalette-${id}-canvas`).getContext('2d'),
		isAltPalette: false,
		isOff: false,
		unEvenOdd: false,
		mask: 'noMask',

		draw() {
			if (this.isOff) {
				this.ctx.fillStyle = '#000000';
				this.ctx.fillRect(0, 0, this.ctx.canvas.width, this.ctx.canvas.height);
				this.updateLegend();
				return;
			}
			const palette = this.isAltPalette ? this.altPalette : this.palette;
			this.ctx.clearRect(0, 0, this.ctx.canvas.width, this.ctx.canvas.height);
			const rectWidth = Math.floor(this.ctx.canvas.width / this.frames);
			for (let i = 0; i < this.frames; i++) {
				const colorIndex = (i + this.frame) % this.frames;
				const color = palette.getColor(colorIndex);
				this.ctx.fillStyle = `rgb(${color.r}, ${color.g}, ${color.b})`;
				this.ctx.fillRect(i * rectWidth, 0, rectWidth, this.ctx.canvas.height);
			}
			this.updateLegend();
		},
		updateLegend() {
			this.legend.innerHTML = `Palette ${this.name} - Frame ${this.frames - this.frame}/${this.frames}`;
		},
		animPlay() {
			paletteAnimationControl(this, 'playing');
		},
		animPause() {
			this.state = 'paused';
			paletteAnimationControl(this, 'paused');
		},
		animPlayPause() {
			if (this.state === 'playing') {
				this.animPause();
			} else {
				this.animPlay();
			}
		},
		animStop() {
			this.frame = 0;
			paletteAnimationControl(this, 'stopped');
		},
		animFrameBack() {
			this.frame++;
			if (this.frame >= this.frames) {
				this.frame = 0;
			}
			paletteAnimationControl(this, 'paused');
		},
		animFrameForward() {
			this.frame--;
			if (this.frame < 0) {
				this.frame = this.frames - 1;
			}
			paletteAnimationControl(this, 'paused');
		},
		switchPalette() {
			if (this.isOff) return;
			this.isAltPalette = !this.isAltPalette;
			window.dispatchEvent(new CustomEvent('changePalette', {
				detail: {
                    id: this.id,
					isAltPalette: this.isAltPalette,
					isOff: this.isOff,
                },
			}));
			this.draw();
		},
		toggleOnOff() {
			this.isOff = !this.isOff;
			window.dispatchEvent(new CustomEvent('changePalette', {
				detail: {
                    id: this.id,
					isAltPalette: this.isAltPalette,
					isOff: this.isOff,
                },
			}));
			this.draw();
		},
		toggleUnEvenOdd() {
			this.unEvenOdd = !this.unEvenOdd;
			window.dispatchEvent(new CustomEvent('unEvenOddToggle', {
				detail: {
                    id: this.id,
					unEvenOdd: this.unEvenOdd,
				},
			}));
			this.draw();
		},
		setMask(maskType) {
			if (maskType === 'even') {
				this.mask = 'maskEven';
			} else if (maskType === 'odd') {
				this.mask = 'maskOdd';
			} else if (maskType === 'noMask') {
				this.mask = 'noMask';
			}
			window.dispatchEvent(new CustomEvent('changeMask', {
				detail: {
					id: this.id,
					mask: this.mask,
				},
			}));
			this.draw();
		},
	};
}