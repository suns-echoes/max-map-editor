// Grid overlay: cell-bevel lines drawn as a fullscreen pass
// after the map (so it works for any render path and sits on top of the pass
// overlay). World is in pixels with 64px tiles, matching the map shaders.
//
// Look: each cell reads as a raised tile — a light inner edge on its top/left
// and a dark inner edge on its bottom/right. Because every boundary always
// pairs a light band with a dark band, the grid stays visible over any
// terrain color (one of the two tones always contrasts).
//
// Robustness: all band math happens in *screen* pixels with a band width
// clamped to >= 1px, so lines can never fall between pixel centers and
// vanish at unlucky zoom levels (the old single-line test did exactly that).
// The only intentional fade-out is when cells get smaller than ~6px on
// screen — below that a grid is noise, and it fades smoothly, never pops.

struct U {
	screen_size: vec2<f32>,
	pan:         vec2<f32>,  // world px at viewport top-left
	map_size:    vec2<f32>,  // in tiles
	zoom:        f32,        // screen px per world px
	strength:    f32,        // master opacity of the bevel
};

@group(0) @binding(0) var<uniform> u: U;

const TILE: f32 = 64.0;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
	let x = f32((vi << 1u) & 2u) * 2.0 - 1.0;
	let y = f32(vi & 2u) * 2.0 - 1.0;
	return vec4<f32>(x, -y, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
	let world = u.pan + frag.xy / u.zoom;
	let map_px = u.map_size * TILE;

	// Only inside the map; cell edges include the four map borders.
	if (world.x < 0.0 || world.y < 0.0 || world.x > map_px.x || world.y > map_px.y) {
		discard;
	}

	let cell_px = TILE * u.zoom;
	// Smooth legibility fade: full strength at >= 6px cells, gone below 3px.
	let fade = clamp((cell_px - 3.0) / 3.0, 0.0, 1.0);
	if (fade <= 0.0) {
		discard;
	}

	// Position inside the cell, in screen px measured from its top-left.
	let f = (world - floor(world / TILE) * TILE) * u.zoom;
	// Bevel width: never below 1px (coverage guarantee), capped at 2px so
	// high zoom keeps a crisp edge instead of a fat frame.
	let w = clamp(u.zoom, 1.0, 2.0);

	let d_light = min(f.x, f.y);                       // top/left inner edge
	let d_dark = min(cell_px - f.x, cell_px - f.y);    // bottom/right inner edge
	let d = min(d_light, d_dark);
	if (d >= w) {
		discard;
	}

	// The nearer edge wins the tone; light is kept a touch softer so the
	// emboss reads as relief, not as a white lattice.
	let is_light = d_light < d_dark;
	let tone = select(0.0, 1.0, is_light);
	let alpha = select(u.strength, u.strength * 0.75, is_light);
	return vec4<f32>(vec3<f32>(tone), alpha * fade);
}
