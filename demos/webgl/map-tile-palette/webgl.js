class WebGL {
	#tilesTextureIndex = 0;
	#tilesTextureUnit = 0;
	#paletteTextureIndex = 1;
	#paletteTextureUnit = 1;
	#tileMapTextureIndex = 2;
	#tileMapTextureUnit = 2;
	#quadBuffer = null;

	#mapWidth = 0;
	#mapHeight = 0;
	#zoomLevel = 1;
	#panX = 0;
	#panY = 0;

	/** @private @type {HTMLCanvasElement} */
	#canvas = null;
	/** @private @type {WebGLRenderingContext} */
	#gl = null;
	/** @private @type {WebGLShader} */
	#program = null;

	/**
	 * @constructor
	 * @param {HTMLCanvasElement} canvas
	 */
	constructor(canvas) {
		this.#canvas = canvas;

		this.#initCanvas();
		this.#initWebGL();
	}

	/**
	 * @returns {this}
	 */
	clear() {
		this.#gl.clearColor(0, 0, 0, 1);
		this.#gl.clear(this.#gl.COLOR_BUFFER_BIT);
		return this;
	}

	/**
	 * @param {string} vertexShaderSource
	 * @param {string} fragmentShaderSource
	 * @returns {this}
	 */
	compileShaders(vertexShaderSource, fragmentShaderSource) {
		this.#createProgram(
			this.#createShader(vertexShaderSource, this.#gl.VERTEX_SHADER),
			this.#createShader(fragmentShaderSource, this.#gl.FRAGMENT_SHADER),
		);
		return this;
	}

	/**
	 * @param {Uint8Array} paletteData
	 * @returns {this}
	 */
	uploadPalette(paletteData) {
		this.#gl.activeTexture(this.#paletteTextureUnit);
		this.#createTexture(paletteData, this.#gl.RGB, 256, 1);
		return this;
	}

	/**
	 * @param {Uint8Array} paletteData
	 * @returns {this}
	 */
	updatePalette(paletteData) {
		this.#gl.activeTexture(this.#paletteTextureUnit);
		this.#gl.texImage2D(this.#gl.TEXTURE_2D, 0, this.#gl.RGB, 256, 1, 0, this.#gl.RGB, this.#gl.UNSIGNED_BYTE, paletteData);
		return this;
	}

	/**
	 * @param {Uint8Array} tilesData The 2D array of tiles data
	 * @param {number} width The width of tile
	 * @param {number} height The height of tile
	 * @param {number} hCount The number of tiles in a row
	 * @param {number} vCount The number of tiles in a column
	 * @returns {this}
	 */
	uploadTiles(tilesData, width, height, hCount, vCount) {
		const tileSetSizeLocation = this.#gl.getUniformLocation(this.#program, 'u_tileSetSize');
		this.#gl.uniform2fv(tileSetSizeLocation, [hCount, vCount]);

		this.#gl.activeTexture(this.#tilesTextureUnit);
		this.#createTexture(tilesData, this.#gl.ALPHA, width * hCount, height * vCount);
		return this;
	}

	/**
	 * @param {Uint8Array} tileMapData
	 * @param {number} width
	 * @param {number} height
	 * @returns {this}
	 */
	uploadTileMap(tileMapData, width, height) {
		const tileMapSizeLocation = this.#gl.getUniformLocation(this.#program, 'u_tileMapSize');
		this.#gl.uniform2fv(tileMapSizeLocation, [width, height]);

		this.#gl.activeTexture(this.#tileMapTextureUnit);
		this.#createTexture(tileMapData, this.#gl.RGBA, width, height);

		return this;
	}

	/**
	 * @param {number} width
	 * @param {number} height
	 * @param {number} panX
	 * @param {number} panY
	 * @returns {this}
	 */
	createMap(width, height, panX, panY, zoomLevel) {
		this.#mapWidth = width;
		this.#mapHeight = height;
		this.#zoomLevel = zoomLevel;
		this.#panX = panX;
		this.#panY = -panY;

		const w = this.#mapWidth / this.#canvas.offsetWidth * this.#zoomLevel;
		const h = this.#mapHeight / this.#canvas.offsetHeight * this.#zoomLevel;
		const _x = this.#panX * 2 * w;
		const _y = this.#panY * 2 * h;

		// Map vertexes (quad) position and size
		this.#quadBuffer = this.#createBuffer('a_position', new Float32Array([
			_x + w, _y + h, // ◤ 🡭
			_x - w, _y + h, // ◤ 🡬
			_x - w, _y - h, // ◤ 🡯
			_x + w, _y + h, // ◢ 🡭
			_x - w, _y - h, // ◢ 🡯
			_x + w, _y - h, // ◢ 🡮
		]), 2, this.#gl.FLOAT);

		// Texture coordinates
		this.#createBuffer('a_texcoord', new Float32Array([
			1, 1,
			0, 1,
			0, 0,
			1, 1,
			0, 0,
			1, 0,
		]), 2, this.#gl.FLOAT);

		return this;
	}

	/**
	 * @param {number} zoomLevel
	 * @returns {this}
	 */
	setZoomLevel(zoomLevel) {
		this.#zoomLevel = zoomLevel;

		const w = this.#mapWidth / this.#canvas.offsetWidth * this.#zoomLevel;
		const h = this.#mapHeight / this.#canvas.offsetHeight * this.#zoomLevel;
		const _x = this.#panX * 2 * w;
		const _y = this.#panY * 2 * h;

		this.#gl.bindBuffer(this.#gl.ARRAY_BUFFER, this.#quadBuffer);
		this.#gl.bufferData(this.#gl.ARRAY_BUFFER, new Float32Array([
			_x + w, _y + h, // ◤ 🡭
			_x - w, _y + h, // ◤ 🡬
			_x - w, _y - h, // ◤ 🡯
			_x + w, _y + h, // ◢ 🡭
			_x - w, _y - h, // ◢ 🡯
			_x + w, _y - h, // ◢ 🡮
		]), this.#gl.STATIC_DRAW);

		return this;
	}

	/**
	 * @param {number} panX
	 * @param {number} panY
	 * @returns {this}
	 */
	setPanning(panX, panY) {
		this.#panX = panX;
		this.#panY = -panY;

		const w = this.#mapWidth / this.#canvas.offsetWidth * this.#zoomLevel;
		const h = this.#mapHeight / this.#canvas.offsetHeight * this.#zoomLevel;
		const _x = this.#panX * 2 * w;
		const _y = this.#panY * 2 * h;

		this.#gl.bindBuffer(this.#gl.ARRAY_BUFFER, this.#quadBuffer);
		this.#gl.bufferData(this.#gl.ARRAY_BUFFER, new Float32Array([
			_x + w, _y + h, // ◤ 🡭
			_x - w, _y + h, // ◤ 🡬
			_x - w, _y - h, // ◤ 🡯
			_x + w, _y + h, // ◢ 🡭
			_x - w, _y - h, // ◢ 🡯
			_x + w, _y - h, // ◢ 🡮
		]), this.#gl.STATIC_DRAW);

		return this;
	}

	// Slow down things a bit.
	#frameCounterReset = 8;
	#frameCounter = this.#frameCounterReset;

	/**
	 * @param {(context: Render) => void} renderPipeline
	 * @returns {this}
	 */
	render(renderPipeline) {
		if (--this.#frameCounter === 0) {
			this.#frameCounter = this.#frameCounterReset;

			renderPipeline(this);
		}

		// This is 6 points of 2 vertices creating tile quad.
		const positionPoints = 6;
		this.#gl.drawArrays(this.#gl.TRIANGLES, 0, positionPoints);

		requestAnimationFrame(() => this.render(renderPipeline));

		return this;
	}

	/**
	 * @private @method
	 * @returns {void}
	 */
	#initCanvas() {
		this.#canvas.width = this.#canvas.offsetWidth;
		this.#canvas.height = this.#canvas.offsetHeight;
	}

	/**
	 * @private @method
	 * @returns {void}
	 */
	#initWebGL() {
		const gl = this.#canvas.getContext('webgl');
		if (!gl) throw new Error('Unable to initialize WebGL');

		this.#tilesTextureUnit = gl.TEXTURE0;
		this.#paletteTextureUnit = gl.TEXTURE1;
		this.#tileMapTextureUnit = gl.TEXTURE2;

		this.#gl = gl;
	}

	/**
	 * @private @method
	 * @param {string} source
	 * @param {WebGLRenderingContextBase['VERTEX_SHADER'] | WebGLRenderingContextBase['FRAGMENT_SHADER']} type
	 * @returns {WebGLShader}
	 */
	#createShader(source, type) {
		const shader = this.#gl.createShader(type);
		if (!shader) throw new Error('Unable to create WebGL shader');

		this.#gl.shaderSource(shader, source);
		this.#gl.compileShader(shader);

		return shader;
	}

	/**
	 * @private @method
	 * @param {WebGLShader} vertexShader
	 * @param {WebGLShader} fragmentShader
	 * @returns {void}
	 */
	#createProgram(vertexShader, fragmentShader) {
		const program = this.#gl.createProgram();
		if (!program) throw new Error('Unable to create WebGL program');

		this.#gl.attachShader(program, vertexShader);
		this.#gl.attachShader(program, fragmentShader);
		this.#gl.linkProgram(program);
		this.#gl.useProgram(program);
		this.#program = program;

		// tell gl to use texture unit 0 for tile
		const tilesLocation = this.#gl.getUniformLocation(program, 'u_tiles');
		this.#gl.uniform1i(tilesLocation, this.#tilesTextureIndex);

		// tell gl to use texture unit 1 for palette
		const paletteLocation = this.#gl.getUniformLocation(program, 'u_palette');
		this.#gl.uniform1i(paletteLocation, this.#paletteTextureIndex);

		const tileMapLocation = this.#gl.getUniformLocation(program, 'u_tileMap');
		this.#gl.uniform1i(tileMapLocation, this.#tileMapTextureIndex);
	}

	/**
	 * @private @method
	 * @param {ArrayBufferView} textureData
	 * @param {GLint} format
	 * @param {number} width
	 * @param {number} height
	 * @returns {WebGLTexture}
	 */
	#createTexture(textureData, format, width = textureData.byteLength, height = 1) {
		const texture = this.#gl.createTexture();
		if (!texture) throw new Error('Unable to create WebGL texture');

		this.#gl.bindTexture(this.#gl.TEXTURE_2D, texture);
		this.#gl.texParameteri(this.#gl.TEXTURE_2D, this.#gl.TEXTURE_WRAP_S, this.#gl.CLAMP_TO_EDGE);
		this.#gl.texParameteri(this.#gl.TEXTURE_2D, this.#gl.TEXTURE_WRAP_T, this.#gl.CLAMP_TO_EDGE);
		this.#gl.texParameteri(this.#gl.TEXTURE_2D, this.#gl.TEXTURE_MIN_FILTER, this.#gl.NEAREST);
		this.#gl.texParameteri(this.#gl.TEXTURE_2D, this.#gl.TEXTURE_MAG_FILTER, this.#gl.NEAREST);
		this.#gl.texImage2D(this.#gl.TEXTURE_2D, 0, format, width, height, 0, format, this.#gl.UNSIGNED_BYTE, textureData);

		return texture;
	}

	/**
	 * @private @method
	 * @param {string} attribName
	 * @param {ArrayBuffer} bufferData
	 * @param {GLint} size
	 * @param {GLenum} type
	 * @returns {WebGLBuffer}
	 */
	#createBuffer(attribName, bufferData, size, type) {
		const buffer = this.#gl.createBuffer();
		if (!buffer) throw new Error('Unable to create WebGL buffer');

		this.#gl.bindBuffer(this.#gl.ARRAY_BUFFER, buffer);
		this.#gl.bufferData(this.#gl.ARRAY_BUFFER, bufferData, this.#gl.STATIC_DRAW);
		const index = this.#gl.getAttribLocation(this.#program, attribName);
		this.#gl.enableVertexAttribArray(index);
		this.#gl.vertexAttribPointer(index, size, type, false, 0, 0);

		return buffer;
	}
}
