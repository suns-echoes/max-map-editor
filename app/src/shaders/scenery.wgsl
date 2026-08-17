// Scenery quads (see scenery_render.rs): screen-space quads sampling the
// scenery sprite atlas through the working palette, so a palette edit or a
// colour cycle recolours a placed object exactly as it recolours the terrain
// tile the art was cut from.
//
// The atlas is Rgba8Uint and carries a sprite's whole cut in one texel:
// **r** = the body's palette index (0 = nothing there), **g** = the shadow's
// alpha (0 = no shadow), **b** = how deep inside its own silhouette the pixel
// sits (`Sprite::edge_distance`, unused by the draw today), **a** = how high the
// object stands there (`Sprite::height_field`). r and g are mutually exclusive
// by construction - a pixel is the object's own ink or ground the object shades.
//
// **The shadows are one layer, and it lies under every body**, so two objects
// whose shadows overlap merge into one shadow at the deeper of the two alphas
// instead of darkening the ground twice over.
//
// **The bodies are one layer too**, because a placement's blend mode has to see
// the scenery under it and no fragment can read the target it draws into. The
// layer holds a *rank*, not a colour: every ink of the working palette is sorted
// by brightness once, and the layer stores `rank + 1` (0 = no scenery here). So
// `brighter` is plain `max` blending and `darker` is `min` - and because the
// rank round-trips back to one palette index, the result is always one of the
// two inks and never a mixture, which is what lets the WRL export agree with the
// screen pixel for pixel.
//
// The passes:
//
//  1. `fs_shadow` -> the shadow layer, `max`.
//  2. `fs_ink` -> the ink layer with plain replace, placements in *reverse*
//     order, so each pixel starts out holding the earliest ink covering it.
//     Without it `min` would meet a cleared 0 and darken everything to nothing,
//     and `higher` would compare against a cleared height rather than against
//     the object it lands on. It seeds the height layer in the same stroke.
//  3. `fs_ink` again, placements in order, one draw per run of equal mode, with
//     that mode's blending - and that mode's depth state, which is what settles
//     `higher`.
//  4. `vs_full` + `fs_merge` darken the map through the shadow layer once.
//  5. `vs_full` + `fs_resolve` paints the ink layer over it.
//  6. `fs_ghost` draws the placement tool's ghost, which stays out of the layer
//     so it can be translucent - but *reads* the layer, so its ink meets the
//     placements under it by its own blend mode, exactly as the placement a
//     click makes will. The mode rides in per vertex (the ghost is one quad, so
//     a run-per-mode pipeline split would buy nothing).
//
// Unlike units.wgsl this applies no team remap and no game statics: scenery is
// terrain art, and it must read against the same palette the map renderer uses
// or a placed mountain would not match the tiles beside it.

struct VsIn {
	@location(0) pos: vec2<f32>,     // clip space
	@location(1) uv: vec2<f32>,      // sprite-local pixels (0..w, 0..h)
	@location(2) origin: vec2<u32>,  // sprite's pixel origin in the atlas
	@location(3) alpha: f32,         // 1 = placed, < 1 = the tool's ghost preview
	@location(4) mode: u32,          // blend mode, `SceneryBlend::ALL` order (ghost only)
};

struct VsOut {
	@builtin(position) pos: vec4<f32>,
	@location(0) uv: vec2<f32>,
	@location(1) @interpolate(flat) origin: vec2<u32>,
	@location(2) @interpolate(flat) alpha: f32,
	@location(3) @interpolate(flat) mode: u32,
};

@group(0) @binding(0) var atlas:   texture_2d<u32>;  // Rgba8Uint
@group(0) @binding(1) var palette: texture_2d<f32>;  // Rgba8UnormSrgb 256x1
// 256x2 R8Uint: row 0 = an ink's rank, row 1 = the ink of a rank. Rebuilt with
// the palette, since brightness order follows the colours.
@group(0) @binding(2) var ranks:   texture_2d<u32>;

@vertex
fn vs_main(in: VsIn) -> VsOut {
	var out: VsOut;
	out.pos = vec4<f32>(in.pos, 0.0, 1.0);
	out.uv = in.uv;
	out.origin = in.origin;
	out.alpha = in.alpha;
	out.mode = in.mode;
	return out;
}

fn texel_at(in: VsOut) -> vec4<u32> {
	let px = in.origin + vec2<u32>(vec2<i32>(floor(in.uv)));
	return textureLoad(atlas, vec2<i32>(px), 0);
}

// Pass 1: the shadow layer. Only the shade plane contributes, and the target
// blends with `max`, so overlapping shadows merge rather than compound.
@fragment
fn fs_shadow(in: VsOut) -> @location(0) f32 {
	let texel = texel_at(in);
	if (texel.r != 0u || texel.g == 0u) {
		discard;
	}
	return f32(texel.g) / 255.0 * in.alpha;
}

