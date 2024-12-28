import { RustAPI } from '^utils/rust-api.ts';


export async function getZipContents(path: string): Promise<void> {
	console.log('getZipContents');
	console.log('path', path);
	console.log(await RustAPI.loadZipFileContent(path));
}
