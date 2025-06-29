export function arrayBufferToHexString(arrayBuffer: Uint8Array): string {
	return Array.from(arrayBuffer).map(byte => byte.toString(16).padStart(2, '0')).join('');
}
