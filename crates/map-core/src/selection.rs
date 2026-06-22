//! Cell selection: a per-cell mask over the map with the set operations the
//! select tools need (replace/add/subtract, by cell or rectangle, plus
//! all/clear/invert/similar) and the boundary-edge walk that draws the thick
//! outline. Selection is **editor state, not document state** - it never
//! enters the undo journal; copy/cut/paste and template capture read it.

use crate::project::{LAYER_GROUND, Project};

/// How a select gesture combines with the existing selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectMode {
	/// Start fresh (no modifier).
	Replace,
	/// Add to the selection (Shift).
	Add,
	/// Subtract from the selection (Ctrl).
	Subtract,
}

impl SelectMode {
	pub fn parse(s: &str) -> Result<Self, String> {
		match s {
			"replace" => Ok(SelectMode::Replace),
			"add" => Ok(SelectMode::Add),
			"sub" => Ok(SelectMode::Subtract),
			other => Err(format!("bad select mode '{other}' (replace|add|sub)")),
		}
	}
}

/// One side of a cell, for the outline walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
	Top,
	Bottom,
	Left,
	Right,
}

/// The selected-cell mask. Sized to the map; resizing the map clears it
/// (the shell re-creates it on dimension changes).
#[derive(Clone)]
pub struct Selection {
	w: u16,
	h: u16,
	mask: Vec<bool>,
	count: usize,
}

impl Selection {
	pub fn new(w: u16, h: u16) -> Self {
		Self { w, h, mask: vec![false; w as usize * h as usize], count: 0 }
	}

	pub fn size(&self) -> (u16, u16) {
		(self.w, self.h)
	}

	/// Number of selected cells (0 = nothing selected).
	pub fn count(&self) -> usize {
		self.count
	}

	pub fn is_empty(&self) -> bool {
		self.count == 0
	}

	pub fn contains(&self, x: u16, y: u16) -> bool {
		x < self.w && y < self.h && self.mask[y as usize * self.w as usize + x as usize]
	}

	fn set(&mut self, x: u16, y: u16, on: bool) {
		if x >= self.w || y >= self.h {
			return;
		}
		let i = y as usize * self.w as usize + x as usize;
		if self.mask[i] != on {
			self.mask[i] = on;
			self.count = if on { self.count + 1 } else { self.count - 1 };
		}
	}

	/// Apply one cell of a select gesture. `Replace` only ever adds here -
	/// the gesture's first cell clears the selection (the tool routing calls
	/// [`Self::clear`] on press when the mode is Replace).
	pub fn apply_cell(&mut self, x: u16, y: u16, mode: SelectMode) {
		self.set(x, y, mode != SelectMode::Subtract);
	}

	/// Apply a rectangle (inclusive corners, any corner order).
	pub fn apply_rect(&mut self, x0: u16, y0: u16, x1: u16, y1: u16, mode: SelectMode) {
		let (xa, xb) = (x0.min(x1), x0.max(x1).min(self.w.saturating_sub(1)));
		let (ya, yb) = (y0.min(y1), y0.max(y1).min(self.h.saturating_sub(1)));
		for y in ya..=yb {
			for x in xa..=xb {
				self.set(x, y, mode != SelectMode::Subtract);
			}
		}
	}

	pub fn clear(&mut self) {
		self.mask.fill(false);
		self.count = 0;
	}

	/// Shift every selected cell by `(dx, dy)`; cells pushed off the map are
	/// dropped. Returns whether the mask changed. The marquee-move primitive
	/// (Alt+drag) - selection only, never the document.
	pub fn translate(&mut self, dx: i32, dy: i32) -> bool {
		if (dx == 0 && dy == 0) || self.count == 0 {
			return false;
		}
		let (w, h) = (self.w as i32, self.h as i32);
		let mut moved = vec![false; self.mask.len()];
		let mut count = 0;
		for y in 0..h {
			for x in 0..w {
				if !self.mask[(y * w + x) as usize] {
					continue;
				}
				let (nx, ny) = (x + dx, y + dy);
				if (0..w).contains(&nx) && (0..h).contains(&ny) {
					moved[(ny * w + nx) as usize] = true;
					count += 1;
				}
			}
		}
		self.mask = moved;
		self.count = count;
		true
	}

	pub fn select_all(&mut self) {
		self.mask.fill(true);
		self.count = self.mask.len();
	}

