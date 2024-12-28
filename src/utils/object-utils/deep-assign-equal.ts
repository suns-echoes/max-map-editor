/**
 * Recursively assigns properties from the source object to the target object.
 * If a property in the source object is an object itself, the function will
 * recursively assign its properties to the corresponding property in the target object.
 *
 * @param target - The target object to which properties will be assigned.
 * @param source - The source object from which properties will be copied.
 * @returns A boolean indicating whether any properties were changed in the target object.
 * @throws Will throw an error if either the target or source is not an object.
 */
export function deepAssignEqual(target: Record<string, any>, source: Record<string, any>): boolean {
	if (typeof target !== 'object' || target === null || typeof source !== 'object' || source === null) {
		throw new Error('Both target and source must be an object.');
	}

	return (function assign (target,  source) {
		let isDifferent = false;

		for (const key in source) {
			if (source.hasOwnProperty(key)) {
				if (typeof source[key] === 'object' && source[key] !== null) {
					if (!target[key]) {
						target[key] = Array.isArray(source[key]) ? [] : {};
					}
					isDifferent = deepAssignEqual(target[key], source[key]) || isDifferent;
				} else {
					if (target[key] !== source[key]) {
						target[key] = source[key];
						isDifferent = true;
					}
				}
			}
		}

		return isDifferent;
	})(target, source);
}
