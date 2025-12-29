import { RustAPI } from '^src/bff/rust-api';


export async function MD5(buffer: ArrayBuffer): Promise<string> {
	return RustAPI.hashMD5(new Uint8Array(buffer));
}
