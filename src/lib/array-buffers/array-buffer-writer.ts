export class ArrayBufferWriter {
	constructor(buffer: ArrayBuffer) {
		this.#dataView = new DataView(buffer);
		this.#uint8 = new Uint8Array(buffer);
	}

	seek(offset: number) {
		this.#offset = offset;
	}

	writeBytes(source: ArrayBuffer, length: number = source.byteLength) {
		this.writeUint8Array(new Uint8Array(source), length);
	}

	writeUint8Array(source: Uint8Array, length: number = source.byteLength) {
		// TODO: Add overflow prevention!
		this.#uint8.set(source, this.#offset);
		this.#offset += length;
	}

	writeUint16Array(source: Uint16Array, length: number = source.length) {
		this.writeUint8Array(new Uint8Array(source.buffer), length * 2);
	}

	writeUInt16LE(value: number) {
		this.#dataView.setUint16(this.#offset, value, true);
		this.#offset += 2;
	}

	#offset: number = 0;
	#dataView: DataView = null!;
	#uint8: Uint8Array = null!;

}
