import { isTauri } from '@tauri-apps/api/core';
import {
	resolveResource as _resolveResource,
} from '@tauri-apps/api/path';


export { appDataDir, appLocalDataDir, resourceDir } from '@tauri-apps/api/path';


export const resolveTextResource = isTauri() ? _resolveResource : async (resource: string) => {
	return 'resolve-resource/' + resource;
};
