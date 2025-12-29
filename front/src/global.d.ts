/// <reference types="vite/client" />

import '@suns-echoes/ts-types';

declare module '*.module.css' {
	const classes: { [key: string]: string };
	export default classes;
}

declare module '*?raw' {
	const content: string;
	export default content;
}
