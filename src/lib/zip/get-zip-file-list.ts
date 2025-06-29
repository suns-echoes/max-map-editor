import { RustAPI } from '^lib/rust-api.ts';


export async function getZipFileList(path: string): Promise<void> {
	console.log('getZipFileList');
	console.log('path', path);
	console.log(await RustAPI.getZipFileList(path));
}
