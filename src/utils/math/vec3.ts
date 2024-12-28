export function vec3_create(): Vec3 {
	return new Float32Array([0, 0, 0]);
}

export function vec3_cross(out: Vec3, a: Vec3, b: Vec3): Vec3 {
	out[0] = a[1] * b[2] - a[2] * b[1];
	out[1] = a[2] * b[0] - a[0] * b[2];
	out[2] = a[0] * b[1] - a[1] * b[0];
	return out;
}

export function vec3_dot(a: Vec3, b: Vec3) {
	return a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
}

export function vec3_length(a: Vec3): number {
	return Math.sqrt(a[0] ** 2 + a[1] ** 2 + a[2] ** 2);
}

export function vec3_normalize(out: Vec3, a: Vec3): Vec3 {
	const len = vec3_length(a);
	if (len === 0) {
		out[0] = 0;
		out[1] = 0;
		out[2] = 0;
	} else {
		out[0] = a[0] / len;
		out[1] = a[1] / len;
		out[2] = a[2] / len;
	}
	return out;
}

export function vec3_scale(out: Vec3, a: Vec3, s: Float): Vec3 {
	out[0] = a[0] * s;
	out[1] = a[1] * s;
	out[2] = a[2] * s;
	return out;
}

export function vec3_sub(out: Vec3, a: Vec3, b: Vec3): Vec3 {
	out[0] = a[0] - b[0];
	out[1] = a[1] - b[1];
	out[2] = a[2] - b[2];
	return out;
}
