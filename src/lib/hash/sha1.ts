import { arrayBufferToHexString } from '^lib/convert/array-buffer-to-hex-string.ts';


export async function SHA1(buffer: ArrayBuffer): Promise<string> {
    const hashBuffer = await crypto.subtle.digest('SHA-256', buffer);
    return arrayBufferToHexString(new Uint8Array(hashBuffer));
}
