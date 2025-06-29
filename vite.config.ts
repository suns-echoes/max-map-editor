import { readFileSync } from 'node:fs';
import path, { dirname, join } from 'node:path';

import { defineConfig, Plugin, UserConfig, ViteDevServer } from 'vite';

import { exposedENV } from './vite.exposed.env.ts';


const vertexShaderProtocol = 'vs:';
const fragmentShaderProtocol = 'fs:';


export default defineConfig(async (config: UserConfig) => ({
	root: '.',
	publicDir: 'src/static',

	define: {
		'__ENV__': JSON.stringify(exposedENV),
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
			'^consts': path.resolve(__dirname, './src/consts'),
			'^components': path.resolve(__dirname, './src/components'),
			'^events': path.resolve(__dirname, './src/events'),
			'^tauri-apps': path.resolve(__dirname, './src/lib/tauri/@tauri-apps'),
			'^tauri': path.resolve(__dirname, './src/lib/tauri'),
			'^state': path.resolve(__dirname, './src/state'),
			'^types': path.resolve(__dirname, './src/types'),
			'^lib': path.resolve(__dirname, './src/lib'),
			'^storage': path.resolve(__dirname, './src/storage'),
			'^styles': path.resolve(__dirname, './src/styles'),
			'^src': path.resolve(__dirname, './src'),
		},
	},

	plugins: [
		{
			/**
			 * The role of this plugin is to provide support
			 * for resolving resource files in development mode
			 * when the Tauri API is not available.
			 */
			name: 'resolve-resource',

			configureServer(server: ViteDevServer) {
				server.middlewares.use((req, res, next) => {
					const url = req.url ?? '';
					if (!url.startsWith('/resolve-resource/')) {
						return next();
					}
					const path = url.substring('resolve-resource/'.length);
					const fileData = readFileSync('.' + path, 'utf8');
					res.setHeader('Access-Control-Allow-Origin', '*');
					res.writeHead(200, { 'Content-Type': 'text/plain' });
					res.write(fileData);
					res.end();
				})
			}
		},
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

				if (source.endsWith('.vs')) {
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

				if (id.startsWith(vertexShaderProtocol) || id.startsWith(fragmentShaderProtocol)) {
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

				if (id.endsWith('.vs') || id.endsWith('.fs')) {
					return `export default \`${code.replaceAll('\`', '\\\`')}\`;`;
				}

				return code;
			},
		} as Plugin,
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
