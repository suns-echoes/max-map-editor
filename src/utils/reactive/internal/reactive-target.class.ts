import type { ReactiveD } from './reactive.types.ts';
import { trace } from './utils.ts';


export abstract class ReactiveTarget implements ReactiveD.Target {
	constructor(type: string, executor: AnyFunction) {
		this._type = type;
		this._executor = executor;
	}

	public destroy(): void {
		this._scope?.delete(this);
		this._scope = null;
		this._executor = null!;
		for (const source of this.sources)
			source.targets.delete(this);
		this.sources.clear();
		this.destroyed = true;
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

	public notify(trace: string | false = false) {
		if (this._trace) console.log(this._debug + '\n\n' + trace);
		this._executor();
		return this;
	}

	public scope(reactiveScope: ReactiveD.Scope) {
		this._scope?.delete(this);
		this._scope = reactiveScope.add(this);
		return this;
	}

	public readonly sources = new Set<ReactiveD.Source>();

	public destroyed = false;

	protected _executor: AnyFunction = null!;

	private _scope: ReactiveD.Scope | null = null;


	// DEBUG
	_type: string = 'ReactiveTarget';
	_debug: string | false = false;
	debug(debugName: string) {
		this._debug = `${this._type}<${debugName}> ${this._stackTrace}`;
		return this;
	}

	_stackTrace = trace();
	_trace: boolean = false;
	trace(enable = true) {
		if (enable === undefined && !!enable)
			this._trace = true;
		return this;
	}
};
