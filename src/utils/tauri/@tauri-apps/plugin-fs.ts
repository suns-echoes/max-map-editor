import { isTauri } from '@tauri-apps/api/core';
import {
	readFile as _readFile,
	readTextFile as _readTextFile,
	ReadFileOptions,
} from '@tauri-apps/plugin-fs';


export const readFile = isTauri() ? _readFile : async (path: string, options?: ReadFileOptions): Promise<Uint8Array> => {
	void options; // TODO: Implement options when needed.
	if (path.startsWith('resolve-resource/')) {
		const response = await fetch(path);
		return new Uint8Array(await response.arrayBuffer());
	}

	throw new Error(`no such file or directory: ${path}`);
};

export const readTextFile = isTauri() ? _readTextFile : async (path: string, options?: ReadFileOptions): Promise<string> => {
	void options; // TODO: Implement options when needed.
	if (path.startsWith('resolve-resource/')) {
		const response = await fetch(path);
		return response.text();
	}

	throw new Error(`no such file or directory: ${path}`);
};
