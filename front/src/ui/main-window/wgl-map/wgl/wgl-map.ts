import vertexShaderSource from './shaders/map.vs?raw';
import fragmentShaderSource from './shaders/map.fs?raw';
import { Perf } from '^lib/perf/perf.ts';
import { WebGL2 } from '^lib/webgl2/webgl2.ts';
import { MAP_LAYERS } from '^consts/map-consts.ts';
import { printDebugInfo } from '^lib/debug/debug.ts';
import { FPS } from '^lib/webgl2/fps.ts';


export class WglMap extends WebGL2 {
	constructor(canvas: HTMLCanvasElement) {
		printDebugInfo('WglMap::constructor');
		super(canvas);

		this.tileCapability = this.getTileCapability();
		this.textures = new Array(this.tileCapability.maxTextureLayers).fill(null);

		const program = this.createProgram(vertexShaderSource, fragmentShaderSource);
		this.gl.useProgram(program);

		this.getUniformLocations(program);
		this.createQuadBuffer();
		this.updateScreenSize();

		this.clear();

		this.gl.enable(this.gl.BLEND);
		this.gl.blendFunc(this.gl.SRC_ALPHA, this.gl.ONE_MINUS_SRC_ALPHA);

		printDebugInfo('WglMap::constructor done');
	}

	/** Create a single full-screen quad in clip space */
	private createQuadBuffer(): void {
		// Full-screen quad: two triangles covering [-1,1] x [-1,1]
		const quadVertices = new Float32Array([
			-1, -1,  // Bottom-left
			 1, -1,  // Bottom-right
			-1,  1,  // Top-left
			-1,  1,  // Top-left
			 1, -1,  // Bottom-right
			 1,  1,  // Top-right
		]);
		this.buffers.quad = this.createBuffer(quadVertices, 0, 2);
	}

	/** Update screen size uniform when canvas resizes */
	private updateScreenSize(): void {
		const canvas = this.gl.canvas as HTMLCanvasElement;
		canvas.width = canvas.parentElement!.clientWidth;
		canvas.height = canvas.parentElement!.clientHeight;
		this.gl.viewport(0, 0, canvas.width, canvas.height);
		this.gl.uniform2f(this.uniformLocations.uScreenSize, canvas.width, canvas.height);
	}

	onCanvasResize() {
		this.updateScreenSize();
		this._limitMapZoom();
		this._limitMapPan();
		this._updateUniforms();
		this.render();
	}

	private _cursor: Vec2 = new Float32Array([0, 0]);

	moveCursor(screenX: number, screenY: number) {
		const canvas = this.gl.canvas as HTMLCanvasElement;
		// Convert screen position to world position
		const worldX = (screenX - canvas.width * 0.5) / this._zoom + this._panX + this.mapWidth * 32;
		const worldY = (screenY - canvas.height * 0.5) / this._zoom + this._panY + this.mapHeight * 32;

		// Convert to cell coordinates
		this._cursor[0] = Math.floor(worldX / 64);
		this._cursor[1] = Math.floor(worldY / 64);

		this.gl.uniform2fv(this.uniformLocations.uCursor, this._cursor);
		this.render();
	}

	// Camera state: pan in world pixels (map pixels), zoom factor
	private _panX: number = 0;
	private _panY: number = 0;
	private _zoom: number = 1;

	public moveCamera(dx: number, dy: number, dz: number, cursorX: number = 0, cursorY: number = 0) {
		const canvas = this.gl.canvas as HTMLCanvasElement;

		if (dz !== 0) {
			// Zoom towards cursor
			const oldZoom = this._zoom;
			this._zoom *= 1 + dz * 0.1;
			this._limitMapZoom();

			// Adjust pan to zoom towards cursor
			const cursorOffsetX = cursorX - canvas.width * 0.5;
			const cursorOffsetY = cursorY - canvas.height * 0.5;

			// Convert cursor offset to world coordinates at old zoom, then adjust
			this._panX += cursorOffsetX / oldZoom - cursorOffsetX / this._zoom;
			this._panY += cursorOffsetY / oldZoom - cursorOffsetY / this._zoom;
		}

		// Pan in world coordinates
		this._panX -= dx / this._zoom;
		this._panY -= dy / this._zoom;

		this._limitMapPan();
		this._updateUniforms();
		this.render();
	}

