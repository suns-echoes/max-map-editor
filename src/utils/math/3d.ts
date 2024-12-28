import { vec3_create, vec3_cross, vec3_normalize, vec3_sub } from './vec3.ts';


export function perspective(out: Mat4x4, fovy: Radians, aspect: Float, near: Float, far: Float): Mat4x4 {
	const f = 1 / Math.tan(fovy / 2);
	const nf = 1 / (near - far);

	out[0] = f / aspect;
	out[1] = 0;
	out[2] = 0;
	out[3] = 0;
	out[4] = 0;
	out[5] = f;
	out[6] = 0;
	out[7] = 0;
	out[8] = 0;
	out[9] = 0;
	out[10] = (far + near) * nf;
	out[11] = -1;
	out[12] = 0;
	out[13] = 0;
	out[14] = 2 * far * near * nf;
	out[15] = 0;

	return out;
}

export function lookAt(out: Mat4x4, eye: Vec3, center: Vec3, up: Vec3): Mat4x4 {
	const z = vec3_normalize(vec3_create(), vec3_sub(vec3_create(), eye, center));
	const x = vec3_normalize(vec3_create(), vec3_cross(vec3_create(), up, z));
	const y = vec3_normalize(vec3_create(), vec3_cross(vec3_create(), z, x));

	out[0] = x[0];
	out[1] = y[0];
	out[2] = z[0];
	out[3] = 0;
	out[4] = x[1];
	out[5] = y[1];
	out[6] = z[1];
	out[7] = 0;
	out[8] = x[2];
	out[9] = y[2];
	out[10] = z[2];
	out[11] = 0;
	out[12] = -(x[0] * eye[0] + x[1] * eye[1] + x[2] * eye[2]);
	out[13] = -(y[0] * eye[0] + y[1] * eye[1] + y[2] * eye[2]);
	out[14] = -(z[0] * eye[0] + z[1] * eye[1] + z[2] * eye[2]);
	out[15] = 1;

	return out;
}
