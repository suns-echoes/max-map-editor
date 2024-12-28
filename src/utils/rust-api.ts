import { invoke } from '@tauri-apps/api/core';


export const RustAPI = {
	openDevTools: (): Promise<void> => invoke('open_devtools'),
	closeDevTools: (): Promise<void> => invoke('close_devtools'),
	isDevToolsOpen: (): Promise<boolean> => invoke('is_devtools_open'),

	updateMAXPath: (): Promise<boolean> => invoke('update_max_path'),
	checkMAXDir: (path: string): Promise<boolean> => invoke('check_max_dir', { path }),

	fileExists: (path: string): Promise<boolean> => invoke('file_exists', { path }),

	readWRLFile: (path: string): Promise<Uint8Array> => invoke('read_wrl_file', { path }),
	writeWRLFile: (path: string, data: Uint8Array): Promise<boolean> => invoke('write_wrl_file', data, { headers: { path } }),

	getZipFileList: (path: string): Promise<void> => invoke('get_zip_file_list', { path }),
	loadZipFileContent: (path: string): Promise<void> => invoke('load_zip_file_content', { path }),

	hashMD5: (data: Uint8Array): Promise<string> => invoke('hash_md5', data),
};