	pub fn invert(&mut self) {
		for m in &mut self.mask {
			*m = !*m;
		}
		self.count = self.mask.len() - self.count;
	}

	/// Select every cell whose ground tile matches one already under the
	/// selection; with nothing selected, match `fallback` (the active brush
	/// tile as `(pack, tile)`), ignoring transforms either way.
	pub fn select_similar(&mut self, project: &Project, fallback: Option<(u8, u16)>) {
		let keys: Vec<(u8, u16)> = if self.is_empty() {
			fallback.into_iter().collect()
		} else {
			let mut keys = Vec::new();
			for y in 0..self.h {
				for x in 0..self.w {
					if !self.contains(x, y) {
						continue;
					}
					if let Some(t) = project.cell(x, y).and_then(|s| s[LAYER_GROUND]) {
						if !keys.contains(&(t.pack, t.tile)) {
							keys.push((t.pack, t.tile));
						}
					}
				}
			}
			keys
		};
		if keys.is_empty() {
			return;
		}
		for y in 0..self.h.min(project.height) {
			for x in 0..self.w.min(project.width) {
				if let Some(t) = project.cell(x, y).and_then(|s| s[LAYER_GROUND]) {
					if keys.contains(&(t.pack, t.tile)) {
						self.set(x, y, true);
					}
				}
			}
		}
	}

	/// The selection's bounding box `(x0, y0, x1, y1)` (inclusive), or `None`
	/// when nothing is selected - the capture window for copy and templates.
	pub fn bounds(&self) -> Option<(u16, u16, u16, u16)> {
		if self.is_empty() {
			return None;
		}
		let (mut x0, mut y0, mut x1, mut y1) = (self.w, self.h, 0u16, 0u16);
		for y in 0..self.h {
			for x in 0..self.w {
				if self.contains(x, y) {
					x0 = x0.min(x);
					y0 = y0.min(y);
					x1 = x1.max(x);
					y1 = y1.max(y);
				}
			}
		}
		Some((x0, y0, x1, y1))
	}

