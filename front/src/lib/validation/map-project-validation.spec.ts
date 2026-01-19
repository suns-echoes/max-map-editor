import {
	parseMapProject,
	validateMapProject,
	MapProjectValidationError,
} from './map-project-validation.ts';


// ============================================================================
// Test Fixtures
// ============================================================================

function validMapProject(): object {
	return {
		version: 1,
		name: 'Test Map',
		description: 'A test map',
		width: 2,
		height: 2,
		use: [
			{ name: 'WATER', tileset: true, version: 1 },
			{ name: 'GREEN', tileset: true, palette: true, version: 1 },
		],
		map: [
			['WATR01', 'WATR02'],
			[['WATR01', 'GSb000'], 'WATR03'],
		],
	};
}


// ============================================================================
// parseMapProject
// ============================================================================

describe('parseMapProject', () => {
	describe('When given valid JSON', () => {
		it('should return parsed MapProject', () => {
			// Arrange
			const json = JSON.stringify(validMapProject());

			// Act
			const result = parseMapProject(json);

			// Assert
			assert.equal(result.name, 'Test Map');
			assert.equal(result.width, 2);
		});
	});

	describe('When given invalid JSON', () => {
		it('should throw with Invalid JSON message', () => {
			// Arrange
			const invalidJson = 'not valid json';

			// Act & Assert
			assert.throws(
				() => parseMapProject(invalidJson),
				/Invalid JSON/
			);
		});
	});

	describe('When given empty string', () => {
		it('should throw with Invalid JSON message', () => {
			// Arrange
			const emptyJson = '';

			// Act & Assert
			assert.throws(
				() => parseMapProject(emptyJson),
				/Invalid JSON/
			);
		});
	});
});


// ============================================================================
// validateMapProject - Root
// ============================================================================

describe('validateMapProject', () => {
	describe('When data is null', () => {
		it('should throw root validation error', () => {
			// Arrange
			const data = null;

			// Act & Assert
			assert.throws(
				() => validateMapProject(data),
				/root.*Expected an object/
			);
		});
	});

	describe('When data is not an object', () => {
		it('should throw root validation error', () => {
			// Arrange
			const data = 'string';

			// Act & Assert
			assert.throws(
				() => validateMapProject(data),
				/root.*Expected an object/
			);
		});
	});
});


// ============================================================================
// validateMapProject - Version
// ============================================================================

describe('validateMapProject version', () => {
	describe('When version is 1', () => {
		it('should not throw', () => {
			// Arrange
			const data = validMapProject();

			// Act & Assert
			assert.doesNotThrow(() => validateMapProject(data));
		});
	});

	describe('When version is missing', () => {
		it('should throw version validation error', () => {
			// Arrange
			const data = validMapProject() as Record<string, unknown>;
			delete data.version;

			// Act & Assert
			assert.throws(
				() => validateMapProject(data),
				/version.*Expected 1/
			);
		});
	});

	describe('When version is wrong value', () => {
		it('should throw version validation error with actual value', () => {
			// Arrange
			const data = { ...validMapProject(), version: 0.2 };

			// Act & Assert
			assert.throws(
				() => validateMapProject(data),
				/version.*Expected 1, got 0.2/
			);
		});
	});
});


// ============================================================================
// validateMapProject - Name
// ============================================================================

describe('validateMapProject name', () => {
	describe('When name is missing', () => {
		it('should throw name validation error', () => {
			// Arrange
			const data = validMapProject() as Record<string, unknown>;
			delete data.name;

			// Act & Assert
			assert.throws(
				() => validateMapProject(data),
				/name.*Expected non-empty string/
			);
		});
	});

	describe('When name is empty string', () => {
		it('should throw name validation error', () => {
			// Arrange
			const data = { ...validMapProject(), name: '' };

			// Act & Assert
			assert.throws(
				() => validateMapProject(data),
				/name.*Expected non-empty string/
			);
		});
	});
});


// ============================================================================
// validateMapProject - Description
// ============================================================================

describe('validateMapProject description', () => {
	describe('When description is empty string', () => {
		it('should not throw', () => {
			// Arrange
			const data = { ...validMapProject(), description: '' };

			// Act & Assert
			assert.doesNotThrow(() => validateMapProject(data));
		});
	});

	describe('When description is not a string', () => {
		it('should throw description validation error', () => {
			// Arrange
			const data = { ...validMapProject(), description: 123 };

			// Act & Assert
			assert.throws(
				() => validateMapProject(data),
				/description.*Expected string/
			);
		});
	});
});


// ============================================================================
// validateMapProject - Dimensions
// ============================================================================

