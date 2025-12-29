import { exposedENV } from '../../vite.exposed.env.ts';


declare global {
	var __ENV__: typeof exposedENV;
}