	private _limitMapZoom() {
		const canvas = this.gl.canvas as HTMLCanvasElement;
		const minZoom = Math.min(
			canvas.width / this.mapModelWidth,
			canvas.height / this.mapModelHeight
		);
		const maxZoom = 2;

		if (this._zoom < minZoom) this._zoom = minZoom;
		if (this._zoom > maxZoom) this._zoom = maxZoom;
	}

	private _limitMapPan() {
		const canvas = this.gl.canvas as HTMLCanvasElement;
		const margin = 128 / this._zoom;

		// Maximum pan is half the map size minus half the visible area plus margin
		const visibleW = canvas.width / this._zoom;
		const visibleH = canvas.height / this._zoom;

		const maxPanX = Math.max(0, (this.mapModelWidth - visibleW) * 0.5 + margin);
		const maxPanY = Math.max(0, (this.mapModelHeight - visibleH) * 0.5 + margin);

		if (this._panX < -maxPanX) this._panX = -maxPanX;
		if (this._panX > maxPanX) this._panX = maxPanX;
		if (this._panY < -maxPanY) this._panY = -maxPanY;
		if (this._panY > maxPanY) this._panY = maxPanY;
	}

	private _updateUniforms() {
		this.gl.uniform2f(this.uniformLocations.uPan, this._panX, this._panY);
		this.gl.uniform1f(this.uniformLocations.uZoom, this._zoom);
		this.gl.uniform2f(this.uniformLocations.uMapSize, this.mapWidth, this.mapHeight);
	}

	initPalette(paletteData: Uint8Array) {
		// Store working palette for color cycling
		this._workingPalette = new Uint8Array(paletteData);
		this.createTexture(this.PALETTE_TEXTURE, this._workingPalette, 256, 1, this.gl.RGBA);
		this.gl.uniform1i(this.uniformLocations.uPaletteTexture, this.PALETTE_TEXTURE);
	}

	// Base and working palette for color cycling
	private _workingPalette: Uint8Array | null = null;

	// Color cycling ranges: [start, end, direction, fps]
	// direction: 0 = backward, 1 = forward
	private readonly _colorCycleRanges = [
		{ start: 9, end: 12, direction: 0, fps: 9 },
		{ start: 13, end: 16, direction: 1, fps: 6 },
		{ start: 17, end: 20, direction: 1, fps: 9 },
		{ start: 21, end: 24, direction: 1, fps: 6 },
		{ start: 25, end: 30, direction: 1, fps: 2 },
		{ start: 31, end: 31, direction: 1, fps: 6 },
		{ start: 96, end: 102, direction: 1, fps: 8 },
		{ start: 103, end: 109, direction: 1, fps: 8 },
		{ start: 110, end: 116, direction: 1, fps: 10 },
		{ start: 117, end: 122, direction: 1, fps: 6 },
		{ start: 123, end: 127, direction: 1, fps: 6 },
	];
	private _lastCycleTime: number[] = this._colorCycleRanges.map(() => 0);

	/** Cycle colors in a palette range */
	private _cycleColors(start: number, end: number, direction: number): void {
		if (!this._workingPalette) return;
		const palette = this._workingPalette;
		const startIdx = start * 4;
		const endIdx = end * 4;

		if (direction === 1) {
			// Forward: shift colors up, wrap last to first
			const lastR = palette[endIdx];
			const lastG = palette[endIdx + 1];
			const lastB = palette[endIdx + 2];
			const lastA = palette[endIdx + 3];
			for (let i = endIdx; i > startIdx; i -= 4) {
				palette[i] = palette[i - 4];
				palette[i + 1] = palette[i - 3];
				palette[i + 2] = palette[i - 2];
				palette[i + 3] = palette[i - 1];
			}
			palette[startIdx] = lastR;
			palette[startIdx + 1] = lastG;
			palette[startIdx + 2] = lastB;
			palette[startIdx + 3] = lastA;
		} else {
			// Backward: shift colors down, wrap first to last
			const firstR = palette[startIdx];
			const firstG = palette[startIdx + 1];
			const firstB = palette[startIdx + 2];
			const firstA = palette[startIdx + 3];
			for (let i = startIdx; i < endIdx; i += 4) {
				palette[i] = palette[i + 4];
				palette[i + 1] = palette[i + 5];
				palette[i + 2] = palette[i + 6];
				palette[i + 3] = palette[i + 7];
			}
			palette[endIdx] = firstR;
			palette[endIdx + 1] = firstG;
			palette[endIdx + 2] = firstB;
			palette[endIdx + 3] = firstA;
		}
	}

