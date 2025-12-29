import { RustAPI } from '^src/bff/rust-api';


export async function writeWRLFile(path: string, data: Uint8Array): Promise<boolean> {
	return RustAPI.writeWRLFile(path, data);
}
