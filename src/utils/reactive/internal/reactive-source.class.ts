import type { ReactiveD } from './reactive.types.ts';
import { Reactive } from '../reactive.class.ts';
import { trace } from './utils.ts';


export abstract class ReactiveSource implements ReactiveD.Source {
	constructor(type: string) {
		this._type = type;
	}

	public destroy(): void {
		this._scope?.delete(this);
		this._scope = null;
		for (const target of this.targets)
			target.unwatch(this);
		this.targets.clear();
		this.destroyed = true;
	}

	public dispatch() {
		this._asyncJobs.length = 0;
		for (const target of this.targets) {
			target.notify(this._asyncJobs, this._debug);
		}
		return this;
	}

	public async sync(): Promise<void> {
		if (this._asyncJobs.length)
			await Promise.all(this._asyncJobs);
		return Reactive.sync();
	}

	public scope(reactiveScope: ReactiveD.Scope) {
		this._scope?.delete(this);
		this._scope = reactiveScope.add(this);
		return this;
	}

	public readonly targets = new Set<ReactiveD.Target>();

	public destroyed = false;

	private _scope: ReactiveD.Scope | null = null;

	private _asyncJobs: Promise<void>[] = [];


	// DEBUG
	_type: string = 'ReactiveSource';
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
}
