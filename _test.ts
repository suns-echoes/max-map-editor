export const mock = {
	arrowFn() {
		const r = () => undefined;
		r(); // Coverage hack.
		return r;
	},
	asyncArrowFn() {
		const r = async () => undefined;
		r(); // Coverage hack.
		return r;
	},
	fn() {
		const r = function () {};
		r(); // Coverage hack.
		return r;
	},
	asyncFn() {
		const r = async function () {};
		r(); // Coverage hack.
		return r;
	},
	gen() {
		const r = function* () {};
		r(); // Coverage hack.
		return r;
	},
	asyncGen() {
		const r = async function* () {};
		r(); // Coverage hack.
		return r;
	},
};

// Coverage hack.
Object.values(mock).forEach((fn) => fn());
