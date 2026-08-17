//! CPU-composes every pack tile into one RGBA atlas, through the **at-rest**
//! (uncycled) palette — the static tile art the retained panels show (the
//! Tile Explorer grid; the templates / toolbox previews follow). Only the map
//! canvas keeps the live indexed GPU pass with palette cycling; panel grids
//! are frozen stills by design.
//!
//! The atlas is registered once with the shared `wgpu-ui` renderer and
//! recomposed (via `replace_texture`, same id) whenever the document revision
//! or the stored palette changes — the shell compares both per frame.

use map_core::Project;
use wgpu_ui::TexRect;

/// Atlas cells per row (64px cells → a 1024px-wide texture).
pub const COLS: u32 = 16;
/// One tile side in pixels (the pack tile format).
pub const N: u32 = 64;

/// The 256-entry RGBA LUT of the at-rest palette: `rgb` is the project's
/// stored 256×3 palette; index 0 is transparent — the same contract the GPU
/// palette starts from (see `PaletteCycler::from_rgb`).
pub fn rest_lut(rgb: &[u8]) -> Vec<u8> {
	let mut lut = Vec::with_capacity(256 * 4);
	for (i, c) in rgb.chunks_exact(3).enumerate() {
		lut.extend_from_slice(&[c[0], c[1], c[2], if i == 0 { 0 } else { 255 }]);
	}
	lut
}

/// Composes every tile of every pack (global index order — the same
/// `sum of preceding packs' tile counts + tile` contract the GPU atlas uses)
/// into a [`COLS`]-wide RGBA atlas. Returns `(rgba, width_px, height_px,
/// tile_count)`; an empty project yields one blank cell so a texture can
/// always be registered.
pub fn compose(project: &Project, lut: &[u8]) -> (Vec<u8>, u32, u32, u32) {
	let count: u32 = project.packs.iter().map(|p| p.tile_count() as u32).sum();
	let rows = count.max(1).div_ceil(COLS);
	let (w, h) = (COLS * N, rows * N);
	let mut rgba = vec![0u8; (w as usize) * (h as usize) * 4];
	let mut i = 0u32;
	let n = N as usize;
	for pack in &project.packs {
		for t in 0..pack.tile_count() {
			let px = pack.tile_pixels(t);
			let (cx, cy) = (((i % COLS) * N) as usize, ((i / COLS) * N) as usize);
			for y in 0..n {
				for x in 0..n {
					let idx = px[y * n + x] as usize;
					let d = ((cy + y) * w as usize + cx + x) * 4;
					rgba[d..d + 4].copy_from_slice(&lut[idx * 4..idx * 4 + 4]);
				}
			}
			i += 1;
		}
	}
	(rgba, w, h, count)
}

/// Composes one tile — transform applied — into a 64×64 RGBA still (the
/// toolbox's active-tile preview). The water-layer slot copies the pixels
/// mask-free, matching what the GPU preview quad drew.
pub fn compose_tile(project: &Project, tile: map_core::TileRef, lut: &[u8]) -> Vec<u8> {
	let mut stack = [None; map_core::MAX_LAYERS];
	stack[map_core::LAYER_WATER] = Some(tile);
	let cell = project.compose_stack(&stack);
	let mut rgba = vec![0u8; cell.len() * 4];
	for (i, &idx) in cell.iter().enumerate() {
		let idx = idx as usize;
		rgba[i * 4..i * 4 + 4].copy_from_slice(&lut[idx * 4..idx * 4 + 4]);
	}
	rgba
}

/// The uv rect of global tile `index` in a `count`-tile atlas.
pub fn uv(index: u32, count: u32) -> TexRect {
	let rows = count.max(1).div_ceil(COLS);
	let (cx, cy) = ((index % COLS) as f32, (index / COLS) as f32);
	let (fw, fh) = (COLS as f32, rows as f32);
	TexRect::new(cx / fw, cy / fh, (cx + 1.0) / fw, (cy + 1.0) / fh)
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::Path;

	#[test]
	fn atlas_covers_every_tile_and_uv_addresses_cells() {
		let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources/assets/tilepacks");
		let project = Project::new(8, 6, &["GREEN".to_string()], &root, 42).unwrap();
		let lut = rest_lut(&project.palette);
		assert_eq!(lut.len(), 256 * 4);
		assert_eq!(lut[3], 0, "index 0 transparent");
		assert_eq!(lut[7], 255, "other entries opaque");

		let (rgba, w, h, count) = compose(&project, &lut);
		let total: u32 = project.packs.iter().map(|p| p.tile_count() as u32).sum();
		assert_eq!(count, total);
		assert_eq!(w, COLS * N);
		assert_eq!(h, count.div_ceil(COLS) * N);
		assert_eq!(rgba.len(), (w as usize) * (h as usize) * 4);
		// The composed art is not blank: some opaque, non-black pixels exist.
		let lit = rgba.chunks_exact(4).filter(|c| c[3] == 255 && (c[0] > 8 || c[1] > 8 || c[2] > 8)).count();
		assert!(lit > 1000, "atlas looks blank ({lit} lit px)");

		// uv: first cell starts at the origin; the cell after wraps rows at COLS.
		let r0 = uv(0, count);
		assert_eq!((r0.u0, r0.v0), (0.0, 0.0));
		let r_wrap = uv(COLS, count);
		assert_eq!(r_wrap.u0, 0.0, "row wrap returns to column 0");
		assert!(r_wrap.v0 > 0.0, "second row starts below the first");
	}
}
