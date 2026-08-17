//! CPU-composes a template's tiles into an RGBA thumbnail — full-size for the
//! Delete / Rename dialogs ([`compose`]), and aspect-fit downscaled for the
//! Templates panel's thumbnail atlas ([`thumb`]). Frozen stills through a
//! palette LUT: panel grids show the rest palette (no water cycling) by
//! design; only the map canvas keeps the live pass.

use map_core::{Project, Template};

/// One tile side in pixels (the pack tile format).
const N: usize = 64;

/// Composes `template` into tightly packed RGBA8 at [`N`] px per cell,
/// returning `(rgba, width_px, height_px)`. `lut` is the 256-entry RGBA
/// palette (`cycler.rgba()`); empty cells and out-of-range indices read
/// palette entry 0.
pub fn compose(project: &Project, template: &Template, lut: &[u8]) -> (Vec<u8>, u32, u32) {
	let tw = template.width.max(1) as usize;
	let th = template.height.max(1) as usize;
	let (cw, ch) = (tw * N, th * N);
	let mut rgba = vec![0u8; cw * ch * 4];
	for dy in 0..template.height {
		for dx in 0..template.width {
			let stack = template.cell_layers(project, dx, dy);
			let cell = project.compose_stack(&stack); // [u8; N*N] palette indices
			for py in 0..N {
				for px in 0..N {
					let idx = cell[py * N + px] as usize;
					let Some(src) = lut.get(idx * 4..idx * 4 + 4) else { continue };
					let dstx = dx as usize * N + px;
					let dsty = dy as usize * N + py;
					let d = (dsty * cw + dstx) * 4;
					rgba[d..d + 4].copy_from_slice(src);
				}
			}
		}
	}
	(rgba, cw as u32, ch as u32)
}

/// [`compose`], then nearest-downscaled so the larger side is at most
/// `max_px` (small templates stay full-size — no upscaling). Returns
/// `(rgba, width_px, height_px)`.
pub fn thumb(project: &Project, template: &Template, lut: &[u8], max_px: u32) -> (Vec<u8>, u32, u32) {
	let (src, sw, sh) = compose(project, template, lut);
	let span = sw.max(sh);
	if span <= max_px {
		return (src, sw, sh);
	}
	let tw = (sw * max_px / span).max(1);
	let th = (sh * max_px / span).max(1);
	let mut out = vec![0u8; (tw as usize) * (th as usize) * 4];
	for y in 0..th as usize {
		let sy = y * sh as usize / th as usize;
		for x in 0..tw as usize {
			let sx = x * sw as usize / tw as usize;
			let s = (sy * sw as usize + sx) * 4;
			let d = (y * tw as usize + x) * 4;
			out[d..d + 4].copy_from_slice(&src[s..s + 4]);
		}
	}
	(out, tw, th)
}
