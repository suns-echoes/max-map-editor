import { describe as _describe, it as _it } from 'node:test';
import _assert from 'node:assert';

declare global {
	var describe: typeof _describe;
	var it: typeof _it;
	var assert: typeof _assert;
}
