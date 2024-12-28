import { perspective } from '^utils/math/3d.ts';
import { mat4_createIdentity } from '^utils/math/mat4.ts';


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
		this.viewportFovy = fovy;
		this.gl.canvas.width = this.canvas.offsetWidth;
		this.gl.canvas.height = this.canvas.offsetHeight;
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
				throw new Error(`Fatal: Failed to compile shader: ${info}`);
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

	createBuffer(data: Float32Array, attributeLocation: number, size: number): WebGLBuffer {
		const buffer = this.gl.createBuffer();

		this.gl.bindBuffer(this.gl.ARRAY_BUFFER, buffer);
		this.gl.bufferData(this.gl.ARRAY_BUFFER, data, this.gl.STATIC_DRAW);
		this.gl.enableVertexAttribArray(attributeLocation);
		this.gl.vertexAttribPointer(attributeLocation, size, this.gl.FLOAT, false, 0, 0);

		return buffer;
	}

	createTexture(textureUnit: GLenum, data: Uint8Array, width: number, height: number, format: GLenum): WebGLTexture {
		const texture = this.gl.createTexture();
		if (!texture) {
			throw new Error('Fatal: Failed to create texture');
		}

		this.textures[textureUnit] = texture;

		this.gl.activeTexture(this.gl.TEXTURE0 + textureUnit);
		this.gl.bindTexture(this.gl.TEXTURE_2D, texture);
		this.gl.pixelStorei(this.gl.UNPACK_ALIGNMENT, 1);
		this.gl.texImage2D(this.gl.TEXTURE_2D, 0, format, width, height, 0, format, this.gl.UNSIGNED_BYTE, data);

		this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_MIN_FILTER, this.gl.NEAREST);
		this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_MAG_FILTER, this.gl.NEAREST);
		this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_WRAP_S, this.gl.CLAMP_TO_EDGE);
		this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_WRAP_T, this.gl.CLAMP_TO_EDGE);

		return texture;
	}

	clear() {
		this.gl.clearColor(0, 0.1, 0, 1);
		this.gl.clear(this.gl.COLOR_BUFFER_BIT | this.gl.DEPTH_BUFFER_BIT);
	}

	getTileCapability(): WglTileCapability {
		const tileSize = 64 ** 2;
		const maxTextureSize = this.gl.getParameter(this.gl.MAX_TEXTURE_SIZE);
		const maxTextureUnits = this.gl.getParameter(this.gl.MAX_TEXTURE_IMAGE_UNITS) - 2;

		const tilesPerCol = maxTextureSize;
		const tilesPerRow = Math.floor(maxTextureSize / tileSize);
		const maxTilesPerTexture = tilesPerCol * tilesPerRow;

		const capabilities: WglTileCapability = {
			maxTilesPerTexture,
			maxTileTextures: maxTextureUnits,
			maxTileCount: maxTextureUnits * maxTilesPerTexture,
			tilesPerCol,
			tilesPerRow,
			tilesTexWidth: tilesPerRow * tileSize,
			tilesTexHeight: tilesPerCol * tileSize
		};

		return capabilities;
	}

	getUniformLocations(program: WebGLProgram) {
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
			this.gl.deleteBuffer(buffer);
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
		maxTilesPerTexture: 0,
		maxTileTextures: 0,
		maxTileCount: 0,
		tilesPerRow: 0,
		tilesPerCol: 0,
		tilesTexWidth: 0,
		tilesTexHeight: 0,
	};

	buffers: Record<string, WebGLBuffer> = {};
	textures: WebGLTexture[] = [];
	uniformLocations: Record<string, WebGLUniformLocation> = {};
	attributeLocations: Record<string, GLint> = {};
}
