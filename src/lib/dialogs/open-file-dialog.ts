import { open } from '@tauri-apps/plugin-dialog';
import { homeDir } from '@tauri-apps/api/path';


export async function openFileDialog(defaultPath?: string) {
	return await open({
		defaultPath: defaultPath ?? await homeDir(),
		directory: false,
		multiple: false,
	});
}
