import { RustAPI } from '^utils/rust-api.ts';


export async function writeWRLFile(path: string, data: Uint8Array): Promise<boolean> {
	return RustAPI.writeWRLFile(path, data);
}