	/// Every boundary edge - a selected cell's side whose neighbour is not
	/// selected - within the inclusive cell window `(x0, y0)..=(x1, y1)`.
	/// These segments form the thick outline around each selected region
	/// (regions need not be contiguous); the window keeps the walk
	/// viewport-sized however large the map is.
	pub fn boundary_edges(&self, x0: u16, y0: u16, x1: u16, y1: u16) -> Vec<(u16, u16, Edge)> {
		let mut out = Vec::new();
		for y in y0..=y1.min(self.h.saturating_sub(1)) {
			for x in x0..=x1.min(self.w.saturating_sub(1)) {
				if !self.contains(x, y) {
					continue;
				}
				if y == 0 || !self.contains(x, y - 1) {
					out.push((x, y, Edge::Top));
				}
				if y + 1 >= self.h || !self.contains(x, y + 1) {
					out.push((x, y, Edge::Bottom));
				}
				if x == 0 || !self.contains(x - 1, y) {
					out.push((x, y, Edge::Left));
				}
				if x + 1 >= self.w || !self.contains(x + 1, y) {
					out.push((x, y, Edge::Right));
				}
			}
		}
		out
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn cell_and_rect_modes_combine() {
		let mut s = Selection::new(8, 8);
		assert!(s.is_empty());
		s.apply_rect(1, 1, 3, 2, SelectMode::Add);
		assert_eq!(s.count(), 6);
		assert!(s.contains(1, 1) && s.contains(3, 2) && !s.contains(4, 2));
		// Corner order doesn't matter.
		s.apply_rect(5, 5, 4, 4, SelectMode::Add);
		assert_eq!(s.count(), 10);
		// Subtract carves.
		s.apply_rect(1, 1, 1, 2, SelectMode::Subtract);
		assert_eq!(s.count(), 8);
		assert!(!s.contains(1, 1));
		// Out-of-range cells are ignored, not panics.
		s.apply_cell(200, 200, SelectMode::Add);
		assert_eq!(s.count(), 8);
	}

	#[test]
	fn all_invert_clear_track_the_count() {
		let mut s = Selection::new(4, 4);
		s.select_all();
		assert_eq!(s.count(), 16);
		s.apply_cell(0, 0, SelectMode::Subtract);
		s.invert();
		assert_eq!(s.count(), 1);
		assert!(s.contains(0, 0));
		s.clear();
		assert!(s.is_empty());
		assert_eq!(s.bounds(), None);
	}

	#[test]
	fn translate_shifts_the_mask_and_drops_offmap_cells() {
		let mut s = Selection::new(6, 6);
		s.apply_rect(1, 1, 2, 2, SelectMode::Add); // 4 cells
		assert!(s.translate(2, 1));
		assert_eq!(s.bounds(), Some((3, 2, 4, 3)), "block moved +2,+1");
		assert_eq!(s.count(), 4);
		// Shoving it toward the right edge drops the columns that fall off
		// (cells at x=3,4 → x=5,6; the x=6 column is off a width-6 map).
		assert!(s.translate(2, 0));
		assert_eq!(s.count(), 2, "one column fell off the map");
		assert!(s.contains(5, 2) && s.contains(5, 3));
		// A no-op delta reports no change.
		assert!(!s.translate(0, 0));
	}

	#[test]
	fn bounds_wrap_disjoint_regions() {
		let mut s = Selection::new(10, 10);
		s.apply_cell(2, 3, SelectMode::Add);
		s.apply_cell(7, 8, SelectMode::Add);
		assert_eq!(s.bounds(), Some((2, 3, 7, 8)));
	}

	#[test]
	fn boundary_edges_outline_a_block_and_a_hole() {
		let mut s = Selection::new(6, 6);
		s.apply_rect(1, 1, 3, 3, SelectMode::Add);
		s.apply_cell(2, 2, SelectMode::Subtract); // a hole in the middle
		let edges = s.boundary_edges(0, 0, 5, 5);
		// Outer ring: 3 sides × 4 directions = 12; the hole adds 4 inner edges.
		assert_eq!(edges.len(), 16);
		// The hole's neighbours each expose one edge facing it.
		assert!(edges.contains(&(2, 1, Edge::Bottom)));
		assert!(edges.contains(&(2, 3, Edge::Top)));
		assert!(edges.contains(&(1, 2, Edge::Right)));
		assert!(edges.contains(&(3, 2, Edge::Left)));
		// A window sees only its slice.
		let windowed = s.boundary_edges(0, 0, 1, 5);
		assert!(windowed.iter().all(|&(x, ..)| x <= 1));
		assert!(!windowed.is_empty());
	}

	#[test]
	fn map_border_counts_as_outside() {
		let mut s = Selection::new(3, 3);
		s.select_all();
		// Every border cell exposes its map-edge sides.
		let edges = s.boundary_edges(0, 0, 2, 2);
		assert_eq!(edges.len(), 12, "3×3 fully selected = 12 perimeter edges");
	}

	#[test]
	fn select_similar_grows_by_ground_tile() {
		use crate::project::{TileRef, Transform};
		fn assets_root() -> std::path::PathBuf {
			std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/assets/tilepacks")
		}
		let mut p = Project::new(4, 4, &["GREEN".to_string()], &assets_root(), 1).unwrap();
		let green = 1u8; // pack 0 = WATER, 1 = GREEN
		let t = |tile| TileRef { pack: green, tile, transform: Transform::default() };
		// Ground tile 3 ("A") at three cells, tile 7 ("B") at two.
		p.place_many(&[
			(0, 0, LAYER_GROUND, Some(t(3))),
			(2, 0, LAYER_GROUND, Some(t(3))),
			(1, 1, LAYER_GROUND, Some(t(3))),
			(1, 0, LAYER_GROUND, Some(t(7))),
			(3, 3, LAYER_GROUND, Some(t(7))),
		]);

		// From one A cell, similar grows to every A cell, never the B cells.
		let mut sel = Selection::new(4, 4);
		sel.set(0, 0, true);
		sel.select_similar(&p, None);
		assert_eq!(sel.count(), 3, "all three tile-A cells selected");
		assert!(sel.contains(2, 0) && sel.contains(1, 1), "the other A cells joined");
		assert!(!sel.contains(1, 0) && !sel.contains(3, 3), "B cells excluded");

		// Empty selection + a fallback key (the active brush) selects by that key.
		let mut sel2 = Selection::new(4, 4);
		sel2.select_similar(&p, Some((green, 7)));
		assert_eq!(sel2.count(), 2, "both tile-B cells via fallback");
		assert!(sel2.contains(1, 0) && sel2.contains(3, 3));

		// Empty selection and no fallback is a no-op.
		let mut sel3 = Selection::new(4, 4);
		sel3.select_similar(&p, None);
		assert_eq!(sel3.count(), 0, "nothing to match -> no-op");
	}
}