	/** Update palette texture after color cycling */
	private _updatePaletteTexture(): void {
		if (!this._workingPalette) return;
		this.gl.activeTexture(this.gl.TEXTURE0 + this.PALETTE_TEXTURE);
		this.gl.texSubImage2D(this.gl.TEXTURE_2D, 0, 0, 0, 256, 1, this.gl.RGBA, this.gl.UNSIGNED_BYTE, this._workingPalette);
	}

	initMap(mapData: Uint16Array, width: number, height: number) {
		if (this.textures[this.MAP_TEXTURE]) {
			this.gl.deleteTexture(this.mapTexture);
		}
		this.mapWidth = width;
		this.mapHeight = height;
		this.mapModelWidth = width * 64;
		this.mapModelHeight = height * 64;

		this.textures[this.MAP_TEXTURE] = this.createTexture(
			this.MAP_TEXTURE, mapData, width, height, this.gl.RGBA16UI, '3d', MAP_LAYERS
		);
		this.gl.uniform1i(this.uniformLocations.uMapTexture, this.MAP_TEXTURE);
		this.gl.uniform2f(this.uniformLocations.uMapSize, width, height);

		// Reset camera
		this._panX = 0;
		this._panY = 0;
		this._zoom = 1;
		this._limitMapZoom();
		this._updateUniforms();
	}

	initTilesets(tilesetData: Uint8Array, layers: number) {
		const perf = Perf('WglMap::uploadTilesets');

		this.createTexture(
			this.TILES_TEXTURE,
			tilesetData,
			this.tileCapability.maxTextureSize,
			this.tileCapability.maxTextureSize,
			this.gl.R8UI,
			'3d',
			layers,
		);
		this.gl.uniform1i(this.uniformLocations.uTilesTexture, this.TILES_TEXTURE);
		this.gl.uniform1ui(this.uniformLocations.uTilesPerRow, this.tileCapability.tilesPerRow);

		perf();
	}

	render() {
		this.clear();
		this.buffers.quad.use();

		// Update time uniform (seconds since page load)
		this.gl.uniform1f(this.uniformLocations.uTime, performance.now() / 1000.0);

		// Render layer 0
		this.gl.uniform1i(this.uniformLocations.uMapLayer, 0);
		this.gl.drawArrays(this.gl.TRIANGLES, 0, 6);

		// Render layer 1
		this.gl.uniform1i(this.uniformLocations.uMapLayer, 1);
		this.gl.drawArrays(this.gl.TRIANGLES, 0, 6);
	}

	private _animationTimer: TimerID | null = null;

	enableAnimation() {
		if (this._animationTimer !== null) return;
		this._animationTimer = setInterval(() => this._animationFrame(), FPS(30));
	}

	private _animationFrame() {
		const now = performance.now();

		// Color cycling
		let paletteChanged = false;
		for (let i = 0; i < this._colorCycleRanges.length; i++) {
			const range = this._colorCycleRanges[i];
			const intervalMs = 1000 / range.fps;
			if (now - this._lastCycleTime[i] >= intervalMs) {
				this._cycleColors(range.start, range.end, range.direction);
				this._lastCycleTime[i] = now;
				paletteChanged = true;
			}
		}

		if (paletteChanged) {
			this._updatePaletteTexture();
		}

		this.render();
	}

	disableAnimation() {
		if (this._animationTimer === null) return;
		clearInterval(this._animationTimer);
		this._animationTimer = null;
	}

	// Texture unit assignments
	PALETTE_TEXTURE = 0;
	MAP_TEXTURE = 1;
	TILES_TEXTURE = 2;

	// Map dimensions
	mapWidth: number = 0;
	mapHeight: number = 0;
	mapModelWidth: number = 0;
	mapModelHeight: number = 0;
	mapTexture: WebGLTexture | null = null;

	uniformLocations: Record<string, WebGLUniformLocation> = {
		uScreenSize: null!,
		uPan: null!,
		uZoom: null!,
		uMapSize: null!,
		uMapLayer: null!,
		uCursor: null!,
		uTime: null!,
		uTilesPerRow: null!,
		uPaletteTexture: null!,
		uMapTexture: null!,
		uTilesTexture: null!,
	};

	attributeLocations: Record<string, GLint> = {
		aPosition: 0,
	};
}
