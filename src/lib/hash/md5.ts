import { RustAPI } from '^lib/rust-api.ts';


export async function MD5(buffer: ArrayBuffer): Promise<string> {
	return RustAPI.hashMD5(new Uint8Array(buffer));
}
