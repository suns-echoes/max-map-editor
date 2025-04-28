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
		dispatch(): ThisType<any>;
		targets: Set<Target>,
	}

	interface _TargetInterface {
		destroy(): void;
		watch(reactiveSources: Source[]): ThisType<any>;
		unwatch(reactiveSource: Source): ThisType<any>;
		notify(debugInfo?: string | false): ThisType<any>;
		sources: Set<Source>,
	}

	export interface Source extends _ScopeableInterface, _SourceInterface {}

	export interface Target extends _ScopeableInterface, _TargetInterface {}

	export interface Middleware extends _ScopeableInterface, _SourceInterface, _TargetInterface {}

	export type Object = Source | Target | Middleware;
}
