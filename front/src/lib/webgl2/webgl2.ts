import { TILE_LENGTH, TILE_SIZE } from '^consts/tile-consts.ts';
import { MAX_TEXTURE_SIZE_LIMIT } from '^consts/wgl-consts.ts';
import { perspective } from '^lib/math/3d.ts';
import { mat4_createIdentity } from '^lib/math/mat4.ts';


export class WebGL2 {
	constructor(canvas: HTMLCanvasElement) {
		const gl = canvas.getContext('webgl2');
		if (!gl) {
			throw new Error('WebGL2 not supported');
		}
		this.canvas = canvas;
		this.gl = gl;
	}

	viewportFovy: number = Math.PI * 0.5;
	viewportProjectionMatrix = mat4_createIdentity();
	initViewport(fovy: number = this.viewportFovy) {
		const canvas = this.gl.canvas as HTMLCanvasElement;
		this.viewportFovy = fovy;
		canvas.width = canvas.parentElement!.clientWidth;
		canvas.height = canvas.parentElement!.clientHeight;
		this.aspect = this.gl.canvas.width / this.gl.canvas.height;
		perspective(this.viewportProjectionMatrix, fovy, this.aspect, 0.1, 100);
		this.gl.uniformMatrix4fv(this.uniformLocations.uProjection, false, this.viewportProjectionMatrix);
		this.gl.viewport(0, 0, this.gl.canvas.width, this.gl.canvas.height);
	}

	createProgram(vertexShaderSource: string, fragmentShaderSource: string): WebGLProgram {
		function compileShader(gl: WebGL2RenderingContext, type: number, source: string): WebGLShader {
			const shader = gl.createShader(type);
			if (!shader) {
				throw new Error('Fatal: Failed to create shader');
			}

			gl.shaderSource(shader, source);
			gl.compileShader(shader);

			if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
				const info = gl.getShaderInfoLog(shader);
				gl.deleteShader(shader);
				const shaderType = type === gl.VERTEX_SHADER ? 'vertex' : 'fragment';
				throw new Error(`Fatal: Failed to compile ${shaderType} shader: ${info}`);
			}

			return shader;
		}

		const program = this.gl.createProgram();
		if (!program) {
			throw new Error('Fatal: Failed to create program');
		}

		this.gl.attachShader(program, compileShader(this.gl, this.gl.VERTEX_SHADER, vertexShaderSource));
		this.gl.attachShader(program, compileShader(this.gl, this.gl.FRAGMENT_SHADER, fragmentShaderSource));
		this.gl.linkProgram(program);

		if (!this.gl.getProgramParameter(program, this.gl.LINK_STATUS)) {
			const info = this.gl.getProgramInfoLog(program);
			this.gl.deleteProgram(program);
			throw new Error(`Fatal: Failed to link program: ${info}`);
		}

