import { invoke as _invoke, isTauri } from '@tauri-apps/api/core';


// @ts-ignore
const invoke = isTauri() ? _invoke : async <T>(cmd: string, data?: any, options?: any): Promise<T> => {
	return Promise.resolve(undefined as any);
};


export const RustAPI = {
	ERROR_PATH_DOES_NOT_EXIST: 'Path does not exist.',
	ERROR_INVALID_MAX_PATH: 'Provided path is not a valid M.A.X. directory.',

	openDevTools: (): Promise<void> => invoke('open_devtools'),
	closeDevTools: (): Promise<void> => invoke('close_devtools'),
	isDevToolsOpen: (): Promise<boolean> => invoke('is_devtools_open'),

	validateMAXDir: (path: string): Promise<boolean> => invoke('validate_max_dir', { path }),
	reloadMAXPath: (): Promise<boolean> => invoke('reload_max_path'),

	updateMAXPath: (): Promise<boolean> => invoke('update_max_path'),

	fileExists: (path: string): Promise<boolean> => invoke('file_exists', { path }),

	readWRLFile: (path: string): Promise<Uint8Array> => invoke('read_wrl_file', { path }),
	writeWRLFile: (path: string, data: Uint8Array): Promise<boolean> => invoke('write_wrl_file', data, { headers: { path } }),

	hashMD5: (data: Uint8Array): Promise<string> => invoke('hash_md5', data),

	imageToWRL: (path: string): Promise<[palette: Uint8Array, indexedImage: Uint8Array]> => invoke('image_to_wrl', { path }),
};
