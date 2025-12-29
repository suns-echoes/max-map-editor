export function isTypedArray(value: any): value is ArrayBufferView {
	return ArrayBuffer.isView(value) && !(value instanceof DataView);
}
