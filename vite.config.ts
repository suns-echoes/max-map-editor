import { readFileSync } from 'node:fs';
import path, { dirname, join } from 'node:path';

import { defineConfig, Plugin, UserConfig } from 'vite';

import { exposedENV } from './vite.exposed.env.ts';


const vertexShaderProtocol = 'vs:';
const fragmentShaderProtocol = 'fs:';
const htmlProtocol = 'html:';
const styleProtocol = 'style:';


export default defineConfig(async (config: UserConfig) => ({
	root: '.',
	publicDir: 'src/static',

	define: {
		'__ENV__': exposedENV,
	},

	build: {
		cssMinify: false,
		minify: false,
		modulePreload: false,
		outDir: 'dist',
		target: 'es2022',

		rollupOptions: {
			cache: false,
			watch: {
				include: ['./src/**/*.ts', './src/**/*.html', './src/**/*.css'],
			},
		},
	},

	clearScreen: false,

	resolve: {
		alias: {
			'^actions': path.resolve(__dirname, './src/actions'),
			'^components': path.resolve(__dirname, './src/components'),
			'^events': path.resolve(__dirname, './src/events'),
			'^types': path.resolve(__dirname, './src/types'),
			'^utils': path.resolve(__dirname, './src/utils'),
			'^storage': path.resolve(__dirname, './src/storage'),
			'^styles': path.resolve(__dirname, './src/styles'),
			'^src': path.resolve(__dirname, './src'),
		},
	},

	plugins: [
		{
			/**
			 * The role of this plugin is to provide support
			 * for importing web component HTML templates
			 * and their styles from CSS files.
			 */

			name: 'web-component-templates',
			enforce: 'pre',

			/**
			 * Development build transformation.
			 */
			resolveId(source, importer) {
				if (config.mode !== 'production') {
					return null;
				}

				// TODO: Find better method for distinguishing web components templates.
				if (source.endsWith('.html') && !source.endsWith('index.html')) {
					const filePath = join(importer ? dirname(importer) : '/', source);

					return {
						id: `${htmlProtocol}${filePath}.js`,
					};
				}

				else if (source.endsWith('.style')) {
					const filePath = join(importer ? dirname(importer) : '/', source);

					return {
						id: `${styleProtocol}${filePath}.js`,
					};
				}

				else if (source.endsWith('.vs')) {
					const filePath = join(importer ? dirname(importer) : '/', source);

					return {
						id: `${vertexShaderProtocol}${filePath}.js`,
					};
				}

				else if (source.endsWith('.fs')) {
					const filePath = join(importer ? dirname(importer) : '/', source);

					return {
						id: `${fragmentShaderProtocol}${filePath}.js`,
					};
				}

				return null;
			},

			load(id) {
				if (config.mode !== 'production') {
					return null;
				}

				if (id.startsWith(htmlProtocol)) {
					const filePath = id.substring(htmlProtocol.length, id.length - 3);
					let code = readFileSync(filePath, 'utf8');

					// Replace placeholders with exposed ENV variables.
					code = code.replace(/\{\{env:([a-z0-9_]+)\}\}/gi, function (_, match) {
						return exposedENV[match] ?? `console.error('ENV VAR NOT EXPOSED:', ${match})`;
					});

					code = `const template = document.createElement('template');
					template.innerHTML = \`${code.replaceAll('\`', '\\\`')}\`;
					export default template;`;

					return {
						code,
					};
				}

				else if (id.startsWith(styleProtocol)) {
					const filePath = id.substring(styleProtocol.length, id.length - 3);
					let code = readFileSync(filePath, 'utf8');

					// Replace placeholders with exposed ENV variables.
					code = code.replace(/\{\{env:([a-z0-9_]+)\}\}/gi, function (_, match) {
						return exposedENV[match] ?? `console.error('ENV VAR NOT EXPOSED:', ${match})`;
					});

					code = `const style = document.createElement('style');
					style.innerHTML = \`${code.replaceAll('\`', '\\\`')}\`;
					export default style;`;

					return {
						code,
					};
				}

				else if (id.startsWith(vertexShaderProtocol) || id.startsWith(fragmentShaderProtocol)) {
					const filePath = id.substring(vertexShaderProtocol.length, id.length - 3);
					let code = readFileSync(filePath, 'utf8');

					return {
						code: `export default \`${code.replaceAll('\`', '\\\`')}\`;`,
					};
				}

				return null;
			},

			/**
			 * Production build transformation.
			 */
			transform(code, id) {
				if (config.mode === 'production') {
					return code;
				}

				// Replace placeholders with exposed ENV variables.
				code = code.replace(/\{\{env:([a-z0-9_]+)\}\}/gi, function (_, match) {
					return exposedENV[match] ?? `<script>console.error('ENV VAR NOT EXPOSED:', ${match})</script>`;
				});

				if (id.endsWith('.html')) {
					return `const template = document.createElement('template');
				 		template.innerHTML = \`${code.replaceAll('\`', '\\\`')}\`;
				 		export default template;`;
				}

				else if (id.endsWith('.style')) {
					return `const style = document.createElement('style');
				 		style.innerHTML = \`${code.replaceAll('\`', '\\\`')}\`;
				 		export default style;`;
				}

				else if (id.endsWith('.vs') || id.endsWith('.fs')) {
					return `export default \`${code.replaceAll('\`', '\\\`')}\`;`;
				}

				return code;
			},
		} as Plugin,

		// {
		// 	name: 'inject-env',
		// 	enforce: 'post',

		// 	transformIndexHtml(html) {
		// 		// Replace placeholders with exposed ENV variables.
		// 		return html.replace(/\{\{env:([a-z0-9_]+)\}\}/gi, function (_, match) {
		// 			return exposedENV[match] ?? `<script>console.error('ENV VAR NOT EXPOSED:', ${match})</script>`;
		// 		});
		// 	},
		// } as Plugin,
	],

	server: {
		// hmr: false,
		port: 1420,
		strictPort: true,
		watch: {
			include: ['./src/**/*.ts', './src/**/*.html', './src/**/*.css', './src/**/*.vs', './src/**/*.fs'],
			ignored: ['**/src-tauri/**'],
		},
	},
}));
