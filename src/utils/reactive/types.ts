export type EffectExecutorFn = () => EffectCleanupFn | void;
export type EffectCleanupFn = () => void;

export type ExprExecutor<T> = (value: T) => T;
