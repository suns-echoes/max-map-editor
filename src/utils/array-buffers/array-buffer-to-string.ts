export function arrayBufferToString(buffer: ArrayBuffer): string {
	return new TextDecoder('utf-8').decode(buffer);
}
