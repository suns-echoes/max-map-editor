import { open } from '@tauri-apps/plugin-dialog';
import { homeDir } from '@tauri-apps/api/path';


export async function openFolderDialog(defaultPath?: string) {
	return await open({
		defaultPath: defaultPath ?? await homeDir(),
		directory: true,
		multiple: false,
	});
}
