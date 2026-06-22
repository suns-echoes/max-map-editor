//! Prerendered MAX-font label atlases at the UI's discrete sizes.
//!
//! Each size is **rasterized from the TrueType outlines** of
//! `assets/max_square.ttf` at its native pixel size (see [`crate::ttf`]), once
//! at startup, so labels draw 1:1 (one screen px = one atlas texel) - crisp at
//! every size, including the in-between UI-scale sizes (15/18/20/24) that a
//! single 60-px master couldn't cleanly produce. Integer per-size
//! advance/offset tables fall out of the bake, so layout is pixel-exact too.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::max_font::{COUNT, FIRST};

/// The MAX UI font (TrueType outlines), rasterized at runtime.
static TTF: &[u8] = include_bytes!("../assets/max_square.ttf");

/// Parse the embedded font once.
fn font() -> &'static crate::ttf::Font<'static> {
	static F: OnceLock<crate::ttf::Font<'static>> = OnceLock::new();
	F.get_or_init(|| crate::ttf::Font::parse(TTF))
}

/// The "design cell" the original 60-px master defined, in font units: cap top
/// ([`ASCENT_FU`]) down to the descender. A label cell of height `px` maps this
/// span to `px`, with the baseline `ASCENT_FU` units below the top - so the
/// runtime rasterization sits at the same baseline/size the UI was built around.
/// (Measured from the shipped master: caps fill rows 0..48 of the 60-px cell,
/// descenders reach row 59 → 3072 above + 768 below = 3840.)
const DESIGN_CELL_FU: f32 = 3840.0;
const ASCENT_FU: f32 = 3072.0;

/// The UI text ladder (px): every label snaps to the nearest of these -
/// 12 (small / dense panels) and 16 (body, titles, buttons). This is the
/// **logical** ladder; the px a label *occupies* on screen. The atlas it
/// samples from may be larger (see [`render_px`]) when the UI is scaled up.
pub const SIZES: [u32; 2] = [12, 16];

/// Every baked coverage atlas (px tall): the logical ladder times each
/// supported UI scale, so a scaled label samples a 1:1 atlas (crisp) instead of
/// a GPU-upscaled smaller one. `{12, 16} × {1.0, 1.25, 1.5}` = these six.
const ATLAS_SIZES: [u32; 6] = [12, 15, 16, 18, 20, 24];

/// Current UI scale (1.0 / 1.25 / 1.5), packed as `f32` bits. One global the
/// label layout + render-size selection read, so fonts scale with the rest of
/// the chrome from a single source. The shell sets it once per frame (and when
/// the scale command runs); it defaults to 1.0 so headless/unscaled runs stay
/// byte-identical. `0x3F80_0000` = `1.0_f32` bits.
static UI_SCALE: AtomicU32 = AtomicU32::new(0x3F80_0000);

/// Set the global UI scale (shell-only; see [`UI_SCALE`]).
pub fn set_ui_scale(scale: f32) {
	UI_SCALE.store(scale.to_bits(), Ordering::Relaxed);
}

/// The current global UI scale.
pub fn ui_scale() -> f32 {
	f32::from_bits(UI_SCALE.load(Ordering::Relaxed))
}

/// One size's prerendered coverage atlas (`atlas_w` × `px`, R8) plus the
/// integer layout metrics for that size.
pub struct Sized {
	pub px: u32,
	pub atlas: Vec<u8>,
	pub atlas_w: u32,
	pub advance: [u32; COUNT as usize],
	pub offset: [u32; COUNT as usize],
}

/// Nearest of `sizes` to `px` (ascending input → ties resolve to the smaller).
fn nearest(px: f32, sizes: &[u32]) -> u32 {
	let mut best = sizes[0];
	let mut best_d = (sizes[0] as f32 - px).abs();
	for &s in &sizes[1..] {
		let d = (s as f32 - px).abs();
		if d < best_d {
			best = s;
			best_d = d;
		}
	}
	best
}

/// Snap a requested px height to the nearest **logical** ladder size (ties →
/// smaller). Layout measures and positions in these units.
pub fn snap(px: f32) -> u32 {
	nearest(px, &SIZES)
}

/// The baked **atlas** size to render a `logical_px` label at under the current
/// UI scale: snap to the logical ladder, then up to the matching scaled atlas
/// (`[`text::push_label`] draws geometry back at the logical size, so layout is
/// unchanged - only the sampled atlas is sharper). At scale 1.0 this is
/// [`snap`].
pub fn render_px(logical_px: f32) -> u32 {
	nearest(snap(logical_px) as f32 * ui_scale(), &ATLAS_SIZES)
}

/// The baked set (all atlas sizes), built once on first use.
pub fn all() -> &'static [Sized] {
	static SET: OnceLock<Vec<Sized>> = OnceLock::new();
	SET.get_or_init(|| ATLAS_SIZES.iter().map(|&px| bake(px)).collect())
}

