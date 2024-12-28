export function mat4_createIdentity(): Mat4x4 {
	const out = new Float32Array(16);
	out[0] = 1;
	out[5] = 1;
	out[10] = 1;
	out[15] = 1;
	return out;
}


export function mat4_identity(out: Mat4x4): Mat4x4 {
	out[0] = 1;
	out[1] = 0;
	out[2] = 0;
	out[3] = 0;
	out[4] = 0;
	out[5] = 1;
	out[6] = 0;
	out[7] = 0;
	out[8] = 0;
	out[9] = 0;
	out[10] = 1;
	out[11] = 0;
	out[12] = 0;
	out[13] = 0;
	out[14] = 0;
	out[15] = 1;
	return out;
}

export function mat4_translate(out: Mat4x4, a: Mat4x4, v: Vec3) {
	if (a === out) {
		out[12] = a[0] * v[0] + a[4] * v[1] + a[8] * v[2] + a[12];
		out[13] = a[1] * v[0] + a[5] * v[1] + a[9] * v[2] + a[13];
		out[14] = a[2] * v[0] + a[6] * v[1] + a[10] * v[2] + a[14];
		out[15] = a[3] * v[0] + a[7] * v[1] + a[11] * v[2] + a[15];
	} else {
		out[0] = a[0];
		out[1] = a[1];
		out[2] = a[2];
		out[3] = a[3];
		out[4] = a[4];
		out[5] = a[5];
		out[6] = a[6];
		out[7] = a[7];
		out[8] = a[8];
		out[9] = a[9];
		out[10] = a[10];
		out[11] = a[11];
		out[12] = a[0] * v[0] + a[4] * v[1] + a[8] * v[2] + a[12];
		out[13] = a[1] * v[0] + a[5] * v[1] + a[9] * v[2] + a[13];
		out[14] = a[2] * v[0] + a[6] * v[1] + a[10] * v[2] + a[14];
		out[15] = a[3] * v[0] + a[7] * v[1] + a[11] * v[2] + a[15];
	}

	return out;
}

export function mat4_scale(out: Mat4x4, a: Mat4x4, v: Vec3) {
	out[0] = a[0] * v[0];
	out[1] = a[1] * v[0];
	out[2] = a[2] * v[0];
	out[3] = a[3] * v[0];
	out[4] = a[4] * v[1];
	out[5] = a[5] * v[1];
	out[6] = a[6] * v[1];
	out[7] = a[7] * v[1];
	out[8] = a[8] * v[2];
	out[9] = a[9] * v[2];
	out[10] = a[10] * v[2];
	out[11] = a[11] * v[2];
	if (a !== out) {
		out[12] = a[12];
		out[13] = a[13];
		out[14] = a[14];
		out[15] = a[15];
	}
	return out;
}