describe('validateMapProject dimensions', () => {
	describe('When width is zero', () => {
		it('should throw width validation error', () => {
			// Arrange
			const data = { ...validMapProject(), width: 0 };

			// Act & Assert
			assert.throws(
				() => validateMapProject(data),
				/width.*Expected positive integer/
			);
		});
	});

	describe('When width is negative', () => {
		it('should throw width validation error', () => {
			// Arrange
			const data = { ...validMapProject(), width: -1 };

			// Act & Assert
			assert.throws(
				() => validateMapProject(data),
				/width.*Expected positive integer/
			);
		});
	});

	describe('When width is decimal', () => {
		it('should throw width validation error', () => {
			// Arrange
			const data = { ...validMapProject(), width: 1.5 };

			// Act & Assert
			assert.throws(
				() => validateMapProject(data),
				/width.*Expected positive integer/
			);
		});
	});

	describe('When height is zero', () => {
		it('should throw height validation error', () => {
			// Arrange
			const data = { ...validMapProject(), height: 0 };

			// Act & Assert
			assert.throws(
				() => validateMapProject(data),
				/height.*Expected positive integer/
			);
		});
	});
});


// ============================================================================
// validateMapProject - Use Array
// ============================================================================

describe('validateMapProject use array', () => {
	describe('When use is not an array', () => {
		it('should throw use validation error', () => {
			// Arrange
			const data = { ...validMapProject(), use: {} };

			// Act & Assert
			assert.throws(
				() => validateMapProject(data),
				/use.*Expected array/
			);
		});
	});

	describe('When use is empty array', () => {
		it('should throw use validation error', () => {
			// Arrange
			const data = { ...validMapProject(), use: [] };

			// Act & Assert
			assert.throws(
				() => validateMapProject(data),
				/use.*Expected at least one asset/
			);
		});
	});

	describe('When use item has no name', () => {
		it('should throw use[0].name validation error', () => {
			// Arrange
			const data = { ...validMapProject(), use: [{ version: 1 }] };

			// Act & Assert
			assert.throws(
				() => validateMapProject(data),
				/use\[0\]\.name.*Expected non-empty string/
			);
		});
	});

	describe('When use item has no version', () => {
		it('should throw use[0].version validation error', () => {
			// Arrange
			const data = { ...validMapProject(), use: [{ name: 'TEST' }] };

			// Act & Assert
			assert.throws(
				() => validateMapProject(data),
				/use\[0\]\.version.*Expected number/
			);
		});
	});

	describe('When use item tileset is not boolean', () => {
		it('should throw use[0].tileset validation error', () => {
			// Arrange
			const data = { ...validMapProject(), use: [{ name: 'TEST', version: 1, tileset: 'yes' }] };

			// Act & Assert
			assert.throws(
				() => validateMapProject(data),
				/use\[0\]\.tileset.*Expected boolean/
			);
		});
	});
});


// ============================================================================
// validateMapProject - Map Array
// ============================================================================

describe('validateMapProject map array', () => {
	describe('When map row count does not match height', () => {
		it('should throw map validation error with counts', () => {
			// Arrange
			const data = { ...validMapProject(), map: [['A']] };

			// Act & Assert
			assert.throws(
				() => validateMapProject(data),
				/map.*Expected 2 rows, got 1/
			);
		});
	});

	describe('When map column count does not match width', () => {
		it('should throw map[0] validation error with counts', () => {
			// Arrange
			const data = { ...validMapProject(), map: [['A'], ['B']] };

			// Act & Assert
			assert.throws(
				() => validateMapProject(data),
				/map\[0\].*Expected 2 cells, got 1/
			);
		});
	});

	describe('When cell is neither string nor array', () => {
		it('should throw map[y][x] validation error', () => {
			// Arrange
			const data = { ...validMapProject(), map: [[123, 'B'], ['C', 'D']] };

			// Act & Assert
			assert.throws(
				() => validateMapProject(data),
				/map\[0\]\[0\].*Expected string or string\[\]/
			);
		});
	});

	describe('When layered cell contains non-string', () => {
		it('should throw map[y][x][layer] validation error', () => {
			// Arrange
			const data = { ...validMapProject(), map: [[['A', 123], 'B'], ['C', 'D']] };

			// Act & Assert
			assert.throws(
				() => validateMapProject(data),
				/map\[0\]\[0\]\[1\].*Expected string, got number/
			);
		});
	});

	describe('When map contains valid layered cells', () => {
		it('should not throw', () => {
			// Arrange
			const data = validMapProject();

			// Act & Assert
			assert.doesNotThrow(() => validateMapProject(data));
		});
	});
});


// ============================================================================
// MapProjectValidationError
// ============================================================================

describe('MapProjectValidationError', () => {
	describe('When constructed', () => {
		it('should have name MapProjectValidationError', () => {
			// Arrange & Act
			const error = new MapProjectValidationError('field', 'message');

			// Assert
			assert.equal(error.name, 'MapProjectValidationError');
		});

		it('should format message with field path', () => {
			// Arrange & Act
			const error = new MapProjectValidationError('use[0].name', 'Expected non-empty string');

			// Assert
			assert.match(error.message, /use\[0\]\.name/);
			assert.match(error.message, /Expected non-empty string/);
		});

		it('should be an instance of Error', () => {
			// Arrange & Act
			const error = new MapProjectValidationError('field', 'message');

			// Assert
			assert.ok(error instanceof Error);
		});
	});
});
