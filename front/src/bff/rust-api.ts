import { invoke as _invoke, isTauri } from '@tauri-apps/api/core';


// @ts-ignore
const invoke = isTauri() ? _invoke : async <T>(cmd: string, data?: any, options?: any): Promise<T> => {
	return Promise.resolve(undefined as any);
};


export const RustAPI = {
	openDevTools: (): Promise<void> => invoke('open_devtools'),

	validateMAXDir: (path: string): Promise<boolean> => invoke('validate_max_dir', { path }),

	imageToWRL: (path: string): Promise<[palette: Uint8Array, indexedImage: Uint8Array]> => invoke('image_to_wrl', { path }),

	xlog: (level: string, message: string): Promise<void> => invoke('xlog_command', { level, message }),
};
