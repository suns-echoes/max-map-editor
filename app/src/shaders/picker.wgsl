// Tile Explorer grid pass + toolbox active-tile preview
//: screen-space quads sampling the project atlas → palette exactly
// like project.wgsl's atlas_pixel. `transform` carries map-core's bits
// (rot 0–1, mirror bit 2; the grid passes 0 = base art). Transparency
// matches the map: a tile's family mask color (from the tile_mask table)
// renders as a dim translucent fill; every other pixel - including index 0
// for an opaque family - is solid. Tiles + templates thus read like the map.

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

@group(0) @binding(0) var atlas:     texture_2d_array<u32>;  // R8Uint
@group(0) @binding(1) var palette:   texture_2d<f32>;        // Rgba8UnormSrgb 256×1
@group(0) @binding(2) var tile_mask: texture_2d<u32>;        // R16Uint, per-tile mask+1 (0 = opaque)

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
	// The tile's family mask (0 = opaque family, else mask color + 1). A pixel
	// equal to the mask color is transparent (the map shows water through it;
	// here a dim translucent fill stands in). Opaque families show every pixel.
	let mask = textureLoad(tile_mask, vec2<i32>(i32(slot), i32(layer)), 0).r;
	if (mask != 0u && pixel == mask - 1u) {
		return vec4<f32>(0.0, 0.0, 0.0, 0.35 * in.alpha);
	}
	// Palette colours are opaque here: index 0 is only see-through when it *is*
	// the family's mask (handled above). The palette texture's own alpha (0 at
	// slot 0, set by the cycler) is ignored, so a no-mask tile's index-0 pixels
	// stay solid - matching the map (which doesn't alpha-blend).
	let c = textureLoad(palette, vec2<i32>(i32(pixel), 0), 0);
	return vec4<f32>(c.rgb, in.alpha);
}
