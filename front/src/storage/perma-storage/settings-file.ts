import { isTauri } from '@tauri-apps/api/core';
import { printDebugInfo } from '^lib/debug/debug.ts';
import { debounce } from '^lib/flow-control/debounce.ts';
import { fs } from '^lib/fs/fs.ts';
import { deepAssignEqual } from '^lib/object-utils/deep-assign-equal.ts';


interface SettingsFileData {
	debug: {
		showDevTools: boolean,
	},
	window: {
		x: number,
		y: number,
		width: number,
		height: number,
		maximized: boolean,
	},
	max: {
		path: string,
	},
	setup: boolean,
}


export const SettingsFile = new class SettingsFile {
	async sync() {
		await printDebugInfo('SettingsFile::sync');

		if (!isTauri()) {
			this.#data.setup = false;
			return;
		} else if (this.#loaded) {
			await this.#write();
		} else {
			await this.#initialize();
		}
	}

	getAll(): SettingsFileData {
		return this.#data;
	}

	get<T extends keyof SettingsFileData>(key: T): SettingsFileData[T] {
		return this.#data[key];
	}

	set(values: PartialDeep<SettingsFileData>): this {
		this.#needSync = deepAssignEqual(this.#data, values);
		return this;
	}

	#data: SettingsFileData = {
		debug: {
			showDevTools: false,
		},
		window: {
			x: screen.width / 2 - 400,
			y: screen.height / 2 - 300,
			width: 800,
			height: 600,
			maximized: false,
		},
		max: {
			path: '',
		},
		setup: true,
	};

	#needSync = false;
	#loaded = false;

	async #initialize() {
		await printDebugInfo('SettingsFile::#initialize');
		if (!(await fs.appLocalDataDir.exists('./settings.json'))) {
			this.#needSync = true;
			await this.#write();
		} else {
			await this.#load();
		}
		this.#loaded = true;
	}

	async #load() {
		await printDebugInfo('SettingsFile::#load');
		this.#data = await fs.appLocalDataDir.readJSONFile('./settings.json');
	}

	/**
	 * @returns The `true` if data was written, `false` if not.
	 */
	#write = (() => debounce(async (): Promise<boolean> => {
		if (this.#needSync) {
			await printDebugInfo('SettingsFile::#write');
			await fs.appLocalDataDir.writeJSONFile('./settings.json', this.#data);
			this.#needSync = false;
			return true;
		} else {
			await printDebugInfo('SettingsFile::#write (skipped)');
		}
		return false;
	}, 1000))();
};
