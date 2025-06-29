export function isES2024Supported() {
	return 'withResolvers' in Promise;
}
