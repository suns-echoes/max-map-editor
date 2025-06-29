import { RustAPI } from '^src/bff/rust-api';


export async function getZipFileList(path: string): Promise<void> {
	console.log('getZipFileList');
	console.log('path', path);
	console.log(await RustAPI.getZipFileList(path));
}
