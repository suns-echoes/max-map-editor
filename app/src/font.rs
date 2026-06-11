//! Prerendered MAX-font label atlases at the UI's discrete sizes.
//!
//! `max_font` ships one coverage atlas baked at 60 px; rendering a 12- or 16-px
//! label by GPU-shrinking that sheet every frame undersamples it (soft, and it
//! shimmers / drifts as positions move sub-pixel). Instead we **box-downsample**
//! the master to each size once at startup, so labels draw 1:1 (one screen px =
//! one atlas texel) — crisp and stable. Integer per-size advance/offset tables
//! come out of the same bake, so layout is pixel-exact too.

use std::sync::OnceLock;

use crate::max_font::{ADVANCE, ATLAS, ATLAS_W, CELL_H, COUNT, OFFSET};

/// The UI text ladder (px): every label snaps to the nearest of these —
/// 12 (small / dense panels) and 16 (body, titles, buttons).
pub const SIZES: [u32; 2] = [12, 16];

/// One size's prerendered coverage atlas (`atlas_w` × `px`, R8) plus the
/// integer layout metrics for that size.
pub struct Sized {
	pub px: u32,
	pub atlas: Vec<u8>,
	pub atlas_w: u32,
	pub advance: [u32; COUNT as usize],
	pub offset: [u32; COUNT as usize],
}

/// Snap a requested px height to the nearest baked size (ties → smaller).
pub fn snap(px: f32) -> u32 {
	let mut best = SIZES[0];
	let mut best_d = (SIZES[0] as f32 - px).abs();
	for &s in &SIZES[1..] {
		let d = (s as f32 - px).abs();
		if d < best_d {
			best = s;
			best_d = d;
		}
	}
	best
}

/// The baked set (all sizes), built once on first use.
pub fn all() -> &'static [Sized] {
	static SET: OnceLock<Vec<Sized>> = OnceLock::new();
	SET.get_or_init(|| SIZES.iter().map(|&px| bake(px)).collect())
}

/// Metrics + atlas for a specific (already-snapped) size; the first size as a
/// fallback for an unknown `px`.
pub fn sized(px: u32) -> &'static Sized {
	let set = all();
	set.iter().find(|s| s.px == px).unwrap_or(&set[0])
}

/// Box-downsample the master atlas to `px` tall, repacking glyphs at their
/// rounded advances.
fn bake(px: u32) -> Sized {
	let scale = px as f32 / CELL_H as f32;
	let mut advance = [0u32; COUNT as usize];
	let mut offset = [0u32; COUNT as usize];
	let mut cursor = 0u32;
	for g in 0..COUNT as usize {
		offset[g] = cursor;
		// Round to an integer advance, but never collapse a drawable glyph.
		advance[g] = ((ADVANCE[g] as f32 * scale).round() as u32).max(u32::from(ADVANCE[g] > 0));
		cursor += advance[g];
	}
	let atlas_w = cursor.max(1);
	let mut atlas = vec![0u8; (atlas_w * px) as usize];
	for g in 0..COUNT as usize {
		let (src_x, src_w) = (OFFSET[g] as u32, ADVANCE[g] as u32);
		let (dst_x, dst_w) = (offset[g], advance[g]);
		if src_w == 0 || dst_w == 0 {
			continue;
		}
		for dy in 0..px {
			let y0 = dy as f32 / px as f32 * CELL_H as f32;
			let y1 = (dy + 1) as f32 / px as f32 * CELL_H as f32;
			for dx in 0..dst_w {
				let x0 = src_x as f32 + dx as f32 / dst_w as f32 * src_w as f32;
				let x1 = src_x as f32 + (dx + 1) as f32 / dst_w as f32 * src_w as f32;
				atlas[(dy * atlas_w + dst_x + dx) as usize] = area_avg(x0, x1, y0, y1);
			}
		}
	}
	// A size that isn't a multiple of 16 box-downsamples to a softer coverage
	// (the master's 60-px grid doesn't divide it cleanly); sharpen the edge ramp
	// so small text stays readable. The 16-px tier is clean and left untouched.
	if !px.is_multiple_of(16) {
		sharpen(&mut atlas, atlas_w, px);
	}
	Sized { px, atlas, atlas_w, advance, offset }
}

/// Unsharp-mask the coverage atlas: subtract a 3×3 box blur to steepen glyph
/// edges (`out = c + AMOUNT·(c − blur)`), making sub-16-px labels crisper.
fn sharpen(atlas: &mut [u8], w: u32, h: u32) {
	const AMOUNT: f32 = 0.6;
	let (w, h) = (w as usize, h as usize);
	let src = atlas.to_vec();
	for y in 0..h {
		for x in 0..w {
			let mut sum = 0.0f32;
			for dy in -1i32..=1 {
				for dx in -1i32..=1 {
					let sx = (x as i32 + dx).clamp(0, w as i32 - 1) as usize;
					let sy = (y as i32 + dy).clamp(0, h as i32 - 1) as usize;
					sum += src[sy * w + sx] as f32;
				}
			}
			let c = src[y * w + x] as f32;
			let sharp = c + AMOUNT * (c - sum / 9.0);
			atlas[y * w + x] = sharp.round().clamp(0.0, 255.0) as u8;
		}
	}
}

/// Average master coverage over the box `[x0,x1) × [y0,y1)` (in master texels).
fn area_avg(x0: f32, x1: f32, y0: f32, y1: f32) -> u8 {
	let (ix0, ix1) = (x0.floor() as u32, (x1.ceil() as u32).min(ATLAS_W));
	let (iy0, iy1) = (y0.floor() as u32, (y1.ceil() as u32).min(CELL_H));
	let mut sum = 0.0f32;
	let mut wsum = 0.0f32;
	for sy in iy0..iy1 {
		let cov_y = (sy as f32 + 1.0).min(y1) - (sy as f32).max(y0);
		if cov_y <= 0.0 {
			continue;
		}
		for sx in ix0..ix1 {
			let cov_x = (sx as f32 + 1.0).min(x1) - (sx as f32).max(x0);
			if cov_x <= 0.0 {
				continue;
			}
			let wgt = cov_x * cov_y;
			sum += ATLAS[(sy * ATLAS_W + sx) as usize] as f32 * wgt;
			wsum += wgt;
		}
	}
	if wsum > 0.0 { (sum / wsum).round().clamp(0.0, 255.0) as u8 } else { 0 }
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
