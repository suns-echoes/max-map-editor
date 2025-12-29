import type { OpenDialogOptions } from '@tauri-apps/plugin-dialog';
import { open } from '@tauri-apps/plugin-dialog';
import { homeDir } from '@tauri-apps/api/path';


export async function openFileDialog(params: OpenDialogOptions) {
	if (!params) {
		params = {
			defaultPath: await homeDir(),
			directory: false,
			multiple: false,
		};
	}

	return open(params);
}
