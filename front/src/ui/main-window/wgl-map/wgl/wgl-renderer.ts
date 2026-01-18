import simpleVS from './shaders/simple.vs?raw';
import simpleFS from './shaders/simple.fs?raw';
import tileVS from './shaders/tile.vs?raw';
import tileFS from './shaders/tile.fs?raw';


/**
 * Simple WebGL2 renderer with orthographic projection.
 * Coordinates are in pixels, origin at top-left.
 */
export class WglRenderer {
	private gl: WebGL2RenderingContext;

	// Simple color program
	private simpleProgram: WebGLProgram;
	private simpleVAO: WebGLVertexArrayObject;
	private simplePositionBuffer: WebGLBuffer;
	private uSimpleProjection: WebGLUniformLocation;
	private uSimpleColor: WebGLUniformLocation;

	// Tile program
	private tileProgram: WebGLProgram;
	private tileVAO: WebGLVertexArrayObject;
	private tilePositionBuffer: WebGLBuffer;
	private tileTexCoordBuffer: WebGLBuffer;
	private uTileProjection: WebGLUniformLocation;
	private uTileTexture: WebGLUniformLocation;
	private uPaletteTexture: WebGLUniformLocation;
	private uTransform: WebGLUniformLocation;

	// Textures
	private paletteTexture: WebGLTexture | null = null;
	private tileTexture: WebGLTexture | null = null;
	private tileTextureCache = new Map<string, WebGLTexture>();

	// Palette data for color cycling
	private basePalette: Uint8Array | null = null;
	private workingPalette: Uint8Array | null = null;

	private projectionMatrix = new Float32Array(16);

	// Reusable vertex buffer for quads (2 triangles = 6 vertices * 2 coords = 12 floats)
	private readonly quadVertices = new Float32Array(12);

	constructor(canvas: HTMLCanvasElement) {
		const gl = canvas.getContext('webgl2');
		if (!gl) {
			throw new Error('WebGL2 not supported');
		}
		this.gl = gl;

		// === Simple color program ===
		this.simpleProgram = this.createProgram(simpleVS, simpleFS);

		const uSimpleProjection = gl.getUniformLocation(this.simpleProgram, 'uProjection');
		const uSimpleColor = gl.getUniformLocation(this.simpleProgram, 'uColor');
		if (!uSimpleProjection || !uSimpleColor) {
			throw new Error('Failed to get simple program uniform locations');
		}
		this.uSimpleProjection = uSimpleProjection;
		this.uSimpleColor = uSimpleColor;

		const simpleVAO = gl.createVertexArray();
		const simplePositionBuffer = gl.createBuffer();
		if (!simpleVAO || !simplePositionBuffer) {
			throw new Error('Failed to create simple program WebGL resources');
		}
		this.simpleVAO = simpleVAO;
		this.simplePositionBuffer = simplePositionBuffer;

		gl.bindVertexArray(this.simpleVAO);
		gl.bindBuffer(gl.ARRAY_BUFFER, this.simplePositionBuffer);
		gl.enableVertexAttribArray(0);
		gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);

		// === Tile program ===
		this.tileProgram = this.createProgram(tileVS, tileFS);

		const uTileProjection = gl.getUniformLocation(this.tileProgram, 'uProjection');
		const uTileTexture = gl.getUniformLocation(this.tileProgram, 'uTileTexture');
		const uPaletteTexture = gl.getUniformLocation(this.tileProgram, 'uPaletteTexture');
		const uTransform = gl.getUniformLocation(this.tileProgram, 'uTransform');
		if (!uTileProjection || !uTileTexture || !uPaletteTexture || !uTransform) {
			throw new Error('Failed to get tile program uniform locations');
		}
		this.uTileProjection = uTileProjection;
		this.uTileTexture = uTileTexture;
		this.uPaletteTexture = uPaletteTexture;
		this.uTransform = uTransform;

