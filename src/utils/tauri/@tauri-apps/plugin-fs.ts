import {
	readTextFile as _readTextFile,
} from '@tauri-apps/plugin-fs';

import { isTauri } from '^tauri/is-tauri.ts';


export const readTextFile = isTauri ? _readTextFile : async (path: string): Promise<string> => {
	if (path.startsWith('resolve-resource/')) {
		const response = await fetch(path);
		return response.text();
	}

	throw new Error(`no such file or directory: ${path}`);
};
