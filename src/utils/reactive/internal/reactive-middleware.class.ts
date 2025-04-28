import type { ReactiveD } from './reactive.types.ts';


export abstract class ReactiveMiddleware implements ReactiveD.Middleware {
	public constructor(type: string, executor: AnyFunction) {
		this._type = type;
		this._executor = executor;
	}

	public destroy(): void {
		this._scope?.delete(this);
		this._scope = null;
		this._executor = null!;
		for (const target of this.targets)
			target.unwatch(this);
		this.targets.clear();
		for (const source of this.sources)
			source.targets.delete(this);
		this.sources.clear();
		this.destroyed = true;
	}

	public notify(trace: string | false = false) {
		if (this._trace) console.log(this._debug + '\n\n' + trace);
		this._executor();
		this.dispatch();
		return this;
	}

	public dispatch() {
		for (const target of this.targets)
			target.notify(this._debug);
		return this;
	}

	public watch(reactiveSources: ReactiveD.Source[]) {
		for (const reactiveSource of reactiveSources) {
			reactiveSource.targets.add(this);
			this.sources.add(reactiveSource);
		}
		return this;
	}

	public unwatch(reactiveSource: ReactiveD.Source) {
		this.sources.delete(reactiveSource);
		if (this.sources.size === 0)
			this.destroy();
		return this;
	}

	public scope(reactiveScope: ReactiveD.Scope) {
		this._scope?.delete(this);
		this._scope = reactiveScope.add(this);
		return this;
	}

	public readonly targets = new Set<ReactiveD.Target>();

	public readonly sources = new Set<ReactiveD.Source>();

	public destroyed = false;

	protected _executor: AnyFunction = null!;

	private _scope: ReactiveD.Scope | null = null;


	// DEBUG
	_type: string = 'ReactiveMiddleware';
	_debug: string | false = false;
	debug(debugName: string) {
		this._debug = `${this._type}<${debugName}> ${this._stackTrace}`;
		return this;
	}

	_stackTrace = this.trace();
	_trace: boolean = false;
	trace(enable = true) {
		if (enable === undefined && !!enable)
			this._trace = true;
		return this;
	}
};
