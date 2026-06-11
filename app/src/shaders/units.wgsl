// Unit-preview quads (see units.rs / units_render.rs): screen-space quads
// sampling the unit sprite atlas (R8Uint, palette indices) through the
// working palette — so palette edits and color cycling recolor units live.
//
// Team colors: FRAMEPIC's three 8-entry team ramps (32..=39 green art,
// 48..=55 blue art, 56..=63 red art) all remap to the active team's ramp
// via `idx & 7`, mirroring the original game's per-player palette rows.
// Shadow quads ignore the palette: any non-zero index is translucent black.

struct VsIn {
	@location(0) pos: vec2<f32>,     // clip space
	@location(1) uv: vec2<f32>,      // sprite-local pixels (0..w, 0..h)
	@location(2) origin: vec2<u32>,  // sprite's pixel origin in the atlas
	@location(3) flags: u32,         // bits 0..3 team, bit 3 shadow
};

struct VsOut {
	@builtin(position) pos: vec4<f32>,
	@location(0) uv: vec2<f32>,
	@location(1) @interpolate(flat) origin: vec2<u32>,
	@location(2) @interpolate(flat) flags: u32,
};

@group(0) @binding(0) var atlas:   texture_2d<u32>;  // R8Uint
@group(0) @binding(1) var palette: texture_2d<f32>;  // Rgba8UnormSrgb 256×1

@vertex
fn vs_main(in: VsIn) -> VsOut {
	var out: VsOut;
	out.pos = vec4<f32>(in.pos, 0.0, 1.0);
	out.uv = in.uv;
	out.origin = in.origin;
	out.flags = in.flags;
	return out;
}

// Source palette indices for each team's 8-color ramp (Red, Green, Blue,
// Gray, Yellow/Derelict) — from the original game's resource manager.
const RAMP: array<array<u32, 8>, 5> = array<array<u32, 8>, 5>(
	array<u32, 8>(56u, 57u, 58u, 59u, 60u, 61u, 62u, 63u),
	array<u32, 8>(32u, 33u, 34u, 35u, 36u, 37u, 38u, 39u),
	array<u32, 8>(48u, 49u, 50u, 51u, 52u, 53u, 54u, 55u),
	array<u32, 8>(255u, 161u, 172u, 169u, 216u, 213u, 212u, 207u),
	array<u32, 8>(216u, 215u, 214u, 213u, 212u, 211u, 210u, 209u),
);

fn remap(idx: u32, team: u32) -> u32 {
	if ((idx >= 32u && idx < 40u) || (idx >= 48u && idx < 64u)) {
		return RAMP[team][idx & 7u];
	}
	return idx;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
	let px = in.origin + vec2<u32>(vec2<i32>(floor(in.uv)));
	let idx = textureLoad(atlas, vec2<i32>(px), 0).r;
	if (idx == 0u) {
		discard;
	}
	if ((in.flags & 8u) != 0u) {
		// Shadow: the game darkens whatever is underneath.
		return vec4<f32>(0.0, 0.0, 0.0, 0.45);
	}
	let team = in.flags & 7u;
	let color = textureLoad(palette, vec2<i32>(i32(remap(idx, team)), 0), 0).rgb;
	return vec4<f32>(color, 1.0);
}
