import { isTypedArray } from './is-typed-array.ts';


export class ArrayBufferReader {
	constructor(buffer: ArrayBufferLike | ArrayBufferView) {
		this.#buffer = isTypedArray(buffer) ? buffer.buffer : buffer;
		this.#dataView = new DataView(this.#buffer);
	}

	seek(offset: number) {
		this.#offset = offset;
	}

	readBytes(length: number = this.#buffer.byteLength - this.#offset): ArrayBufferLike {
		const bytes = this.#buffer.slice(this.#offset, this.#offset + length);
		this.#offset += length;
		return bytes;
	}

	readUint8Array(length = this.#buffer.byteLength - this.#offset): Uint8Array {
		return new Uint8Array(this.readBytes(length));
	}

	readUint16Array(length: number = this.#buffer.byteLength - this.#offset): Uint16Array {
		return new Uint16Array(this.readBytes(length * 2));
	}

	readUInt16LE() {
		const value = this.#dataView.getUint16(this.#offset, true);
		this.#offset += 2;
		return value;
	}

	#offset: number = 0;
	#buffer: ArrayBufferLike = null!;
	#dataView: DataView = null!;
}
