import vertexShaderSource from './shaders/map.vs';
import fragmentShaderSource from './shaders/map.fs';
import { Perf } from '^lib/perf/perf.ts';
import { WebGL2 } from '^lib/webgl2/webgl2.ts';
import { MAP_LAYERS } from '^consts/map-consts.ts';
import { mat4_createIdentity, mat4_identity, mat4_scale, mat4_translate } from '^lib/math/mat4.ts';
import { lookAt } from '^lib/math/3d.ts';
import { printDebugInfo } from '^lib/debug/debug.ts';
import { FPS } from '^lib/webgl2/fps.ts';


export class WglMap extends WebGL2 {
	constructor(canvas: HTMLCanvasElement) {
		printDebugInfo('WglMap::constructor');
		super(canvas);

		this.tileCapability = this.getTileCapability();
		this.textures = new Array(this.tileCapability.maxTileTextures).fill(null);

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

	private _viewMatrix = mat4_createIdentity();

	initView() {
		const cameraPosition: Vec3 = [0, 0, this._defaultCameraPositionZ];
		const cameraTarget: Vec3 = [0, 0, -this._defaultCameraPositionZ];
		const cameraUp: Vec3 = [0, 1, 0];
		lookAt(this._viewMatrix, cameraPosition, cameraTarget, cameraUp);
		this.gl.uniformMatrix4fv(this.uniformLocations.uView, false, this._viewMatrix);
	}

	private _modelMatrix = mat4_createIdentity();

	initModel() {
		this.factor = this.mapModelHeight / this.gl.canvas.height;
		mat4_identity(this._modelMatrix);
		// mat4_translate(this.modelMatrix, this.modelMatrix, [0, 0, -1]);
		mat4_scale(this._modelMatrix, this._modelMatrix, [this.factor, this.factor, 1]);
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

	private _defaultCameraPositionZ = 1;

	private _updateCameraZ(dz: number) {
		this.camera[2] += dz * Math.sqrt(this._defaultCameraPositionZ - this.camera[2]) * 0.25;
		this._limitMapZoom();
	}

	/**
	 * Limit camera zoom from "show whole map" to x2 zoom
	 */
	private _limitMapZoom() {
		if (this.camera[2] < -(this.factor - 1)) {
			this.camera[2] = -(this.factor - 1);
		} else if (this.camera[2] > 0.5) {
			this.camera[2] = 0.5;
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
			this._updateCameraZ(dz);

			const newMapZoom = 1 / (this._defaultCameraPositionZ - this.camera[2]);
			const zoomFactor = newMapZoom / this.mapZoom;
			this.mapZoom = newMapZoom;
			this.gl.uniform1f(this.uniformLocations.uZoom, this.mapZoom);

			// this._updatePanToScreenCenter(dx, dy, zoomFactor);
			this._updatePanToCursor(dx, dy, zoomFactor, cursorX, cursorY);
		} else {
			this._updatePan(dx, dy);
		}

		const cameraPanCoefficient = this.factor * 2 / this.mapZoom;
		this.camera[0] = this._mapPanX / this.mapModelWidth * cameraPanCoefficient;
		this.camera[1] = this._mapPanY / this.mapModelHeight * cameraPanCoefficient;

		const view = mat4_createIdentity();
		mat4_translate(view, this._viewMatrix, this.camera);

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
	initMap(mapData: Uint8Array, width: number, height: number) {
		if (this.textures[this.MAP_TEXTURE]) {
			this.gl.deleteTexture(this.mapTexture);
		}
		this.mapWidth = width;
		this.mapHeight = height;
		this.mapModelWidth = width * 64;
		this.mapModelHeight = height * 64;
		this.textures[this.MAP_TEXTURE] = this.createTexture(this.MAP_TEXTURE, mapData, width, height, this.gl.RGBA, '3d', MAP_LAYERS);
		this.gl.uniform1i(this.uniformLocations.uMapTexture, this.MAP_TEXTURE);

		this.initModel();
	}

	initTilesets(tileDataSets: Uint8Array[]) {
		const perf = Perf('WglMap::uploadTilesets');

		for (let i = 0; i < tileDataSets.length; i++) {
			const textureUnit = this[`TILES_TEXTURE${i}` as keyof WglMap] as GLenum;
			if (textureUnit === undefined) {
				throw new Error(`Fatal: Texture unit overflow: ${i}`);
			}
			this.createTexture(textureUnit, tileDataSets[i], this.tileCapability.tilesTexWidth, this.tileCapability.tilesTexWidth, this.gl.RGBA);
			this.gl.uniform1i(this.uniformLocations[`uTilesTexture${i}`], textureUnit);
		}

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
				this.gl.uniform1i(this.uniformLocations.uAnimationFrame_10fps, this._animationFrame_10fps);
			}
			if (time % 125 === 0) {
				this._animationFrame_8fps = this._animationFrame_8fps + 1;
				if (this._animationFrame_8fps === this._animationFrameCycle) this._animationFrame_8fps = 0;
				this.gl.uniform1i(this.uniformLocations.uAnimationFrame_8fps, this._animationFrame_8fps);
			}
			if (time % 150 === 0) {
				this._animationFrame_6fps = this._animationFrame_6fps + 1;
				if (this._animationFrame_6fps === this._animationFrameCycle) this._animationFrame_6fps = 0;
				this.gl.uniform1i(this.uniformLocations.uAnimationFrame_6fps, this._animationFrame_6fps);
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
	TILES_TEXTURE0 = 2;

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
		uTilesTexture0: null!,
		// uTilesTexture1: null!,
		// uTilesTexture2: null!,
		// uTilesTexture3: null!,
	};

	attributeLocations: Record<string, GLint> = {
		aMapPosition: 0,
		aTexCoord: 1,
	};
}
