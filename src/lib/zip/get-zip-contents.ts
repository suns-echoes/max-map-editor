import { RustAPI } from '^src/bff/rust-api';


export async function getZipContents(path: string): Promise<void> {
	console.log('getZipContents');
	console.log('path', path);
	console.log(await RustAPI.loadZipFileContent(path));
}
