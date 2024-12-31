import { readFileSync } from 'node:fs';
import { exposedENV } from './vite.exposed.env.ts';

export function viteStyleTransform(code: string, id: string) {
	// TODO: Handle `@import url` statements.
	// TODO: Handle `@import` statements with relative paths.
	// TODO: Handle recursive `@import` statements.

	// Replace `@import` statements with the actual file contents.
	const transformedCode = code.replace(/@import\s+['"]([^'"]+)['"];/g, function (_, path) {
		const filePath = path.startsWith('/') ? '.' + path : path;
		return `/* @import "${path}"; */
${readFileSync(filePath, 'utf8')}
/* @end-import "${path}"; */
`;
	});

	// Replace placeholders with exposed ENV variables.
	code = code.replace(/\{\{env:([a-z0-9_]+)\}\}/gi, function (_, match) {
		return exposedENV[match] ?? `console.error('ENV VAR NOT EXPOSED:', ${match})`;
	});

	return `const style = document.createElement('style');
	style.innerHTML = \`${transformedCode.replaceAll('\`', '\\\`')}\`;
	export default style;
	`;
}
