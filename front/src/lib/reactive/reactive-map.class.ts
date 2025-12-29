import { ReactiveSource } from './internal/reactive-source.class.ts';


export class ReactiveMap<K, V> extends ReactiveSource {
	value = new Map<K, V>();

	constructor() {
		super('ReactiveMap');
	}

	get(key: K): V | undefined {
		return this.value.get(key);
	}

	set(key: K, value: V): void {
		this.value.set(key, value);
		this.dispatch();
	}

	delete(key: K): void {
		this.value.delete(key);
		this.dispatch();
	}

	has(key: K): boolean {
		return this.value.has(key);
	}

	clear(): void {
		this.value.clear();
		this.dispatch();
	}

	entries(): IterableIterator<[K, V]> {
		return this.value.entries();
	}

	keys(): IterableIterator<K> {
		return this.value.keys();
	}

	values(): IterableIterator<V> {
		return this.value.values();
	}

	forEach(callback: (value: V, key: K) => void): void {
		this.value.forEach(callback);
	}

	[Symbol.iterator](): IterableIterator<[K, V]> {
		return this.value.entries();
	}

	destroy(): void {
		this.value.clear();
		super.destroy();
	}
}
