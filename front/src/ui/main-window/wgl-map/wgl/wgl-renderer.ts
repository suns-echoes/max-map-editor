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

	// Textures
	private paletteTexture: WebGLTexture | null = null;
	private tileTexture: WebGLTexture | null = null;

	private projectionMatrix = new Float32Array(16);

	constructor(canvas: HTMLCanvasElement) {
		const gl = canvas.getContext('webgl2');
		if (!gl) {
			throw new Error('WebGL2 not supported');
		}
		this.gl = gl;

		// === Simple color program ===
		this.simpleProgram = this.createProgram(simpleVS, simpleFS);
		this.uSimpleProjection = gl.getUniformLocation(this.simpleProgram, 'uProjection')!;
		this.uSimpleColor = gl.getUniformLocation(this.simpleProgram, 'uColor')!;

		this.simpleVAO = gl.createVertexArray()!;
		gl.bindVertexArray(this.simpleVAO);
		this.simplePositionBuffer = gl.createBuffer()!;
		gl.bindBuffer(gl.ARRAY_BUFFER, this.simplePositionBuffer);
		gl.enableVertexAttribArray(0);
		gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);

		// === Tile program ===
		this.tileProgram = this.createProgram(tileVS, tileFS);
		this.uTileProjection = gl.getUniformLocation(this.tileProgram, 'uProjection')!;
		this.uTileTexture = gl.getUniformLocation(this.tileProgram, 'uTileTexture')!;
		this.uPaletteTexture = gl.getUniformLocation(this.tileProgram, 'uPaletteTexture')!;

		this.tileVAO = gl.createVertexArray()!;
		gl.bindVertexArray(this.tileVAO);

		// Position buffer (location 0)
		this.tilePositionBuffer = gl.createBuffer()!;
		gl.bindBuffer(gl.ARRAY_BUFFER, this.tilePositionBuffer);
		gl.enableVertexAttribArray(0);
		gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);

		// TexCoord buffer (location 1)
		this.tileTexCoordBuffer = gl.createBuffer()!;
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

		console.log('WglRenderer initialized');
	}

	/**
	 * Upload palette data (256 RGBA colors)
	 */
	uploadPalette(paletteData: Uint8Array) {
		const gl = this.gl;

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

		console.log('Palette uploaded');
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

		console.log('Tile uploaded');
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

		const vertices = new Float32Array([
			x1, y1,
			x2, y1,
			x1, y2,
			x2, y1,
			x2, y2,
			x1, y2,
		]);

		gl.bindBuffer(gl.ARRAY_BUFFER, this.tilePositionBuffer);
		gl.bufferData(gl.ARRAY_BUFFER, vertices, gl.DYNAMIC_DRAW);

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

		console.log(`WglRenderer resized: ${canvas.width}x${canvas.height}`);
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

		// Two triangles forming a quad
		const vertices = new Float32Array([
			x1, y1,  // top-left
			x2, y1,  // top-right
			x1, y2,  // bottom-left
			x2, y1,  // top-right
			x2, y2,  // bottom-right
			x1, y2,  // bottom-left
		]);

		gl.bindBuffer(gl.ARRAY_BUFFER, this.simplePositionBuffer);
		gl.bufferData(gl.ARRAY_BUFFER, vertices, gl.DYNAMIC_DRAW);
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
}
