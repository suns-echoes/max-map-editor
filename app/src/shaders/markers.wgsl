// Resource-marker quads (see markers.rs / markers_render.rs): screen-space
// quads sampling the marker sprite atlas (R8Uint, palette indices) through the
// working palette — so palette edits and colour cycling recolour the markers
// live, matching the map. No team remap (markers are neutral) and no shadow.

struct VsIn {
	@location(0) pos: vec2<f32>,     // clip space
	@location(1) uv: vec2<f32>,      // sprite-local pixels (0..w, 0..h)
	@location(2) origin: vec2<u32>,  // sprite's pixel origin in the atlas
};

struct VsOut {
	@builtin(position) pos: vec4<f32>,
	@location(0) uv: vec2<f32>,
	@location(1) @interpolate(flat) origin: vec2<u32>,
};

@group(0) @binding(0) var atlas:   texture_2d<u32>;  // R8Uint
@group(0) @binding(1) var palette: texture_2d<f32>;  // Rgba8UnormSrgb 256×1

@vertex
fn vs_main(in: VsIn) -> VsOut {
	var out: VsOut;
	out.pos = vec4<f32>(in.pos, 0.0, 1.0);
	out.uv = in.uv;
	out.origin = in.origin;
	return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
	let px = in.origin + vec2<u32>(vec2<i32>(floor(in.uv)));
	let idx = textureLoad(atlas, vec2<i32>(px), 0).r;
	if (idx == 0u) {
		discard;
	}
	let color = textureLoad(palette, vec2<i32>(i32(idx), 0), 0).rgb;
	return vec4<f32>(color, 1.0);
}
