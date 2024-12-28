import type { WglMap } from '^components/x-main-window/components/x-wgl-map/wgl/wgl-map';
import { Signal } from '^utils/reactive/signal.class.ts';


export const AppState = {
	mapProject: new Signal<MapProject | null>(null),
	mapSize: new Signal<Size>({ width: 0, height: 0 }),
	palette: new Signal<Uint8Array | null>(null),
	map: new Signal<Uint8Array | null>(null),
	tiles: new Signal<Tiles | null>(null),

	wglMap: new Signal<WglMap | null>(null),
};
