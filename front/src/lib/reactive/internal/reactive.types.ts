export namespace ReactiveD {
	export interface Scope {
		destroy(): void;
		add(source: Object): Scope;
		delete(source: Object): void;
	}

	interface _ScopeableInterface {
		destroy(): void;
		scope(scope: Scope): ThisType<any>;
	}

	interface _SourceInterface {
		destroy(): void;
		dispatch(asyncJobs: Promise<void>[]): ThisType<any>;
		targets: Set<Target>,
	}

	interface _TargetInterface {
		destroy(): void;
		watch(reactiveSources: Source[]): ThisType<any>;
		unwatch(reactiveSource: Source): ThisType<any>;
		notify(asyncJobs: Promise<void>[], debugInfo?: string | false): ThisType<any>;
		sources: Set<Source>,
	}

	export interface Source extends _ScopeableInterface, _SourceInterface {}

	export interface Target extends _ScopeableInterface, _TargetInterface {}

	export interface AsyncTarget extends _ScopeableInterface, _TargetInterface {
		sync(): Promise<void>;
		isAsync: true;
	}

	export interface Middleware extends _ScopeableInterface, _SourceInterface, _TargetInterface {}

	export type Object = Source | AsyncTarget | Target | Middleware;
}
