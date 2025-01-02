import vertexShaderSource from './wgl-map.vs';
import fragmentShaderSource from './wgl-map.fs';
import { Perf } from '^utils/perf/perf.ts';
import { WebGL2 } from '^utils/webgl2/webgl2.ts';
import { MAP_LAYERS } from '^consts/map-consts.ts';
import { mat4_createIdentity, mat4_identity, mat4_scale, mat4_translate } from '^utils/math/mat4.ts';
import { lookAt } from '^utils/math/3d.ts';
import { printDebugInfo } from '^utils/debug/debug.ts';


export class WglMap extends WebGL2 {
	constructor(canvas: HTMLCanvasElement) {
		printDebugInfo('WglMap::constructor');
		super(canvas);

		this.tileCapability = this.getTileCapability();
		this.textures = new Array(this.tileCapability.maxTileTextures).fill(null);
		console.info('Tile capability:', this.tileCapability);

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

	viewMatrix = mat4_createIdentity();
	initView() {
		const cameraPosition: Vec3 = [0, 0, 1];
		const cameraTarget: Vec3 = [0, 0, -1];
		const cameraUp: Vec3 = [0, 1, 0];
		lookAt(this.viewMatrix, cameraPosition, cameraTarget, cameraUp);
		this.gl.uniformMatrix4fv(this.uniformLocations.uView, false, this.viewMatrix);
	}

	modelMatrix = mat4_createIdentity();
	initModel() {
		this.factor = this.mapModelHeight / this.gl.canvas.height;
		mat4_identity(this.modelMatrix);
		// mat4_translate(this.modelMatrix, this.modelMatrix, [0, 0, -1]);
		mat4_scale(this.modelMatrix, this.modelMatrix, [this.factor, this.factor, 1]);
		this.gl.uniformMatrix4fv(this.uniformLocations.uModel, false, this.modelMatrix);
	}

	cursor: Vec2 = new Float32Array([56, 56]);
	initCursor() {
		this.gl.uniform2fv(this.uniformLocations.uCursor, this.cursor);
	}

	moveCamera(dx: number, dy: number, dz: number) {
		const cameraZOrigin = 1;
		this.camera[2] += dz * Math.sqrt(cameraZOrigin - this.camera[2]) * 0.5;

		// Limit camera zoom from "show whole map" to x2 zoom
		if (this.camera[2] < -(this.factor - 1)) {
			this.camera[2] = -(this.factor - 1);
		} else if (this.camera[2] > 0.5) {
			this.camera[2] = 0.5;
		}

		const zoomLevel = 1 / (cameraZOrigin - this.camera[2]);

		this.camera[0] += dx / this.mapModelWidth * this.factor * 2 / zoomLevel;
		this.camera[1] -= dy / this.mapModelHeight * this.factor * 2 / zoomLevel;

		const view = mat4_createIdentity();
		mat4_translate(view, this.viewMatrix, this.camera);

		this.gl.uniformMatrix4fv(this.uniformLocations.uView, false, view);
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

	animationFrame_6fps: number = 0;
	animationFrame_8fps: number = 0;
	animationFrame_10fps: number = 0;
	animationTimer_6fps: TimerID | null = null;
	animationTimer_8fps: TimerID | null = null;
	animationTimer_10fps: TimerID | null = null;
	/** Common number for all animation frames count. */
	animationFrameCycle: number = 7 * 6 * 5;
	enableAnimation() {
		if (this.animationTimer_6fps !== null) {
			return;
		}

		this.animationTimer_6fps = setInterval(() => {
			this.gl.uniform1i(this.uniformLocations.uAnimationFrame_6fps, this.animationFrame_6fps++);
			if (this.animationFrame_6fps >= this.animationFrameCycle) this.animationFrame_6fps = 0;
			this.render();
		}, 167);
		this.animationTimer_8fps = setInterval(() => {
			this.gl.uniform1i(this.uniformLocations.uAnimationFrame_8fps, this.animationFrame_8fps++);
			if (this.animationFrame_8fps >= this.animationFrameCycle) this.animationFrame_8fps = 0;
			this.render();
		}, 125);
		this.animationTimer_10fps = setInterval(() => {
			this.gl.uniform1i(this.uniformLocations.uAnimationFrame_10fps, this.animationFrame_10fps++);
			if (this.animationFrame_10fps >= this.animationFrameCycle) this.animationFrame_10fps = 0;
			this.render();
		}, 100);
	}

	disableAnimation() {
		if (this.animationTimer_6fps === null) return;
		clearInterval(this.animationTimer_6fps);
		this.animationTimer_6fps = null;
		clearInterval(this.animationTimer_8fps!);
		this.animationTimer_8fps = null;
		clearInterval(this.animationTimer_10fps!);
		this.animationTimer_10fps = null;
	}

	PALETTE_TEXTURE = 0;
	MAP_TEXTURE = 1;
	TILES_TEXTURE0 = 2;

	factor: number = 1;
	camera: Vec3 = [0, 0, 0];

	mapModelWidth: number = 0;
	mapModelHeight: number = 0;

	mapTexture: WebGLTexture | null = null;

	uniformLocations: Record<string, WebGLUniformLocation> = {
		uModel: null!,
		uView: null!,
		uProjection: null!,
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
