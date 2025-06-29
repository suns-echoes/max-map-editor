import type { WglMap } from '^src/ui/main-window/wgl-map/wgl/wgl-map';
import { Value } from '^lib/reactive/value.class.ts';


export const AppState = {
	mapProject: new Value<MapProject | null>(null),
	mapSize: new Value<Size>({ width: 0, height: 0 }),
	palette: new Value<Uint8Array | null>(null),
	map: new Value<Uint8Array | null>(null),
	tiles: new Value<Tiles | null>(null),

	wglMap: new Value<WglMap | null>(null),

	reset() {
		this.mapProject.set(null);
		this.mapSize.set({ width: 0, height: 0 });
		this.palette.set(null);
		this.map.set(null);
		this.tiles.set(null);
	}
};
