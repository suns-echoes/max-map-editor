import { appLocalDataDir } from '@tauri-apps/api/path';
import { BaseDirectory, exists, readTextFile, writeTextFile } from '@tauri-apps/plugin-fs';

import { RustAPI } from '^lib/rust-api.ts';


export const fs = {
	appLocalDataDir: {

		async exists(path: string): Promise<boolean> {
			console.log(await appLocalDataDir());

			if (path === '.') return exists(await appLocalDataDir());
			return exists(path, { baseDir: BaseDirectory.AppLocalData });
		},

		async mkdir(path: string): Promise<void> {
			if (path === '.') this.mkdir(await appLocalDataDir());
			return this.mkdir(path);
		},

		async readJSONFile<T>(path: string): Promise<T> {
			return JSON.parse(await readTextFile(
				path,
				{ baseDir: BaseDirectory.AppLocalData },
			));
		},

		async writeJSONFile<T>(path: string, data: T): Promise<void> {
			await writeTextFile(path, JSON.stringify(data, null, '\t'), { baseDir: BaseDirectory.AppLocalData });
		},

	},

	maxData: {

		async readWRLFile(path: string): Promise<Uint8Array> {
			return RustAPI.readWRLFile(path);
		},

		async writeWRLFile(path: string, data: ArrayBuffer): Promise<boolean> {
			return RustAPI.writeWRLFile(path, new Uint8Array(data));
		},

	},
};
