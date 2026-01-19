import { hmrDispose, hmrDisposeAll, hmrAccept, hmrEffect, _testing } from './hmr.ts';

const {
	hmrDisposeInternal,
	hmrDisposeAllInternal,
	hmrAcceptInternal,
	hmrEffectInternal,
} = _testing;


// ============================================================================
// Mock Fixtures
// ============================================================================

function mockImportMeta(withHot: boolean = true): { hot?: MockHot } {
	if (!withHot) return {};

	return {
		hot: {
			accept: () => {},
			dispose: () => {},
			data: {},
		},
	};
}

interface MockHot {
	accept: (callback?: (newModule: unknown) => void) => void;
	dispose: (callback: (data: Record<string, unknown>) => void) => void;
	data: Record<string, unknown>;
}


// ============================================================================
// hmrDisposeInternal
// ============================================================================

describe('hmrDisposeInternal', () => {
	describe('When import.meta.hot is available', () => {
		it('should register dispose callback', () => {
			// Arrange
			let disposeCallbackRegistered = false;
			const meta = mockImportMeta();
			meta.hot!.dispose = () => { disposeCallbackRegistered = true; };

			// Act
			hmrDisposeInternal(meta, () => {});

			// Assert
			assert.equal(disposeCallbackRegistered, true);
		});
	});

	describe('When import.meta.hot is not available', () => {
		it('should not throw', () => {
			// Arrange
			const meta = mockImportMeta(false);
			const cleanup = () => {};

			// Act & Assert
			assert.doesNotThrow(() => hmrDisposeInternal(meta, cleanup));
		});
	});
});


// ============================================================================
// hmrDisposeAllInternal
// ============================================================================

describe('hmrDisposeAllInternal', () => {
	describe('When dispose is triggered', () => {
		it('should call dispose on all disposables', () => {
			// Arrange
			const disposed: string[] = [];
			const disposables = [
				{ dispose: () => disposed.push('a') },
				{ dispose: () => disposed.push('b') },
				{ dispose: () => disposed.push('c') },
			];
			let disposeCallback: ((data: Record<string, unknown>) => void) | undefined;
			const meta = mockImportMeta();
			meta.hot!.dispose = (cb) => { disposeCallback = cb; };

			// Act
			hmrDisposeAllInternal(meta, disposables);
			disposeCallback?.({});

			// Assert
			assert.deepEqual(disposed, ['a', 'b', 'c']);
		});
	});
});


// ============================================================================
// hmrAcceptInternal
// ============================================================================

describe('hmrAcceptInternal', () => {
	describe('When import.meta.hot is available', () => {
		it('should call accept', () => {
			// Arrange
			let acceptCalled = false;
			const meta = mockImportMeta();
			meta.hot!.accept = () => { acceptCalled = true; };

			// Act
			hmrAcceptInternal(meta);

			// Assert
			assert.equal(acceptCalled, true);
		});
	});
});


// ============================================================================
// hmrEffectInternal
// ============================================================================

describe('hmrEffectInternal', () => {
	describe('When creating an effect', () => {
		it('should return the created effect', () => {
			// Arrange
			const meta = mockImportMeta();
			const mockEffect = { dispose: () => {} };

			// Act
			const result = hmrEffectInternal(meta, () => mockEffect);

			// Assert
			assert.strictEqual(result, mockEffect);
		});

		it('should store effect in hot.data.effects', () => {
			// Arrange
			const meta = mockImportMeta();
			const mockEffect = { dispose: () => {} };

			// Act
			hmrEffectInternal(meta, () => mockEffect);

			// Assert
			const effects = meta.hot!.data.effects as unknown[];
			assert.ok(Array.isArray(effects));
			assert.equal(effects.length, 1);
			assert.strictEqual(effects[0], mockEffect);
		});
	});

	describe('When module is disposed', () => {
		it('should dispose all registered effects', () => {
			// Arrange
			const disposed: number[] = [];
			const meta = mockImportMeta();
			let disposeCallback: ((data: Record<string, unknown>) => void) | undefined;
			meta.hot!.dispose = (cb) => { disposeCallback = cb; };

			// Act - create multiple effects
			hmrEffectInternal(meta, () => ({ dispose: () => disposed.push(1) }));
			hmrEffectInternal(meta, () => ({ dispose: () => disposed.push(2) }));

			// Simulate HMR dispose with the accumulated data
			disposeCallback?.(meta.hot!.data);

			// Assert
			assert.deepEqual(disposed, [1, 2]);
		});
	});
});


// ============================================================================
// Public API (no-op in test environment)
// ============================================================================

describe('hmrDispose', () => {
	describe('When not in DEV mode', () => {
		it('should not throw and be a no-op', () => {
			// Arrange
			const meta = mockImportMeta();

			// Act & Assert
			assert.doesNotThrow(() => hmrDispose(meta, () => {}));
		});
	});
});


describe('hmrDisposeAll', () => {
	describe('When not in DEV mode', () => {
		it('should not throw and be a no-op', () => {
			// Arrange
			const meta = mockImportMeta();
			const disposables = [{ dispose: () => {} }];

			// Act & Assert
			assert.doesNotThrow(() => hmrDisposeAll(meta, disposables));
		});
	});
});


describe('hmrAccept', () => {
	describe('When not in DEV mode', () => {
		it('should not throw and be a no-op', () => {
			// Arrange
			const meta = mockImportMeta();

			// Act & Assert
			assert.doesNotThrow(() => hmrAccept(meta));
		});
	});
});


describe('hmrEffect', () => {
	describe('When not in DEV mode', () => {
		it('should still create and return the effect', () => {
			// Arrange
			const meta = mockImportMeta();
			const mockEffect = { dispose: () => {} };

			// Act
			const result = hmrEffect(meta, () => mockEffect);

			// Assert
			assert.strictEqual(result, mockEffect);
		});
	});
});
