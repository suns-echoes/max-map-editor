import { deepAssignEqual } from './deep-assign-equal.ts';


describe('deepAssignEqual', () => {
	describe('When assigning basic properties', () => {
		it('should add new properties to target and return true', () => {
			// Arrange
			const target = { a: 1 };
			const source = { b: 2 };

			// Act
			const result = deepAssignEqual(target, source);

			// Assert
			assert.deepEqual(target, { a: 1, b: 2 });
			assert.equal(result, true);
		});
	});

	describe('When assigning nested objects', () => {
		it('should deep merge properties and return true', () => {
			// Arrange
			const target = { a: 1, b: { c: 3 } };
			const source = { b: { d: 4 } };

			// Act
			const result = deepAssignEqual(target, source);

			// Assert
			assert.deepEqual(target, { a: 1, b: { c: 3, d: 4 } });
			assert.equal(result, true);
		});
	});

	describe('When target or source is not an object', () => {
		it('should throw error for null target', () => {
			// Arrange & Act & Assert
			// @ts-expect-error
			assert.throws(() => deepAssignEqual(null, {}), /Both target and source must be an object/);
		});

		it('should throw error for null source', () => {
			// Arrange & Act & Assert
			// @ts-expect-error
			assert.throws(() => deepAssignEqual({}, null), /Both target and source must be an object/);
		});

		it('should throw error for primitive target', () => {
			// Arrange & Act & Assert
			// @ts-expect-error
			assert.throws(() => deepAssignEqual(1, {}), /Both target and source must be an object/);
		});

		it('should throw error for primitive source', () => {
			// Arrange & Act & Assert
			// @ts-expect-error
			assert.throws(() => deepAssignEqual({}, 1), /Both target and source must be an object/);
		});
	});

	describe('When source contains arrays', () => {
		it('should replace target array and return true', () => {
			// Arrange
			const target = { a: [1, 2] };
			const source = { a: [3, 4] };

			// Act
			const result = deepAssignEqual(target, source);

			// Assert
			assert.deepEqual(target, { a: [3, 4] });
			assert.equal(result, true);
		});
	});

	describe('When source equals target', () => {
		it('should return false indicating no changes', () => {
			// Arrange
			const target = { a: 1, b: { c: 3 } };
			const source = { a: 1, b: { c: 3 } };

			// Act
			const result = deepAssignEqual(target, source);

			// Assert
			assert.deepEqual(target, { a: 1, b: { c: 3 } });
			assert.equal(result, false);
		});
	});
});
