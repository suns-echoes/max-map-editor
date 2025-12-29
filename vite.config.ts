import path from 'path';
import { defineConfig } from 'vite';

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
	root: 'front',
	clearScreen: false,
	publicDir: 'static',
	build: {
		target: 'es2024',
	},
	resolve: {
		alias: {
			'^actions': path.resolve(__dirname, './front/src/actions'),
			'^consts': path.resolve(__dirname, './front/src/consts'),
			'^components': path.resolve(__dirname, './front/src/components'),
			'^events': path.resolve(__dirname, './front/src/events'),
			'^tauri-apps': path.resolve(__dirname, './front/src/lib/tauri/@tauri-apps'),
			'^tauri': path.resolve(__dirname, './front/src/lib/tauri'),
			'^state': path.resolve(__dirname, './front/src/state'),
			'^types': path.resolve(__dirname, './front/src/types'),
			'^lib': path.resolve(__dirname, './front/src/lib'),
			'^storage': path.resolve(__dirname, './front/src/storage'),
			'^styles': path.resolve(__dirname, './front/src/styles'),
			'^src': path.resolve(__dirname, './front/src'),
		},
	},
	server: {
		port: 1420,
		strictPort: true,
		host: host || false,
		hmr: host
			? {
				protocol: 'ws',
				host,
				port: 1421,
			}
			: undefined,
		watch: {
			ignored: ['crates/**'],
		},
	},
}));
