import {
	appDataDir as _appDataDir,
	appLocalDataDir as _appLocalDataDir,
	resourceDir	as _resourceDir,
	resolveResource as _resolveResource,
} from '@tauri-apps/api/path';

import { isTauri } from '^tauri/is-tauri.ts';

export const appDataDir = isTauri ? _appDataDir : () => Promise.resolve('APP_DATA_DIR_NOT_AVAILABLE');
export const appLocalDataDir = isTauri ? _appLocalDataDir : () => Promise.resolve('APP_LOCAL_DATA_DIR_NOT_AVAILABLE');
export const resourceDir = isTauri ? _resourceDir : () => Promise.resolve('RESOURCE_DIR_NOT_AVAILABLE');

export const resolveResource = isTauri ? _resolveResource : async (resource: string) => {
	return 'resolve-resource/' + resource;
};
