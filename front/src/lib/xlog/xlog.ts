import { RustAPI } from '^src/bff/rust-api.ts';

export class xlog {
	static success(...messages: any[]) {
		console.log(...messages);
		RustAPI.xlog('SUCCESS', prepareMessageString(messages));
	}

	static info(...messages: any[]) {
		console.info(...messages);
		RustAPI.xlog('INFO', prepareMessageString(messages));
	}

	static warn(...messages: any[]) {
		console.warn(...messages);
		RustAPI.xlog('WARN', prepareMessageString(messages));
	}

	static error(...messages: any[]) {
		console.error(...messages);
		RustAPI.xlog('ERROR', prepareMessageString(messages));
	}
}

function prepareMessageString(messages: any[]): string {
	return messages.map(function (message: any) {
		if (typeof message === 'string') {
			return message;
		}

		if (typeof message === 'number') {
			return message.toString(10);
		}

		if (typeof message === undefined) {
			return '[undefined]';
		}

		if (message === null) {
			return 'null';
		}

		if (isObject(message)) {
			if (isError(message)) {
				let errorMessage = message.message;
				if (message.stack) {
					errorMessage + '\n' + message.stack;
				}
				return errorMessage;
			}

			return JSON.stringify(message);
		}

		try {
			return message.toString();
		} catch {
			return message;
		}
	}).join(' ');
}

function isObject(value: any): value is Record<any, any> {
	return typeof value === 'object' && value !== null;
}

function isError(value: any): value is Error {
	return value instanceof Error;
}
