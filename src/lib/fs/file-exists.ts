import { RustAPI } from '^src/bff/rust-api';

export async function fileExists(path: string): Promise<boolean> {
	return RustAPI.fileExists(path);
}
