import { RustAPI } from '^lib/rust-api.ts';

export async function fileExists(path: string): Promise<boolean> {
	return RustAPI.fileExists(path);
}
