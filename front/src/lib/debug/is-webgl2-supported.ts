export function isWebGL2Supported() {
	const canvas = document.createElement('canvas') as HTMLCanvasElement;
	const gl = canvas.getContext('webgl2');
	return !!gl;
}
