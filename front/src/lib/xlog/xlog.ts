import { RustAPI } from '^src/bff/rust-api.ts';

export class xlog {
	static success(...messages: string[]) {
		const message = messages.join(' ');
		console.log(`%c[SUCCESS] ${message}`, 'color: green;');
		RustAPI.xlog('SUCCESS', message);
	}

	static info(...messages: string[]) {
		const message = messages.join(' ');
		console.info(`%c[INFO] ${message}`, 'color: skyblue;');
		RustAPI.xlog('INFO', message);
	}

	static warn(...messages: string[]) {
		const message = messages.join(' ');
		console.warn(`%c[WARN] ${message}`, 'color: orange;');
		RustAPI.xlog('WARN', message);
	}

	static error(...messages: string[]) {
		const message = messages.join(' ');
		console.error(`%c[ERROR] ${message}`, 'color: red;');
		RustAPI.xlog('ERROR', message);
	}
}
