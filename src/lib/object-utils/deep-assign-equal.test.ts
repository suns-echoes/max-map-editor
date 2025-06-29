import { deepAssignEqual } from './deep-assign-equal.ts';


describe('deepAssignEqual', () => {
	it('should assign basic properties from source to target', () => {
		const target = { a: 1 };
		const source = { b: 2 };
		const result = deepAssignEqual(target, source);
		assert.deepEqual(target, { a: 1, b: 2 });
		assert.equal(result, true);
	});

	it('should assign nested object properties from source to target', () => {
		const target = { a: 1, b: { c: 3 } };
		const source = { b: { d: 4 } };
		const result = deepAssignEqual(target, source);
		assert.deepEqual(target, { a: 1, b: { c: 3, d: 4 } });
		assert.equal(result, true);
	});

	it('should throw an error if target or source is not an object', () => {
		// @ts-expect-error
		assert.throws(() => deepAssignEqual(null, {}), /Both target and source must be an object/);
		// @ts-expect-error
		assert.throws(() => deepAssignEqual({}, null), /Both target and source must be an object/);
		// @ts-expect-error
		assert.throws(() => deepAssignEqual(1, {}), /Both target and source must be an object/);
		// @ts-expect-error
		assert.throws(() => deepAssignEqual({}, 1), /Both target and source must be an object/);
	});

	it('should handle arrays within objects', () => {
		const target = { a: [1, 2] };
		const source = { a: [3, 4] };
		const result = deepAssignEqual(target, source);
		assert.deepEqual(target, { a: [3, 4] });
		assert.equal(result, true);
	});

	it('should return false if no properties were changed', () => {
		const target = { a: 1, b: { c: 3 } };
		const source = { a: 1, b: { c: 3 } };
		const result = deepAssignEqual(target, source);
		assert.deepEqual(target, { a: 1, b: { c: 3 } });
		assert.equal(result, false);
	});
});