struct InkOut {
	@location(0) rank: f32,
	// How high this object stands here, `height / 255`. The pipeline's depth
	// state is the placement's mode: `higher` tests `GreaterEqual` against the
	// layer, every other mode writes without testing, and both write - so the
	// height layer always describes the object whose ink is actually there.
	@builtin(frag_depth) height: f32,
};

// Passes 2 and 3: this placement's ink as a brightness rank. The pipeline's
// blend op is the placement's mode - replace, `max` or `min` - so the mode is
// applied against whatever rank is already in the layer; `higher` is settled by
// the depth test instead, because "keep the taller object's ink" is a condition
// on one value deciding another, which no blend function can express.
@fragment
fn fs_ink(in: VsOut) -> InkOut {
	let texel = texel_at(in);
	if (texel.r == 0u) {
		discard;
	}
	let rank = textureLoad(ranks, vec2<i32>(i32(texel.r), 0), 0).r;
	return InkOut(f32(rank + 1u) / 255.0, f32(texel.a) / 255.0);
}

// ----- the layer readers ------------------------------------------------------

// Group 1, not 0: these are the *targets* of earlier passes, and a texture
// cannot be bound while it is being rendered into - so only the pipelines that
// read them declare the group, one binding each. The ghost reads the ink layer
// too: by pass 6 it is finished being drawn into, and the ghost needs to know
// what its own ink is landing on, and how high that thing stands.
@group(1) @binding(0) var shadow: texture_2d<f32>;  // R8Unorm, screen-sized
@group(1) @binding(1) var ink:    texture_2d<f32>;  // R8Unorm, screen-sized
// The height layer, which is the ink passes' depth buffer - the ghost is the one
// reader, and it draws after they have let go of it.
@group(1) @binding(2) var height: texture_depth_2d;

// A full-viewport triangle - the resolves cover whatever the scissor allows, so
// they need no vertex buffer and no geometry of their own.
@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
	let x = f32(i32(i) / 2) * 4.0 - 1.0;
	let y = f32(i32(i) & 1) * 4.0 - 1.0;
	return vec4<f32>(x, y, 0.0, 1.0);
}

// Pass 6: one quad's own ink, straight onto the map - the ghost only, and
// translucent, so it never enters the ink layer. It still *reads* that layer:
// where a placement's ink already covers the pixel, the ghost's own rank meets
// it under `in.mode` (0 replace, 1 `max`, 2 `min`) - the same arithmetic the
// pipeline blend does for a real placement, done by hand because a translucent
// quad cannot go through the layer. Over bare ground (`stored == 0`) every mode
// is just the ghost's own ink, which is what the pass-2 seed buys a placement.
//
// `higher` (mode 3) is the same by hand: the depth test a placement would get,
// read off the height layer, `>=` so a tie keeps the ghost's own ink exactly as
// `SceneryBlend::pick` keeps the placement's.
@fragment
fn fs_ghost(in: VsOut) -> @location(0) vec4<f32> {
	let texel = texel_at(in);
	if (texel.r == 0u) {
		discard;
	}
	// The layer's encoding: `rank + 1`, so 0 can mean "no scenery here".
	var stored_rank = textureLoad(ranks, vec2<i32>(i32(texel.r), 0), 0).r + 1u;
	let under = u32(round(textureLoad(ink, vec2<i32>(in.pos.xy), 0).r * 255.0));
	if (under != 0u) {
		if (in.mode == 1u) {
			stored_rank = max(stored_rank, under);
		} else if (in.mode == 2u) {
			stored_rank = min(stored_rank, under);
		} else if (in.mode == 3u) {
			let stood = u32(round(textureLoad(height, vec2<i32>(in.pos.xy), 0) * 255.0));
			if (stood > texel.a) {
				stored_rank = under;
			}
		}
	}
	let index = textureLoad(ranks, vec2<i32>(i32(stored_rank - 1u), 1), 0).r;
	return vec4<f32>(textureLoad(palette, vec2<i32>(i32(index), 0), 0).rgb, in.alpha);
}

@fragment
fn fs_merge(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
	let a = textureLoad(shadow, vec2<i32>(pos.xy), 0).r;
	if (a <= 0.0) {
		discard;
	}
	// The same flat black the game (and units.wgsl) casts, at the deepest alpha
	// any one object laid down here.
	return vec4<f32>(0.0, 0.0, 0.0, a);
}

@fragment
fn fs_resolve(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
	let stored = u32(round(textureLoad(ink, vec2<i32>(pos.xy), 0).r * 255.0));
	if (stored == 0u) {
		discard;
	}
	let index = textureLoad(ranks, vec2<i32>(i32(stored - 1u), 1), 0).r;
	return vec4<f32>(textureLoad(palette, vec2<i32>(i32(index), 0), 0).rgb, 1.0);
}