		const tileVAO = gl.createVertexArray();
		const tilePositionBuffer = gl.createBuffer();
		const tileTexCoordBuffer = gl.createBuffer();
		if (!tileVAO || !tilePositionBuffer || !tileTexCoordBuffer) {
			throw new Error('Failed to create tile program WebGL resources');
		}
		this.tileVAO = tileVAO;
		this.tilePositionBuffer = tilePositionBuffer;
		this.tileTexCoordBuffer = tileTexCoordBuffer;

		gl.bindVertexArray(this.tileVAO);

		// Position buffer (location 0)
		gl.bindBuffer(gl.ARRAY_BUFFER, this.tilePositionBuffer);
		gl.enableVertexAttribArray(0);
		gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);

		// TexCoord buffer (location 1)
		gl.bindBuffer(gl.ARRAY_BUFFER, this.tileTexCoordBuffer);
		gl.bindBuffer(gl.ARRAY_BUFFER, this.tileTexCoordBuffer);
		gl.enableVertexAttribArray(1);
		gl.vertexAttribPointer(1, 2, gl.FLOAT, false, 0, 0);

		// Static tex coords for a quad
		const texCoords = new Float32Array([
			0, 0,  // top-left
			1, 0,  // top-right
			0, 1,  // bottom-left
			1, 0,  // top-right
			1, 1,  // bottom-right
			0, 1,  // bottom-left
		]);
		gl.bufferData(gl.ARRAY_BUFFER, texCoords, gl.STATIC_DRAW);

		// Initial setup
		this.resize();
	}

	/**
	 * Upload palette data (256 RGBA colors)
	 */
	uploadPalette(paletteData: Uint8Array) {
		const gl = this.gl;

		// Store base palette for color cycling
		this.basePalette = new Uint8Array(paletteData);
		this.workingPalette = new Uint8Array(paletteData);

		if (this.paletteTexture) {
			gl.deleteTexture(this.paletteTexture);
		}

		this.paletteTexture = gl.createTexture()!;
		gl.bindTexture(gl.TEXTURE_2D, this.paletteTexture);
		gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 256, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, paletteData);
		gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
		gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
		gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
		gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
	}

	/**
	 * Cycle colors in a palette range
	 * @param startIndex - First palette index in the range
	 * @param endIndex - Last palette index in the range
	 * @param direction - 0 = backward (shift colors down), 1 = forward (shift colors up)
	 */
	cycleColors(startIndex: number, endIndex: number, direction: number) {
		if (!this.basePalette || !this.workingPalette) return;

		const rangeSize = endIndex - startIndex + 1;
		if (rangeSize < 2) return;

		if (direction === 1) {
			// Forward: shift colors up (index N gets color from index N-1, first gets last)
			// Save the last color
			const lastR = this.workingPalette[(endIndex) * 4];
			const lastG = this.workingPalette[(endIndex) * 4 + 1];
			const lastB = this.workingPalette[(endIndex) * 4 + 2];
			const lastA = this.workingPalette[(endIndex) * 4 + 3];

			// Shift colors up
			for (let i = endIndex; i > startIndex; i--) {
				const dstIdx = i * 4;
				const srcIdx = (i - 1) * 4;
				this.workingPalette[dstIdx] = this.workingPalette[srcIdx];
				this.workingPalette[dstIdx + 1] = this.workingPalette[srcIdx + 1];
				this.workingPalette[dstIdx + 2] = this.workingPalette[srcIdx + 2];
				this.workingPalette[dstIdx + 3] = this.workingPalette[srcIdx + 3];
			}

			// First gets last
			const firstIdx = startIndex * 4;
			this.workingPalette[firstIdx] = lastR;
			this.workingPalette[firstIdx + 1] = lastG;
			this.workingPalette[firstIdx + 2] = lastB;
			this.workingPalette[firstIdx + 3] = lastA;
		} else {
			// Backward: shift colors down (index N gets color from index N+1, last gets first)
			// Save the first color
			const firstR = this.workingPalette[(startIndex) * 4];
			const firstG = this.workingPalette[(startIndex) * 4 + 1];
			const firstB = this.workingPalette[(startIndex) * 4 + 2];
			const firstA = this.workingPalette[(startIndex) * 4 + 3];

			// Shift colors down
			for (let i = startIndex; i < endIndex; i++) {
				const dstIdx = i * 4;
				const srcIdx = (i + 1) * 4;
				this.workingPalette[dstIdx] = this.workingPalette[srcIdx];
				this.workingPalette[dstIdx + 1] = this.workingPalette[srcIdx + 1];
				this.workingPalette[dstIdx + 2] = this.workingPalette[srcIdx + 2];
				this.workingPalette[dstIdx + 3] = this.workingPalette[srcIdx + 3];
			}

			// Last gets first
			const lastIdx = endIndex * 4;
			this.workingPalette[lastIdx] = firstR;
			this.workingPalette[lastIdx + 1] = firstG;
			this.workingPalette[lastIdx + 2] = firstB;
			this.workingPalette[lastIdx + 3] = firstA;
		}
	}

	/**
	 * Update the palette texture with current working palette (call after cycling)
	 */
	updatePaletteTexture() {
		if (!this.workingPalette || !this.paletteTexture) return;

		const gl = this.gl;
		gl.bindTexture(gl.TEXTURE_2D, this.paletteTexture);
		gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, 256, 1, gl.RGBA, gl.UNSIGNED_BYTE, this.workingPalette);
	}

	/**
	 * Upload a single tile (64x64 palette indices)
	 */
	uploadTile(tileData: Uint8Array) {
		const gl = this.gl;

		if (this.tileTexture) {
			gl.deleteTexture(this.tileTexture);
		}

		this.tileTexture = gl.createTexture()!;
		gl.bindTexture(gl.TEXTURE_2D, this.tileTexture);
		gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8UI, 64, 64, 0, gl.RED_INTEGER, gl.UNSIGNED_BYTE, tileData);
		gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
		gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
		gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
		gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
	}

	/**
	 * Upload all tiles from the tiles map and cache them
	 */
	uploadAllTiles(tiles: Map<string, { data: Uint8Array }>) {
		const gl = this.gl;

		// Clear existing cache
		for (const texture of this.tileTextureCache.values()) {
			gl.deleteTexture(texture);
		}
		this.tileTextureCache.clear();

		// Upload each tile
		for (const [id, tile] of tiles) {
			const texture = gl.createTexture()!;
			gl.bindTexture(gl.TEXTURE_2D, texture);
			gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8UI, 64, 64, 0, gl.RED_INTEGER, gl.UNSIGNED_BYTE, tile.data);
			gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
			gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
			gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
			gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
			this.tileTextureCache.set(id, texture);
		}
	}

	/**
	 * Draw a tile by ID at pixel coordinates
	 * @param tileId - Base tile ID (without transformation flags)
	 * @param x - X position in pixels
	 * @param y - Y position in pixels
	 * @param size - Tile size in pixels
	 * @param transform - Transformation flags: 0=none, 1=rot90(E), 2=rot180(S), 3=rot270(W), +4=flipH
	 */
	drawTileById(tileId: string, x: number, y: number, size: number = 64, transform: number = 0): boolean {
		const texture = this.tileTextureCache.get(tileId);
		if (!texture || !this.paletteTexture) {
			return false;
		}

		const gl = this.gl;

		gl.useProgram(this.tileProgram);
		gl.bindVertexArray(this.tileVAO);

		// Update projection
		gl.uniformMatrix4fv(this.uTileProjection, false, this.projectionMatrix);

		// Set transformation
		gl.uniform1i(this.uTransform, transform);

		// Bind textures
		gl.activeTexture(gl.TEXTURE0);
		gl.bindTexture(gl.TEXTURE_2D, texture);
		gl.uniform1i(this.uTileTexture, 0);

		gl.activeTexture(gl.TEXTURE1);
		gl.bindTexture(gl.TEXTURE_2D, this.paletteTexture);
		gl.uniform1i(this.uPaletteTexture, 1);

		// Update position buffer
		const x1 = x;
		const y1 = y;
		const x2 = x + size;
		const y2 = y + size;

		const v = this.quadVertices;
		v[0] = x1; v[1] = y1;
		v[2] = x2; v[3] = y1;
		v[4] = x1; v[5] = y2;
		v[6] = x2; v[7] = y1;
		v[8] = x2; v[9] = y2;
		v[10] = x1; v[11] = y2;

		gl.bindBuffer(gl.ARRAY_BUFFER, this.tilePositionBuffer);
		gl.bufferData(gl.ARRAY_BUFFER, v, gl.DYNAMIC_DRAW);

		gl.drawArrays(gl.TRIANGLES, 0, 6);
		return true;
	}

	/**
	 * Draw the uploaded tile at pixel coordinates
	 */
	drawTile(x: number, y: number, size: number = 64) {
		if (!this.tileTexture || !this.paletteTexture) {
			console.warn('Tile or palette not uploaded');
			return;
		}

		const gl = this.gl;

		gl.useProgram(this.tileProgram);
		gl.bindVertexArray(this.tileVAO);

		// Update projection
		gl.uniformMatrix4fv(this.uTileProjection, false, this.projectionMatrix);

		// Bind textures
		gl.activeTexture(gl.TEXTURE0);
		gl.bindTexture(gl.TEXTURE_2D, this.tileTexture);
		gl.uniform1i(this.uTileTexture, 0);

		gl.activeTexture(gl.TEXTURE1);
		gl.bindTexture(gl.TEXTURE_2D, this.paletteTexture);
		gl.uniform1i(this.uPaletteTexture, 1);

		// Update position buffer
		const x1 = x;
		const y1 = y;
		const x2 = x + size;
		const y2 = y + size;

		const v = this.quadVertices;
		v[0] = x1; v[1] = y1;
		v[2] = x2; v[3] = y1;
		v[4] = x1; v[5] = y2;
		v[6] = x2; v[7] = y1;
		v[8] = x2; v[9] = y2;
		v[10] = x1; v[11] = y2;

		gl.bindBuffer(gl.ARRAY_BUFFER, this.tilePositionBuffer);
		gl.bufferData(gl.ARRAY_BUFFER, v, gl.DYNAMIC_DRAW);

		gl.drawArrays(gl.TRIANGLES, 0, 6);
	}

	/**
	 * Handle canvas resize - updates viewport and projection matrix
	 */
	resize() {
		const canvas = this.gl.canvas as HTMLCanvasElement;
		const parent = canvas.parentElement;
		if (!parent) return;

		// Set canvas size to match container
		canvas.width = parent.clientWidth;
		canvas.height = parent.clientHeight;

		// Update viewport
		this.gl.viewport(0, 0, canvas.width, canvas.height);

		// Create orthographic projection matrix
		// Maps pixel coordinates to clip space
		// Origin at top-left, Y grows downward (like CSS)
		this.createOrthographicMatrix(
			0, canvas.width,    // left, right
			canvas.height, 0,   // bottom, top (flipped for Y-down)
			-1, 1               // near, far
		);
	}

	/**
	 * Create orthographic projection matrix
	 */
	private createOrthographicMatrix(
		left: number, right: number,
		bottom: number, top: number,
		near: number, far: number
	) {
		const m = this.projectionMatrix;
		const lr = 1 / (left - right);
		const bt = 1 / (bottom - top);
		const nf = 1 / (near - far);

		m[0] = -2 * lr;
		m[1] = 0;
		m[2] = 0;
		m[3] = 0;

		m[4] = 0;
		m[5] = -2 * bt;
		m[6] = 0;
		m[7] = 0;

		m[8] = 0;
		m[9] = 0;
		m[10] = 2 * nf;
		m[11] = 0;

		m[12] = (left + right) * lr;
		m[13] = (top + bottom) * bt;
		m[14] = (far + near) * nf;
		m[15] = 1;
	}

	/**
	 * Clear the canvas
	 */
	clear(r = 0.1, g = 0.0, b = 0.1, a = 1.0) {
		this.gl.clearColor(r, g, b, a);
		this.gl.clear(this.gl.COLOR_BUFFER_BIT);
	}

	/**
	 * Get the canvas element
	 */
	getCanvas(): HTMLCanvasElement {
		return this.gl.canvas as HTMLCanvasElement;
	}

	/**
	 * Draw a rectangle at pixel coordinates using simple color shader
	 */
	drawRect(x: number, y: number, width: number, height: number) {
		const gl = this.gl;
		const x1 = x;
		const y1 = y;
		const x2 = x + width;
		const y2 = y + height;

		gl.useProgram(this.simpleProgram);
		gl.bindVertexArray(this.simpleVAO);
		gl.uniformMatrix4fv(this.uSimpleProjection, false, this.projectionMatrix);

		// Reuse quad vertex buffer
		const v = this.quadVertices;
		v[0] = x1; v[1] = y1;
		v[2] = x2; v[3] = y1;
		v[4] = x1; v[5] = y2;
		v[6] = x2; v[7] = y1;
		v[8] = x2; v[9] = y2;
		v[10] = x1; v[11] = y2;

		gl.bindBuffer(gl.ARRAY_BUFFER, this.simplePositionBuffer);
		gl.bufferData(gl.ARRAY_BUFFER, v, gl.DYNAMIC_DRAW);
		gl.drawArrays(gl.TRIANGLES, 0, 6);
	}

	/**
	 * Set the drawing color for simple shapes
	 */
	setColor(r: number, g: number, b: number, a = 1.0) {
		this.gl.useProgram(this.simpleProgram);
		this.gl.uniform4f(this.uSimpleColor, r, g, b, a);
	}

	/**
	 * Enable additive blending (color = src + dst)
	 */
	enableAdditiveBlend() {
		const gl = this.gl;
		gl.enable(gl.BLEND);
		gl.blendFunc(gl.ONE, gl.ONE);
	}

	/**
	 * Disable blending
	 */
	disableBlend() {
		this.gl.disable(this.gl.BLEND);
	}

	/**
	 * Compile and link shader program
	 */
	private createProgram(vsSource: string, fsSource: string): WebGLProgram {
		const vs = this.compileShader(this.gl.VERTEX_SHADER, vsSource);
		const fs = this.compileShader(this.gl.FRAGMENT_SHADER, fsSource);

		const program = this.gl.createProgram()!;
		this.gl.attachShader(program, vs);
		this.gl.attachShader(program, fs);
		this.gl.linkProgram(program);

		if (!this.gl.getProgramParameter(program, this.gl.LINK_STATUS)) {
			const info = this.gl.getProgramInfoLog(program);
			throw new Error(`Failed to link program: ${info}`);
		}

		return program;
	}

	/**
	 * Compile a shader
	 */
	private compileShader(type: number, source: string): WebGLShader {
		const shader = this.gl.createShader(type)!;
		this.gl.shaderSource(shader, source);
		this.gl.compileShader(shader);

		if (!this.gl.getShaderParameter(shader, this.gl.COMPILE_STATUS)) {
			const info = this.gl.getShaderInfoLog(shader);
			const typeName = type === this.gl.VERTEX_SHADER ? 'vertex' : 'fragment';
			throw new Error(`Failed to compile ${typeName} shader: ${info}`);
		}

		return shader;
	}

	/**
	 * Dispose all WebGL resources
	 */
	dispose() {
		const gl = this.gl;

		// Delete tile texture cache
		for (const texture of this.tileTextureCache.values()) {
			gl.deleteTexture(texture);
		}
		this.tileTextureCache.clear();

		// Delete individual textures
		if (this.tileTexture) {
			gl.deleteTexture(this.tileTexture);
			this.tileTexture = null;
		}
		if (this.paletteTexture) {
			gl.deleteTexture(this.paletteTexture);
			this.paletteTexture = null;
		}

		// Delete buffers
		gl.deleteBuffer(this.simplePositionBuffer);
		gl.deleteBuffer(this.tilePositionBuffer);
		gl.deleteBuffer(this.tileTexCoordBuffer);

		// Delete VAOs
		gl.deleteVertexArray(this.simpleVAO);
		gl.deleteVertexArray(this.tileVAO);

		// Delete programs
		gl.deleteProgram(this.simpleProgram);
		gl.deleteProgram(this.tileProgram);

		// Clear palette data
		this.basePalette = null;
		this.workingPalette = null;
	}
}
