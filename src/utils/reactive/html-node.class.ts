export class HTMLNode<T extends HTMLElement = HTMLElement, const C extends HTMLNode<HTMLElement>[] = any[]> {
	element: T;

	// Lifecycle methods

	constructor(element: T) {
		this.element = element;
	}

	destroy(): void {
		this.element = null!;
	}

	// DOM tree manipulation methods

	children(offset: number = 0): C {
		const children: HTMLNode[] = [];
		const elements = this.element.children;
		for (let i = offset; i < elements.length; i++)
			children.push(new HTMLNode(elements[i] as HTMLElement));
		return children as any;
	}

	childGroups(groupSize: number = 1, offset: number = 0): HTMLNode[][] {
		const groups: HTMLNode[][] = [];
		const children = this.element.children;
		for (let i = offset; i < children.length; i += groupSize) {
			const group: HTMLNode[] = [];
			// Warning: This may skip last nodes if children.length is not divisible by groupSize
			for (let j = 0; j < groupSize && i + j < children.length; j++)
				group.push(new HTMLNode(children[i + j] as HTMLElement));
			groups.push(group);
		}
		return groups as any;
	}

	get length(): number {
		return this.element.childElementCount;
	}

	nodes<const D extends HTMLNode[]>(nodes: D): HTMLNode<T, D>;
	nodes<const D extends (HTMLNode | HTMLElement | undefined | null | boolean)[]>(nodes: D): HTMLNode<T, ToHTMLNode<D>>;
	nodes(nodes: any): any {
		for (let i = 0; i < nodes.length; i++) {
			const n = nodes[i];
			if (n instanceof HTMLNode) this.element.appendChild(n.element);
			else if (n instanceof HTMLElement) this.element.appendChild(n);
		}
		return this as any;
	}

	trim(count: number): HTMLNode[] {
		const children = this.element.children;
		for (let i = 0; i < count; i++) {
			if (children.length > 0) {
				const child = children[children.length - 1] as HTMLElement;
				HTMLNode._cleanupRegistry.get(child)?.();
				this.element.removeChild(child);
			}
		}
		return this as any;
	}

    append<N extends HTMLNode>(node: N): HTMLNode<T, [...C, N]>;
    append<N extends HTMLElement>(node: N): HTMLNode<T, [...C, HTMLNode<N>]>;
    append(node: any): any {
		this.element.appendChild(node instanceof HTMLNode ? node.element : node);
		return this;
	}

    appendN<N extends HTMLNode[]>(nodes: N): HTMLNode<T, [...C, ...N]>;
    appendN<N extends (HTMLNode | HTMLElement)[]>(nodes: N): HTMLNode<T, ToHTMLNode<N>>;
    appendN(nodes: any): any {
		for (let i = 0; i < nodes.length; i++) {
			const n = nodes[i];
			this.element.appendChild(n instanceof HTMLNode ? n.element : n);
		}
		return this;
	}

	private static _cleanupRegistry: WeakMap<HTMLElement, () => void> = new WeakMap();

	cleanup(cleanupFn: () => void): HTMLNode<T, C> {
		const element = this.element;
		HTMLNode._cleanupRegistry.set(element, function cleanup(): void {
			cleanupFn();
			HTMLNode._cleanupRegistry.delete(element);
		});
		return this;
	}

	remove(node: HTMLNode | HTMLElement): void {
		const child = node instanceof HTMLNode ? node.element : node;
		this.element.removeChild(child);
		HTMLNode._cleanupRegistry.get(child)?.();
	}

	removeN(nodes: (HTMLNode | HTMLElement)[]): void {
		for (let i = 0; i < nodes.length; i++)
			this.remove(nodes[i]);
	}

	clear(): HTMLNode<T, C> {
		const element = this.element;
		while(element.firstChild) element.firstChild.remove();
		return this;
	}

	clone(subtree: boolean): HTMLNode<T, C> {
		const clone = this.element.cloneNode(subtree) as T;
		return new HTMLNode(clone);
	}

	// Event handling methods

	private static _eventRegistry: HTMLNodeEventRegistry = new WeakMap();

	addEventListener<K extends keyof HTMLElementEventMap>(
		type: K,
		listener: (this: T, ev: HTMLElementEventMap[K]) => any,
		options?: boolean | AddEventListenerOptions,
	): HTMLNodeEventRemover {
		const element = this.element;

		function eventRemover(): void {
			element.removeEventListener(type, listener as EventListener, options);

			const eventRemovers = HTMLNode._eventRegistry.get(element);
			if (eventRemovers) {
				eventRemovers.delete(eventRemover);
				if (eventRemovers.size === 0) {
					HTMLNode._eventRegistry.delete(element);
				}
			}
		}

		const eventRemovers = HTMLNode._eventRegistry.get(element);
		if (eventRemovers) eventRemovers.add(eventRemover);
		else HTMLNode._eventRegistry.set(this.element, new Set([eventRemover]));

		this.element.addEventListener(type, listener as EventListener, options);

		return eventRemover;
	}

	removeEventListener(eventRemover: HTMLNodeEventRemover): HTMLNode<T, C> {
		eventRemover();
		return this;
	}

	removeAllEventListeners(): HTMLNode<T, C> {
		const eventRemoversSet = HTMLNode._eventRegistry.get(this.element);
		if (eventRemoversSet) {
			const eventRemovers = [...eventRemoversSet];
			HTMLNode._eventRegistry.get(this.element)?.clear();
			HTMLNode._eventRegistry.delete(this.element);
			for (const eventRemover of eventRemovers) eventRemover();
		}
		return this;
	}

	// Node manipulation methods

	attr(key: string, value: string | number | null | undefined): HTMLNode<T, C> {
		if (value === null || value === undefined)
			this.element.removeAttribute(key);
		else
			this.element.setAttribute(key, typeof value === 'number' ? value.toString(10) : value);
		return this;
	}

	attribs(attribs: Record<string, string | number>): HTMLNode<T, C> {
		for (const [key, value] of Object.entries(attribs))
			if (value === null || value === undefined)
				this.element.removeAttribute(key);
			else
				this.element.setAttribute(key, typeof value === 'number' ? value.toString(10) : value);
		return this;
	}

	class(className: string): HTMLNode<T, C> {
		this.element.className = className;
		return this;
	}

	classes(...classes: string[]): HTMLNode<T, C> {
		this.element.className = '';
		this.element.classList.add(...classes);
		return this;
	}

	data(data: Record<string, string>): HTMLNode<T, C> {
		for (const [key, value] of Object.entries(data)) {
			this.element.dataset[key] = value;
		}
		return this;
	}

	id(id: string): HTMLNode<T, C> {
		this.element.id = id;
		return this;
	}

	props(props: Record<string, string>): HTMLNode<T, C> {
		for (const [key, value] of Object.entries(props))
			(this.element as any)[key] = value;
		return this;
	}

	style(style: Partial<CSSStyleDeclaration>): HTMLNode<T, C> {
		for (const key in style) {
			this.element.style[key] = style[key]!;
		}
		return this;
	}

	src(url: string): HTMLNode<T, C> {
		if (this.element instanceof HTMLImageElement || this.element instanceof HTMLVideoElement)
			this.element.src = url;
		else if (this.element instanceof HTMLLinkElement)
			this.element.href = url;
		else if (this.element instanceof HTMLAudioElement)
			this.element.src = url;
		return this;
	}

	html(html: string): HTMLNode<T, C> {
		this.element.innerHTML = html;
		return this;
	}

	text(text: string | number): HTMLNode<T, C> {
		this.element.textContent = typeof text === 'number' ? text.toString(10) : text;
		return this;
	}

	value(value: string | number): HTMLNode<T, C> {
		if (this.element instanceof HTMLInputElement || this.element instanceof HTMLTextAreaElement)
			this.element.value = typeof value === 'number' ? value.toString(10) : value;
		return this;
	}
}


// =============================================================================

type HTMLNodeEventRemover = () => void;
type HTMLNodeEventRegistry = WeakMap<HTMLElement, Set<HTMLNodeEventRemover>>;

type ToHTMLNode<T extends (HTMLNode | HTMLElement | undefined | null | boolean)[]> = {
	[K in keyof T]: T[K] extends HTMLElement ? HTMLNode<T[K]> : T[K] extends HTMLNode ? T[K] : never;
};
