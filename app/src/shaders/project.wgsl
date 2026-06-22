// Layered project pass: water + ground lookups per
// fragment, per-cell transforms, ground index-0 falls through to water.
//
// Atlas is a 2D-array "3D atlas": 1024×1024 layers of 16×16 tiles -
// global tile index → (layer = idx / 256, cell = idx % 256). Cell data is
// Rgba16Uint: r/g = water idx+1 / transform, b/a = ground idx+1 / transform.
// Transform bits match map-core's `Transform::bits`: rot in bits 0–1
// (clockwise quarter turns), mirror in bit 2; sampling mirrors
// `transform_into` (undo rotation ccw, then un-mirror x).

const TILE_SIZE: f32 = 64.0;

struct Uniforms {
	screen_size:   vec2<f32>,
	pan:           vec2<f32>,  // world px at viewport top-left
	map_size:      vec2<f32>,  // in tiles
	zoom:          f32,        // screen px per world px
	tiles_per_row: u32,        // unused here (atlas is layered)
};

// Pass overlay: when enabled, tint each cell by its pass value.
struct Overlay {
	enabled:    u32,  // 0 = off
	layer_mask: u32,  // bit n = composite layer n (bit0 water, bit1 ground)
	_pad1:      u32,
	_pad2:      u32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var cells:   texture_2d<u32>;        // Rgba16Uint
@group(0) @binding(2) var atlas:   texture_2d_array<u32>;  // R8Uint
@group(0) @binding(3) var palette: texture_2d<f32>;        // Rgba8UnormSrgb 256×1
@group(0) @binding(4) var pass_tex: texture_2d<u32>;       // R8Uint, pass per cell
@group(0) @binding(5) var<uniform> overlay: Overlay;
@group(0) @binding(6) var tile_mask: texture_2d<u32>;      // R16Uint, per-tile mask+1 (0 = opaque)

const VOID_COLOR: vec4<f32> = vec4<f32>(0.045, 0.045, 0.06, 1.0);

// The map is framed by a thin highlight outline. (The app background behind it
// is dimmed on the CPU side - see the steel fill in `render_frame` - so the map
// itself keeps its true colours.)
const OUTLINE_PX: f32 = 2.0;
// Editor accent green (#44FF00, sRGB→linear) - matches theme::ACCENT.
const OUTLINE_COLOR: vec4<f32> = vec4<f32>(0.058, 1.0, 0.0, 1.0);

// Per-pass channel mask (simple-wrl-editor parity): 0 land→green,
// 1 water→blue, 2 shore→yellow, 3 blocked→red.
fn channel_mask(pv: u32) -> vec3<f32> {
	if (pv == 0u) { return vec3<f32>(0.0, 1.0, 0.0); }
	if (pv == 1u) { return vec3<f32>(0.0, 0.0, 1.0); }
	if (pv == 2u) { return vec3<f32>(1.0, 1.0, 0.0); }
	return vec3<f32>(1.0, 0.0, 0.0);
}

// Replace the tile color with its grayscale times the pass mask - the
// grayscale is the "color floor" that keeps very dark tiles visible.
fn apply_overlay(color: vec4<f32>, tile_xy: vec2<u32>) -> vec4<f32> {
	if (overlay.enabled == 0u) {
		return color;
	}
	let pv = textureLoad(pass_tex, vec2<i32>(tile_xy), 0).r;
	let gray = dot(color.rgb, vec3<f32>(0.299, 0.587, 0.114));
	return vec4<f32>(vec3<f32>(gray) * channel_mask(pv), color.a);
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
	let x = f32((vi << 1u) & 2u) * 2.0 - 1.0;
	let y = f32(vi & 2u) * 2.0 - 1.0;
	return vec4<f32>(x, -y, 0.0, 1.0);
}

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

fn atlas_pixel(index: u32, sub: vec2<u32>) -> u32 {
	let layer = index >> 8u;
	let slot = index & 255u;
	let origin = vec2<u32>((slot % 16u) * 64u, (slot / 16u) * 64u);
	return textureLoad(atlas, vec2<i32>(origin + sub), i32(layer), 0).r;
}

// The family transparency mask of a tile: 0 = opaque, else mask color + 1.
// The table is 256 wide (x = idx & 255, y = idx >> 8).
fn tile_mask_of(index: u32) -> u32 {
	return textureLoad(tile_mask, vec2<i32>(i32(index & 255u), i32(index >> 8u)), 0).r;
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
	let world = u.pan + frag.xy / u.zoom;
	let map_px = u.map_size * TILE_SIZE;

	// How far the fragment lies outside the map rectangle, in *screen* px (so
	// the outline keeps its width at any zoom). 0 means inside the map.
	let outside_world = max(max(-world, world - map_px), vec2<f32>(0.0, 0.0));
	let outside_px = max(outside_world.x, outside_world.y) * u.zoom;
	if (outside_px > 0.0) {
		// A thin highlight frame hugs the map edge; beyond it, discard so the
		// app-background steel (drawn before this pass) shows through (#16).
		if (outside_px <= OUTLINE_PX) {
			return OUTLINE_COLOR;
		}
		discard;
		return VOID_COLOR;
	}

	// "Show only selected" hides the non-active layers (a view filter).
	let show_water = (overlay.layer_mask & 1u) != 0u;
	let show_ground = (overlay.layer_mask & 2u) != 0u;

	let tile_xy = vec2<u32>(world / TILE_SIZE);
	let sub = vec2<u32>(world - vec2<f32>(tile_xy) * TILE_SIZE);
	let cell = textureLoad(cells, vec2<i32>(tile_xy), 0);

	// Ground over water. Only families with a mask are transparent (the mask
	// color falls through); opaque families fully cover.
	var g_present = false;
	var g_pixel = 0u;
	var g_mask = 0u; // 0 = opaque family, else mask color + 1
	if (show_ground && cell.b > 0u) {
		g_present = true;
		g_pixel = atlas_pixel(cell.b - 1u, transformed_sub(sub, cell.a));
		g_mask = tile_mask_of(cell.b - 1u);
	}
	// Opaque ground, or a non-mask pixel of a masked family: the ground wins.
	if (g_present && (g_mask == 0u || g_pixel != g_mask - 1u)) {
		return apply_overlay(textureLoad(palette, vec2<i32>(i32(g_pixel), 0), 0), tile_xy);
	}
	// A masked (transparent) ground pixel falls through to the water beneath.
	if (show_water && cell.r > 0u) {
		let pixel = atlas_pixel(cell.r - 1u, transformed_sub(sub, cell.g));
		return apply_overlay(textureLoad(palette, vec2<i32>(i32(pixel), 0), 0), tile_xy);
	}
	// Masked ground pixel with no water beneath - show the mask color itself.
	if (g_present) {
		let mi = select(0u, g_mask - 1u, g_mask > 0u);
		return apply_overlay(textureLoad(palette, vec2<i32>(i32(mi), 0), 0), tile_xy);
	}
	return VOID_COLOR;
}
