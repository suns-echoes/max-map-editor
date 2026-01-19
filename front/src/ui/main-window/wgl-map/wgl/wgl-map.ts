import vertexShaderSource from './shaders/map.vs?raw';
import fragmentShaderSource from './shaders/map.fs?raw';
import { Perf } from '^lib/perf/perf.ts';
import { WebGL2 } from '^lib/webgl2/webgl2.ts';
import { MAP_LAYERS } from '^consts/map-consts.ts';
import { mat4_createIdentity, mat4_identity, mat4_scale, mat4_translate } from '^lib/math/mat4.ts';
import { orthographic } from '^lib/math/3d.ts';
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

		this.initViewport();
		this.initView();
		this.initModel();
		this.initCursor();
		this.createMapTexCoordBuffer();
		this.createMapMeshBuffer();

		this.clear();

		this.gl.enable(this.gl.BLEND);
		this.gl.blendFunc(this.gl.SRC_ALPHA, this.gl.ONE_MINUS_SRC_ALPHA);

		printDebugInfo('WglMap::constructor done');
	}

	onCanvasResize() {
		this.initViewport();
		this.initModel();
	}

	/**
	 * Initialize viewport with orthographic projection for 2D map rendering.
	 * Maps screen coordinates where (0,0) is center, positive X is right, positive Y is up.
	 */
	override initViewport() {
		const canvas = this.gl.canvas as HTMLCanvasElement;
		canvas.width = canvas.parentElement!.clientWidth;
		canvas.height = canvas.parentElement!.clientHeight;
		this.aspect = canvas.width / canvas.height;

		// Orthographic projection: half-width and half-height in screen pixels
		const halfWidth = canvas.width * 0.5;
		const halfHeight = canvas.height * 0.5;

		orthographic(
			this.viewportProjectionMatrix,
			-halfWidth, halfWidth,    // left, right
			-halfHeight, halfHeight,  // bottom, top
			-1, 1                     // near, far
		);

		this.gl.uniformMatrix4fv(this.uniformLocations.uProjection, false, this.viewportProjectionMatrix);
		this.gl.viewport(0, 0, canvas.width, canvas.height);
	}

	private _viewMatrix = mat4_createIdentity();

	initView() {
		// With orthographic projection, view is just identity (camera at origin)
		mat4_identity(this._viewMatrix);
		this.gl.uniformMatrix4fv(this.uniformLocations.uView, false, this._viewMatrix);
	}

	private _modelMatrix = mat4_createIdentity();

	initModel() {
		if (this.mapModelHeight === 0) return;

		// factor = ratio of map pixels to screen pixels at zoom 1
		this.factor = this.mapModelHeight / this.gl.canvas.height;

		// Scale model to map's pixel dimensions (centered at origin)
		// Vertices are [-0.5, 0.5] so scaling by mapModelWidth/Height gives pixel size
		mat4_identity(this._modelMatrix);
		mat4_scale(this._modelMatrix, this._modelMatrix, [this.mapModelWidth, this.mapModelHeight, 1]);
		this.gl.uniformMatrix4fv(this.uniformLocations.uModel, false, this._modelMatrix);
	}

	private _cursor: Vec2 = new Float32Array([56, 56]);

	initCursor() {
		this.gl.uniform1f(this.uniformLocations.uZoom, this.mapZoom);
		this.gl.uniform2fv(this.uniformLocations.uCursor, this._cursor);
	}

	private _mapPanX: number = 0;
	private _mapPanY: number = 0;

	moveCursor(x: number, y: number) {
		const cursorX = x - this.gl.canvas.width * 0.5 - this._mapPanX;
		const cursorY = y - this.gl.canvas.height * 0.5 + this._mapPanY;
		const invTileSizeAndZoom = 0.015625 / this.mapZoom;

		this._cursor[0] = Math.floor(cursorX * invTileSizeAndZoom + this.mapWidth * 0.5);
		this._cursor[1] = Math.floor(cursorY * invTileSizeAndZoom + this.mapHeight * 0.5);

		this.gl.uniform2fv(this.uniformLocations.uCursor, this._cursor);
		this.render();
	}

	private _updateZoom(dz: number) {
		// Smooth zoom: multiply by a factor based on scroll delta
		const zoomSpeed = 0.1;
		this.mapZoom *= 1 + dz * zoomSpeed;
		this._limitMapZoom();
	}

	/**
	 * Limit zoom range:
	 * - Max: 2x (each map pixel = 2 screen pixels)
	 * - Min: fit map to smallest window dimension
	 */
	private _limitMapZoom() {
		const minZoom = Math.min(
			this.gl.canvas.width / this.mapModelWidth,
			this.gl.canvas.height / this.mapModelHeight
		);
		const maxZoom = 2;

		if (this.mapZoom < minZoom) {
			this.mapZoom = minZoom;
		} else if (this.mapZoom > maxZoom) {
			this.mapZoom = maxZoom;
		}
	}

	/**
	 * Zoom map relative to the screen center.
	 */
	// private _updatePanToScreenCenter(dx: number, dy: number, zoomFactor: number) {
	// 	this._mapPanX = (this._mapPanX + dx) * zoomFactor;
	// 	this._mapPanY = (this._mapPanY - dy) * zoomFactor;
	// 	this._limitMapPan();
	// }

	/**
	 * Zoom map relative to the cursor position.
	 */
	private _updatePanToCursor(dx: number, dy: number, zoomFactor: number, cursorX: number, cursorY: number) {
		const cursorToCenterX = cursorX - this.gl.canvas.width * 0.5;
		const cursorToCenterY = cursorY - this.gl.canvas.height * 0.5;
		this._mapPanX = (this._mapPanX + dx - cursorToCenterX) * zoomFactor + cursorToCenterX;
		this._mapPanY = (this._mapPanY - dy + cursorToCenterY) * zoomFactor - cursorToCenterY;
		this._limitMapPan();
	}

	/**
	 * Update map pan (no zoom).
	 */
	private _updatePan(dx: number, dy: number) {
		this._mapPanX += dx;
		this._mapPanY -= dy;
		this._limitMapPan();
	}

	private _mapMargin = 64 * 2;

	private _limitMapPan() {
		const mapMargin = this._mapMargin / this.mapZoom;
		if ((this.mapModelWidth + mapMargin) * this.mapZoom >= this.gl.canvas.width) {
			const maxPanX = (this.mapModelWidth * 0.5 + mapMargin) * this.mapZoom - this.gl.canvas.width * 0.5;
			if (this._mapPanX < -maxPanX) {
				this._mapPanX = -maxPanX;
			} else if (this._mapPanX > maxPanX) {
				this._mapPanX = maxPanX;
			}
		} else {
			this._mapPanX = 0;
		}
		if ((this.mapModelHeight + mapMargin) * this.mapZoom >= this.gl.canvas.height) {
			const maxPanY = (this.mapModelHeight * 0.5 + mapMargin) * this.mapZoom - this.gl.canvas.height * 0.5;
			if (this._mapPanY < -maxPanY) {
				this._mapPanY = -maxPanY;
			} else if (this._mapPanY > maxPanY) {
				this._mapPanY = maxPanY;
			}
		} else {
			this._mapPanY = 0;
		}
	}

	public moveCamera(dx: number, dy: number, dz: number, cursorX: number = 0, cursorY: number = 0) {
		if (dz !== 0) {
			const oldZoom = this.mapZoom;
			this._updateZoom(dz);
			const zoomFactor = this.mapZoom / oldZoom;

			this.gl.uniform1f(this.uniformLocations.uZoom, this.mapZoom);
			this._updatePanToCursor(dx, dy, zoomFactor, cursorX, cursorY);
		} else {
			this._updatePan(dx, dy);
		}

		// With orthographic projection, pan is directly in pixels
		this.camera[0] = this._mapPanX;
		this.camera[1] = this._mapPanY;
		this.camera[2] = 0;

		// Apply zoom and pan via view matrix
		const view = mat4_createIdentity();
		mat4_scale(view, view, [this.mapZoom, this.mapZoom, 1]);
		mat4_translate(view, view, this.camera);

		this.gl.uniformMatrix4fv(this.uniformLocations.uView, false, view);
		this.render();
	}

	initPalette(paletteData: Uint8Array) {
		this.createTexture(this.PALETTE_TEXTURE, paletteData, 256, 1, this.gl.RGBA);
		this.gl.uniform1i(this.uniformLocations.uPaletteTexture, this.PALETTE_TEXTURE);
	}

	/**
	 * Initialize or reinitialize the map texture.
	 * Use this method whenever the map size changes.
	 */
	initMap(mapData: Uint16Array, width: number, height: number) {
		if (this.textures[this.MAP_TEXTURE]) {
			this.gl.deleteTexture(this.mapTexture);
		}
		this.mapWidth = width;
		this.mapHeight = height;
		this.mapModelWidth = width * 64;
		this.mapModelHeight = height * 64;
		this.textures[this.MAP_TEXTURE] = this.createTexture(this.MAP_TEXTURE, mapData, width, height, this.gl.RGBA16UI, '3d', MAP_LAYERS);
		this.gl.uniform1i(this.uniformLocations.uMapTexture, this.MAP_TEXTURE);

		this.initModel();
	}

	initTilesets(tilesetData: Uint8Array, layers: number) {
		const perf = Perf('WglMap::uploadTilesets');

		const textureUnit = this.TILES_TEXTURE;
		this.createTexture(
			textureUnit,
			tilesetData,
			this.tileCapability.maxTextureSize,
			this.tileCapability.maxTextureSize,
			this.gl.R8UI,
			'3d',
			layers,
		);
		this.gl.uniform1i(this.uniformLocations.uTilesTexture, textureUnit);

		perf();
	}

	createMapTexCoordBuffer(): void {
		this.buffers.texCoord = this.createBuffer(new Float32Array([
			1, 1, // ◤ 🡭
			0, 1, // ◤ 🡬
			0, 0, // ◤ 🡯
			1, 1, // ◢ 🡭
			0, 0, // ◢ 🡯
			1, 0, // ◢ 🡮
		]), this.attributeLocations.aTexCoord, 2);
	}

	createMapMeshBuffer(): void {
		const mapMeshData = new Float32Array([
			 1,  1,  0, // ◤ 🡭
			-1,  1,  0, // ◤ 🡬
			-1, -1,  0, // ◤ 🡯
			 1,  1,  0, // ◢ 🡭
			-1, -1,  0, // ◢ 🡯
			 1, -1,  0, // ◢ 🡮
	   	]);
		for (let i = 0; i < MAP_LAYERS; i++) {
			this.buffers[`mapMesh${i}`] = this.createBuffer(mapMeshData, this.attributeLocations.aMapPosition, 3);
		}
	}

	render() {
		this.buffers.mapMesh0.use();
		this.gl.uniform1f(this.uniformLocations.uMapLayer, 0);
		this.gl.drawArrays(this.gl.TRIANGLES, 0, 6);

		this.buffers.mapMesh1.use();
		this.gl.uniform1f(this.uniformLocations.uMapLayer, 1);
		this.gl.drawArrays(this.gl.TRIANGLES, 0, 6);
	}

	private _animationFrame_6fps: number = 0;
	private _animationFrame_8fps: number = 0;
	private _animationFrame_10fps: number = 0;
	private _animationTimer: TimerID | null = null;
	/** Common number for all animation frames count. */
	private _animationFrameCycle: number = 7 * 6 * 5;

	enableAnimation() {
		if (this._animationTimer !== null) return;

		let time = 0;
		let timeCycle = 100 * 125 * 150;

		this._animationTimer = setInterval(() => {
			if (time % 100 === 0) {
				this._animationFrame_10fps = this._animationFrame_10fps + 1;
				if (this._animationFrame_10fps === this._animationFrameCycle) this._animationFrame_10fps = 0;
				this.gl.uniform1ui(this.uniformLocations.uAnimationFrame_10fps, this._animationFrame_10fps);
			}
			if (time % 125 === 0) {
				this._animationFrame_8fps = this._animationFrame_8fps + 1;
				if (this._animationFrame_8fps === this._animationFrameCycle) this._animationFrame_8fps = 0;
				this.gl.uniform1ui(this.uniformLocations.uAnimationFrame_8fps, this._animationFrame_8fps);
			}
			if (time % 150 === 0) {
				this._animationFrame_6fps = this._animationFrame_6fps + 1;
				if (this._animationFrame_6fps === this._animationFrameCycle) this._animationFrame_6fps = 0;
				this.gl.uniform1ui(this.uniformLocations.uAnimationFrame_6fps, this._animationFrame_6fps);
			}
			if ((time += 25) === timeCycle) time = 0;
			this.render();
		}, FPS(30));
	}

	disableAnimation() {
		if (this._animationTimer === null) return;
		clearInterval(this._animationTimer);
		this._animationTimer = null;
	}

	PALETTE_TEXTURE = 0;
	MAP_TEXTURE = 1;
	TILES_TEXTURE = 2;

	factor: number = 1;
	camera: Vec3 = [0, 0, 0];

	mapZoom: number = 1;
	mapWidth: number = 0;
	mapHeight: number = 0;
	mapModelWidth: number = 0;
	mapModelHeight: number = 0;

	mapTexture: WebGLTexture | null = null;

	uniformLocations: Record<string, WebGLUniformLocation> = {
		uModel: null!,
		uView: null!,
		uProjection: null!,
		uZoom: null!,
		uCursor: null!,
		uMapLayer: null!,
		uAnimationFrame_6fps: null!,
		uAnimationFrame_8fps: null!,
		uAnimationFrame_10fps: null!,
		uPaletteTexture: null!,
		uMapTexture: null!,
		uTilesTexture: null!,
	};

	attributeLocations: Record<string, GLint> = {
		aMapPosition: 0,
		aTexCoord: 1,
	};
}
