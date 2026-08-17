//! Shared geometry for the scrolling thumbnail grids (Tile Picker, Units,
//! Templates Explorer): fixed-size cells flowing left-to-right and wrapping,
//! with a reserved scrollbar gutter.
//!
//! Each panel supplies its cell size, gutter and per-row name-strip extra; the
//! column count, cell rects and scroll range all come from here so the three
//! panels can't drift apart. A grid is the geometry of a **content widget
//! arranged into its own viewport**, so its clip is simply `body`. The gutter
//! is the theme's scrollbar metric, sampled at the widget's `arrange` (where
//! `ctx.theme` is in hand) — the same number the embedded `Scroller` paints
//! its bar with, so the reserved gutter and the drawn bar can't drift apart.

use crate::ui::Rect;

/// One scrolling cell grid laid out over a panel `body`.
pub struct Grid {
	/// The panel body the grid lives in.
	pub body: Rect,
	/// Cell (thumbnail) edge length.
	pub cell: f32,
	/// Gap between cells (and rows).
	pub gap: f32,
	/// Inner padding inside the body.
	pub pad: f32,
	/// The scrollbar gutter reserved on the right —
	/// `Theme::metrics().scrollbar`, sampled at `arrange`.
	pub gutter: f32,
	/// Extra height below each cell row (e.g. a name strip); `0.0` if none.
	pub row_extra: f32,
}

impl Grid {
	/// Columns that fit, reserving the scrollbar gutter (at least 1).
	pub fn cols(&self) -> usize {
		cols(self.body, self.cell, self.gap, self.pad, self.gutter)
	}

	/// Vertical distance between successive rows.
	fn row_pitch(&self) -> f32 {
		self.cell + self.row_extra + self.gap
	}

	/// Screen rect of cell `i` at the given scroll offset.
	pub fn item_rect(&self, i: usize, scroll: f32) -> Rect {
		let n = self.cols();
		let (row, col) = (i / n, i % n);
		Rect::new(
			self.body.x + self.pad + col as f32 * (self.cell + self.gap),
			self.body.y + self.pad - scroll + row as f32 * self.row_pitch(),
			self.cell,
			self.cell,
		)
	}

	/// Number of rows `count` items occupy.
	pub fn rows(&self, count: usize) -> usize {
		count.div_ceil(self.cols())
	}

	/// Scroll range so the last row can just reach the body bottom.
	pub fn max_scroll(&self, count: usize) -> f32 {
		let content = self.rows(count) as f32 * self.row_pitch() + 2.0 * self.pad - self.gap;
		crate::ui::scroll_max(content, self.body.h)
	}

	/// Grid content height - what a scrollbar over the grid measures its thumb
	/// against.
	pub fn content_height(&self, count: usize) -> f32 {
		self.rows(count) as f32 * self.row_pitch() + 2.0 * self.pad - self.gap
	}

	/// The flat (row-major) index a point falls in - the inverse of
	/// [`Self::item_rect`] - or `None` if it is left of / above the grid or past
	/// the last column. Doesn't check the item count or the cell interior; the
	/// caller confirms with `item_rect(i).contains(Vec2::new(x, y))`.
	pub fn index_at(&self, x: f32, y: f32, scroll: f32) -> Option<usize> {
		let cols = self.cols();
		let col = ((x - (self.body.x + self.pad)) / (self.cell + self.gap)).floor();
		let row = ((y - (self.body.y + self.pad) + scroll) / self.row_pitch()).floor();
		if col < 0.0 || row < 0.0 || col as usize >= cols {
			return None;
		}
		Some(row as usize * cols + col as usize)
	}
}

/// Columns that fit across `body` for `cell`-sized cells, reserving `gutter`
/// (the theme's scrollbar metric) on the right (at least 1 column). Standalone
/// so a caller can size columns without building a full [`Grid`].
pub fn cols(body: Rect, cell: f32, gap: f32, pad: f32, gutter: f32) -> usize {
	let inner = body.w - pad * 2.0 - gutter;
	(((inner + gap) / (cell + gap)).floor() as usize).max(1)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn index_at_inverts_item_rect_and_rejects_outside_points() {
		let g = Grid {
			body: Rect::new(10.0, 20.0, 300.0, 400.0),
			cell: 48.0,
			gap: 6.0,
			pad: 8.0,
			gutter: 8.0,
			row_extra: 12.0,
		};
		let cols = g.cols();
		assert!(cols >= 2, "a 300px body fits several 48px columns: {cols}");
		// The flat index round-trips through the drawn rect, scrolled or not.
		for i in [0usize, 1, cols - 1, cols, 3 * cols + 2] {
			let r = g.item_rect(i, 30.0);
			assert_eq!(g.index_at(r.x + 1.0, r.y + 1.0, 30.0), Some(i), "cell {i} round-trips");
		}
		// Left of the first column, in the top padding, past the last column: no cell.
		let r0 = g.item_rect(0, 0.0);
		assert_eq!(g.index_at(r0.x - 9.0, r0.y + 1.0, 0.0), None, "left of the grid");
		assert_eq!(g.index_at(r0.x + 1.0, g.body.y + 2.0, 0.0), None, "above: the top padding");
		let last = g.item_rect(cols - 1, 0.0);
		assert_eq!(g.index_at(last.x + last.w + g.gap + 1.0, r0.y + 1.0, 0.0), None, "the scrollbar gutter");
	}
}
