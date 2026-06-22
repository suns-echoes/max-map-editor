//! Shared geometry for the scrolling thumbnail grids (Tile Picker, Units,
//! Templates Explorer): a header row on top, then fixed-size cells flowing
//! left-to-right and wrapping, with a reserved scrollbar gutter.
//!
//! Each panel supplies its cell size, header height, and per-row name-strip
//! extra; the column count, cell rects, scroll range, and clip area all come
//! from here so the three panels can't drift apart.

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
	/// Header-row height the grid starts below (a constant, or a flowed height).
	pub header: f32,
	/// Extra height below each cell row (e.g. a name strip); `0.0` if none.
	pub row_extra: f32,
}

impl Grid {
	/// Columns that fit, reserving the scrollbar gutter (at least 1).
	pub fn cols(&self) -> usize {
		cols(self.body, self.cell, self.gap, self.pad)
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
			self.body.y + self.header + self.pad - scroll + row as f32 * self.row_pitch(),
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
		let content = self.rows(count) as f32 * self.row_pitch() + self.header + 2.0 * self.pad - self.gap;
		crate::ui::scroll_max(content, self.body.h)
	}

	/// Grid content height within the clip window (below the header) - what a
	/// scrollbar over the grid measures its thumb against.
	pub fn content_height(&self, count: usize) -> f32 {
		self.rows(count) as f32 * self.row_pitch() + 2.0 * self.pad - self.gap
	}

	/// The flat (row-major) index a point falls in - the inverse of
	/// [`Self::item_rect`] - or `None` if it is left of / above the grid or past
	/// the last column. Doesn't check the item count or the cell interior; the
	/// caller confirms with `item_rect(i).contains(x, y)`.
	pub fn index_at(&self, x: f32, y: f32, scroll: f32) -> Option<usize> {
		let cols = self.cols();
		let col = ((x - (self.body.x + self.pad)) / (self.cell + self.gap)).floor();
		let row = ((y - (self.body.y + self.header + self.pad) + scroll) / self.row_pitch()).floor();
		if col < 0.0 || row < 0.0 || col as usize >= cols {
			return None;
		}
		Some(row as usize * cols + col as usize)
	}
}

/// Columns that fit across `body` for `cell`-sized cells, reserving the
/// scrollbar gutter (at least 1). Standalone so a caller can size columns
/// without building a full [`Grid`] (the header isn't needed).
pub fn cols(body: Rect, cell: f32, gap: f32, pad: f32) -> usize {
	let inner = body.w - pad * 2.0 - crate::ui::SCROLLBAR_W;
	(((inner + gap) / (cell + gap)).floor() as usize).max(1)
}

/// The clip area below a `header`-tall row at the top of `body`.
pub fn scissor(body: Rect, header: f32) -> Rect {
	Rect::new(body.x, body.y + header, body.w, (body.h - header).max(0.0))
}
