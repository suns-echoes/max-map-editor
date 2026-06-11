// Tile Explorer grid pass + toolbox active-tile preview
//: screen-space quads sampling the project atlas → palette exactly
// like project.wgsl's atlas_pixel. `transform` carries map-core's bits
// (rot 0–1, mirror bit 2; the grid passes 0 = base art). Index-0 pixels
// render as a dim translucent fill so transparency reads.

struct VsIn {
	@location(0) pos: vec2<f32>,   // clip space
	@location(1) uv: vec2<f32>,    // 0..1 within the tile
	@location(2) index: u32,       // global atlas tile index
	@location(3) transform: u32,   // map-core Transform::bits
	@location(4) alpha: f32,       // whole-quad opacity (ghost previews <1)
};

struct VsOut {
	@builtin(position) pos: vec4<f32>,
	@location(0) uv: vec2<f32>,
	@location(1) @interpolate(flat) index: u32,
	@location(2) @interpolate(flat) transform: u32,
	@location(3) @interpolate(flat) alpha: f32,
};

@group(0) @binding(0) var atlas:   texture_2d_array<u32>;  // R8Uint
@group(0) @binding(1) var palette: texture_2d<f32>;        // Rgba8UnormSrgb 256×1

@vertex
fn vs_main(in: VsIn) -> VsOut {
	var out: VsOut;
	out.pos = vec4<f32>(in.pos, 0.0, 1.0);
	out.uv = in.uv;
	out.index = in.index;
	out.transform = in.transform;
	out.alpha = in.alpha;
	return out;
}

// Mirrors project.wgsl / map-core transform_into (undo rotation ccw, then
// un-mirror x).
fn transformed_sub(sub: vec2<u32>, bits: u32) -> vec2<u32> {
	var sx = sub.x;
	var sy = sub.y;
	let rot = bits & 3u;
	for (var r = 0u; r < rot; r += 1u) {
		let t = sy;
		sy = 63u - sx;
		sx = t;
	}
	if ((bits & 4u) != 0u) {
		sx = 63u - sx;
	}
	return vec2<u32>(sx, sy);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
	let raw = vec2<u32>(clamp(in.uv, vec2<f32>(0.0), vec2<f32>(0.99999)) * 64.0);
	let sub = transformed_sub(raw, in.transform);
	let layer = in.index >> 8u;
	let slot = in.index & 255u;
	let origin = vec2<u32>((slot % 16u) * 64u, (slot / 16u) * 64u);
	let pixel = textureLoad(atlas, vec2<i32>(origin + sub), i32(layer), 0).r;
	if (pixel == 0u) {
		return vec4<f32>(0.0, 0.0, 0.0, 0.35 * in.alpha);
	}
	let c = textureLoad(palette, vec2<i32>(i32(pixel), 0), 0);
	return vec4<f32>(c.rgb, c.a * in.alpha);
}