		return program;
	}

	createBuffer(data: Float32Array, attributeLocation: number, size: number): WebGL2BufferStruct {
		const buffer = this.gl.createBuffer();

		this.gl.bindBuffer(this.gl.ARRAY_BUFFER, buffer);
		this.gl.bufferData(this.gl.ARRAY_BUFFER, data, this.gl.STATIC_DRAW);
		this.gl.enableVertexAttribArray(attributeLocation);
		this.gl.vertexAttribPointer(attributeLocation, size, this.gl.FLOAT, false, 0, 0);

		return {
			glBuffer: buffer,
			use: () => {
				this.gl.bindBuffer(this.gl.ARRAY_BUFFER, buffer);
				// this.gl.enableVertexAttribArray(attributeLocation);
				// this.gl.vertexAttribPointer(attributeLocation, size, this.gl.FLOAT, false, 0, 0);
			},
			destroy: () => this.gl.deleteBuffer(buffer),
		};
	}

	createTexture(unit: GLenum, data: Uint8Array, width: number, height: number, format: typeof this.gl.RGBA | typeof this.gl.R8UI | typeof this.gl.RGBA8UI, target?: '2d'): WebGLTexture;
	createTexture(unit: GLenum, data: Uint16Array, width: number, height: number, format: typeof this.gl.RGBA16UI, target?: '2d'): WebGLTexture;
	createTexture(unit: GLenum, data: Uint8Array, width: number, height: number, format: typeof this.gl.RGBA | typeof this.gl.R8UI | typeof this.gl.RGBA8UI, target: '3d', depth: number): WebGLTexture;
	createTexture(unit: GLenum, data: Uint16Array, width: number, height: number, format: typeof this.gl.RGBA16UI, target: '3d', depth: number): WebGLTexture;
	createTexture(unit: GLenum, data: Uint8Array | Uint16Array, width: number, height: number, format: typeof this.gl.RGBA | typeof this.gl.R8UI | typeof this.gl.RGBA8UI | typeof this.gl.RGBA16UI, target: '2d' | '3d' = '2d', depth: number = 1): WebGLTexture {
		const texture = this.gl.createTexture();
		if (!texture) {
			throw new Error('Fatal: Failed to create texture');
		}

		const targetSampler = target === '3d'
			? this.gl.TEXTURE_2D_ARRAY
			: this.gl.TEXTURE_2D;

		this.gl.activeTexture(this.gl.TEXTURE0 + unit);
		this.gl.bindTexture(targetSampler, texture);
		this.gl.pixelStorei(this.gl.UNPACK_ALIGNMENT, 1);

		let internalFormat: GLenum;
		let pixelFormat: GLenum;
		let pixelType: GLenum;
		let isIntegerTexture: boolean = false;

		if (data instanceof Uint16Array && format === this.gl.RGBA16UI) {
			console.info("Creating RGBA16UI texture (16-bit unsigned integer).");
			internalFormat = this.gl.RGBA16UI;
			pixelFormat = this.gl.RGBA_INTEGER;
			pixelType = this.gl.UNSIGNED_SHORT;
			isIntegerTexture = true;
		} else if (data instanceof Uint8Array && format === this.gl.R8UI) {
			console.info("Creating R8UI texture (8-bit unsigned integer).");
			internalFormat = this.gl.R8UI;
			pixelFormat = this.gl.RED_INTEGER;
			pixelType = this.gl.UNSIGNED_BYTE;
			isIntegerTexture = true;
		} else if (data instanceof Uint8Array && format === this.gl.RGBA8UI) {
			console.info("Creating RGBA8UI texture (8-bit unsigned integer).");
			internalFormat = this.gl.RGBA8UI;
			pixelFormat = this.gl.RGBA_INTEGER;
			pixelType = this.gl.UNSIGNED_BYTE;
			isIntegerTexture = true;
		} else if (data instanceof Uint8Array && format === this.gl.RGBA) {
			console.info("Creating RGBA8 texture (8-bit normalized unsigned).");
			internalFormat = this.gl.RGBA8;
			pixelFormat = this.gl.RGBA;
			pixelType = this.gl.UNSIGNED_BYTE;
			isIntegerTexture = false;
		} else {
			const dataTypeName = Object.prototype.toString.call(data).slice(8, -1);
			const formatName = formatToString(this.gl, format);
			throw new Error(`Fatal: Unsupported texture combination: data=${dataTypeName}, format=${formatName}.`);
		}

		// Use texStorage for immutable texture allocation
		// Mipmap level is 1 because we explicitly don't generate mipmaps for integer textures,
		// and if we were to generate them for non-integer, texStorage handles it.
		const numMipLevels = 1; // Always 1 for simplicity in this combined function; could be more for filterable
		// textures where you generate mipmaps.

		if (target === '3d') {
			this.gl.texStorage3D(targetSampler, numMipLevels, internalFormat, width, height, depth);
			this.gl.texSubImage3D(targetSampler, 0, 0, 0, 0, width, height, depth, pixelFormat, pixelType, data);
		} else { // '2d'
			this.gl.texStorage2D(targetSampler, numMipLevels, internalFormat, width, height);
			this.gl.texSubImage2D(targetSampler, 0, 0, 0, width, height, pixelFormat, pixelType, data);
		}

		// Set texture parameters based on whether it's an integer texture
		if (isIntegerTexture) {
			// Integer textures MUST use NEAREST filtering and no mipmaps
			this.gl.texParameteri(targetSampler, this.gl.TEXTURE_MIN_FILTER, this.gl.NEAREST);
			this.gl.texParameteri(targetSampler, this.gl.TEXTURE_MAG_FILTER, this.gl.NEAREST);
			this.gl.texParameteri(targetSampler, this.gl.TEXTURE_BASE_LEVEL, 0);
			this.gl.texParameteri(targetSampler, this.gl.TEXTURE_MAX_LEVEL, 0);
		} else {
			// For non-integer textures (e.g., RGBA8), use common filtering.
			// You might want to generate mipmaps here if your textures are power-of-2
			// or if you handle NPOT mipmaps.
			this.gl.texParameteri(targetSampler, this.gl.TEXTURE_MIN_FILTER, this.gl.LINEAR); // Using LINEAR as a simple default
			this.gl.texParameteri(targetSampler, this.gl.TEXTURE_MAG_FILTER, this.gl.LINEAR);
			// Optionally, generate mipmaps for filterable textures:
			// if (width % 2 === 0 && height % 2 === 0) { // Simple check for power-of-2 for basic mipmapping
			//    this.gl.generateMipmap(targetSampler);
			//    this.gl.texParameteri(targetSampler, this.gl.TEXTURE_MIN_FILTER, this.gl.LINEAR_MIPMAP_LINEAR);
			// }
		}

		// Wrapping modes (common for both types)
		this.gl.texParameteri(targetSampler, this.gl.TEXTURE_WRAP_S, this.gl.CLAMP_TO_EDGE);
		this.gl.texParameteri(targetSampler, this.gl.TEXTURE_WRAP_T, this.gl.CLAMP_TO_EDGE);
		if (target === '3d') {
			this.gl.texParameteri(targetSampler, this.gl.TEXTURE_WRAP_R, this.gl.CLAMP_TO_EDGE);
		}

		this.textures.push(texture);
		return texture;
	}

	clear() {
		this.gl.clearColor(0, 0.1, 0, 1);
		this.gl.clear(this.gl.COLOR_BUFFER_BIT | this.gl.DEPTH_BUFFER_BIT);
	}

	getTileCapability(): WglTileCapability {
		function roundDownToPowerOfTwo(value: number): number {
			if (value <= 0) return 0;
			let power = 1;
			while (power <= value) {
				power *= 2;
			}
			return power / 2;
		}

		const maxTextureSize = roundDownToPowerOfTwo(
			Math.min(MAX_TEXTURE_SIZE_LIMIT, this.gl.getParameter(this.gl.MAX_TEXTURE_SIZE)),
		);

		const maxTextureLength = maxTextureSize * maxTextureSize;
		const maxTextureLayers = Math.min(MAX_TEXTURE_SIZE_LIMIT, this.gl.getParameter(this.gl.MAX_ARRAY_TEXTURE_LAYERS));
		const maxTextureUnits = this.gl.getParameter(this.gl.MAX_TEXTURE_IMAGE_UNITS) - 2;

		if (maxTextureSize < TILE_LENGTH || maxTextureLayers <= 0 || maxTextureUnits <= 0) {
			throw new Error('Fatal: WebGL2 capabilities are not sufficient for tile rendering');
		}

		const tilesPerCol = maxTextureSize;
		const tilesPerRow = Math.floor(maxTextureSize / TILE_LENGTH);
		const maxTilesPerTextureLayer = Math.floor(maxTextureLength / TILE_LENGTH);

		const capabilities: WglTileCapability = {
			maxTextureSize,
			maxTextureLayers,
			maxTilesPerTextureLayer,
			maxTiles: maxTilesPerTextureLayer * maxTextureLayers,
			tilesPerCol,
			tilesPerRow,
			tilesTexWidth: tilesPerRow * TILE_SIZE,
			tilesTexHeight: tilesPerCol * TILE_SIZE
		};

		return capabilities;
	}

	getUniformLocations(program: WebGLProgram): void {
		for (const uniformName of Object.keys(this.uniformLocations)) {
			const location = this.gl.getUniformLocation(program, uniformName);
			if (!location) {
				throw new Error(`Fatal: Failed to get uniform location: ${uniformName}`);
			}
			this.uniformLocations[uniformName] = location;
		}
	}

	cleanup() {
		console.info('WglMap::cleanup');

		const numTextureUnits = this.gl.getParameter(this.gl.MAX_TEXTURE_IMAGE_UNITS);
		for (let unit = 0; unit < numTextureUnits; unit++) {
			this.gl.activeTexture(this.gl.TEXTURE0 + unit);
			this.gl.bindTexture(this.gl.TEXTURE_2D, null);
			// this.gl.bindTexture(this.gl.TEXTURE_CUBE_MAP, null);
		}
		this.gl.bindBuffer(this.gl.ARRAY_BUFFER, null);
		this.gl.bindBuffer(this.gl.ELEMENT_ARRAY_BUFFER, null);
		this.gl.bindRenderbuffer(this.gl.RENDERBUFFER, null);
		this.gl.bindFramebuffer(this.gl.FRAMEBUFFER, null);
		for (const texture of this.textures) {
			this.gl.deleteTexture(texture);
		}
		for (const buffer of Object.values(this.buffers)) {
			buffer.destroy();
		}
		// this.gl.deleteRenderbuffer(someRenderbuffer);
		// this.gl.deleteFramebuffer(someFramebuffer);

		this.gl.useProgram(null);
	}

	render() {
		this.gl.drawArrays(this.gl.TRIANGLES, 0, 6);
	}

	gl: WebGL2RenderingContext = null!;
	canvas: HTMLCanvasElement = null!;

	aspect: number = 1;

	tileCapability: WglTileCapability = {
		maxTextureSize: 0,
		maxTextureLayers: 0,
		maxTilesPerTextureLayer: 0,
		maxTiles: 0,
		tilesPerRow: 0,
		tilesPerCol: 0,
		tilesTexWidth: 0,
		tilesTexHeight: 0
	};

	buffers: Record<string, WebGL2BufferStruct> = {};
	textures: WebGLTexture[] = [];
	uniformLocations: Record<string, WebGLUniformLocation> = {};
	attributeLocations: Record<string, GLint> = {};
}


type WebGL2BufferStruct = {
	glBuffer: WebGLBuffer;
	use: () => void;
	destroy: () => void;
}


function formatToString(gl: WebGL2RenderingContext, format: GLenum): string {
	switch (format) {
		case gl.RGBA: return 'gl.RGBA';
		case gl.RGBA8: return 'gl.RGBA8';
		case gl.R8UI: return 'gl.R8UI';
		case gl.RGBA16UI: return 'gl.RGBA16UI';
		case gl.RGBA_INTEGER: return 'gl.RGBA_INTEGER';
		default: return `UnknownFormat(0x${format.toString(16)})`;
	}
}