/// Metrics + atlas for a specific (already-snapped) size; the first size as a
/// fallback for an unknown `px`.
pub fn sized(px: u32) -> &'static Sized {
	let set = all();
	set.iter().find(|s| s.px == px).unwrap_or(&set[0])
}

/// Rasterize every glyph from the TTF outlines at cell height `px`, packed
/// left-to-right at their rounded advances (an R8 coverage strip + integer
/// metrics). The design cell ([`DESIGN_CELL_FU`]) maps to `px`, baseline
/// [`ASCENT_FU`] down from the top.
fn bake(px: u32) -> Sized {
	let f = font();
	let scale = px as f32 / DESIGN_CELL_FU;
	let baseline = ASCENT_FU * scale; // px from the top of the cell
	let mut advance = [0u32; COUNT as usize];
	let mut offset = [0u32; COUNT as usize];
	// Flatten each glyph's outline into the cell's pixel space (y flipped) up
	// front, so the second pass can fill straight into the packed atlas.
	let mut glyph_edges: Vec<Vec<[f32; 4]>> = Vec::with_capacity(COUNT as usize);
	let mut cursor = 0u32;
	for g in 0..COUNT as usize {
		let ch = char::from_u32(FIRST as u32 + g as u32).unwrap_or(' ');
		let gid = f.glyph_index(ch);
		let contours = f.outline(gid, 0);
		// Keep a drawable glyph at least 1 px wide (matches the old bake).
		let w = (f.advance_width(gid) as f32 * scale).round() as u32;
		let w = w.max(u32::from(!contours.is_empty()));
		let mut edges = Vec::new();
		for cont in &contours {
			let mapped: Vec<(f32, f32, bool)> =
				cont.iter().map(|&(x, y, on)| (x * scale, baseline - y * scale, on)).collect();
			crate::ttf::flatten(&mapped, &mut edges);
		}
		offset[g] = cursor;
		advance[g] = w;
		cursor += w;
		glyph_edges.push(edges);
	}
	let atlas_w = cursor.max(1);
	let mut atlas = vec![0u8; (atlas_w * px) as usize];
	for g in 0..COUNT as usize {
		let w = advance[g] as usize;
		if w == 0 || glyph_edges[g].is_empty() {
			continue;
		}
		let cov = crate::ttf::fill(&glyph_edges[g], w, px as usize);
		let ox = offset[g] as usize;
		for row in 0..px as usize {
			let dst = row * atlas_w as usize + ox;
			atlas[dst..dst + w].copy_from_slice(&cov[row * w..row * w + w]);
		}
	}
	Sized { px, atlas, atlas_w, advance, offset }
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn snap_picks_nearest_smaller_on_tie() {
		assert_eq!(snap(11.0), 12);
		assert_eq!(snap(13.0), 12);
		assert_eq!(snap(14.0), 12); // tie 12/16 → smaller
		assert_eq!(snap(15.0), 16);
		assert_eq!(snap(16.0), 16);
		assert_eq!(snap(40.0), 16);
	}

	#[test]
	fn rasterized_glyphs_sit_on_the_baseline() {
		// At 16 px the baseline is ~13 px down (0.8 × the cell). A cap ('H') must
		// carry no ink below it; a descender ('g') must - i.e. the TTF outlines are
		// rasterized at the right baseline/scale, not floating or clipped.
		let s = sized(16);
		let ink_below = |ch: u8, from: u32| -> u32 {
			let g = (ch - FIRST) as usize;
			(from..s.px)
				.map(|y| {
					(0..s.advance[g]).map(|x| s.atlas[(y * s.atlas_w + s.offset[g] + x) as usize] as u32).sum::<u32>()
				})
				.sum()
		};
		assert_eq!(ink_below(b'H', 14), 0, "a cap has no ink in the bottom 2 rows (below the baseline)");
		assert!(ink_below(b'g', 14) > 0, "a descender reaches below the baseline");
	}

	#[test]
	fn baked_atlas_matches_its_metrics() {
		for s in all() {
			assert_eq!(s.atlas.len(), (s.atlas_w * s.px) as usize, "atlas size = w*h");
			// Packed offsets are contiguous and end exactly at atlas_w.
			let mut cursor = 0;
			for g in 0..COUNT as usize {
				assert_eq!(s.offset[g], cursor);
				cursor += s.advance[g];
			}
			assert_eq!(cursor, s.atlas_w);
			// A capital 'M' (a dense glyph) must carry some ink at every size.
			let g = (b'M' - crate::max_font::FIRST) as usize;
			let col_ink: u32 = (0..s.px)
				.map(|y| {
					(0..s.advance[g]).map(|x| s.atlas[(y * s.atlas_w + s.offset[g] + x) as usize] as u32).sum::<u32>()
				})
				.sum();
			assert!(col_ink > 0, "size {} 'M' should have coverage", s.px);
		}
	}
}
