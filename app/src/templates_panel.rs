//! Templates Explorer dockable: a picker-style grid of tile templates -
//! shipped stock ones and the user's own - rendered as live thumbnails
//! through the tile pass. Clicking a template arms it as the ghost stamp
//! under the cursor. The two-row header holds Save (selection → template),
//! Import, Delete, Dups (remove exact duplicates), Rename, and a preview-size
//! dropdown. Pure geometry/draw - the shell routes clicks to commands
//! (`template-pick` / `template-save` / `template-delete` / `template-rename`
//! / `template-dedupe` / file dialog) or to the preview-size view state.

use map_core::Project;

use crate::picker::{self, TileQuad};
use crate::state::TemplateEntry;
use crate::theme;
use crate::ui::{Hot, Rect, SteelMap, UiQuads};

/// Height of one header row of controls.
const HEADER_ROW: f32 = 22.0;
const PAD: f32 = 4.0;
const GAP: f32 = 4.0;
/// Gap/margin between header controls.
const CTRL_GAP: f32 = 4.0;
const CTRL_MARGIN: f32 = 2.0;
/// Width reserved at the top-right of the first header row for the count.
const COUNT_W: f32 = 26.0;
/// The name strip under each thumbnail.
const NAME_H: f32 = 14.0;

/// Thumbnail sizes the preview-size dropdown offers, with their labels (very
/// small 32 .. very large 128). The shell stores the chosen px in
/// `EditorState::templates_cell`.
pub const PREVIEW_SIZES: [(f32, &str); 5] =
	[(32.0, "very small"), (48.0, "small"), (64.0, "medium"), (96.0, "large"), (128.0, "very large")];

/// The label for a cell size (falls back to the px when off-grid).
fn size_label(cell: f32) -> String {
	PREVIEW_SIZES
		.iter()
		.find(|&&(px, _)| px == cell)
		.map(|&(_, n)| n.to_string())
		.unwrap_or_else(|| format!("{cell:.0} px"))
}

/// What a click in the panel resolved to. `Pick` carries the index into the
/// **visible** (pack-compatible) list the view was built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
	Pick(usize),
	Save,
	Import,
	Delete,
	/// Remove duplicates (opens the modal).
	Dedupe,
	/// Rename the selected template (opens the modal).
	Rename,
	/// Open the user-templates folder in the OS file manager.
	Explore,
	/// Toggle the preview-size dropdown open/closed.
	SizeBox,
	/// Pick preview-size option `i` (index into [`PREVIEW_SIZES`]).
	SizeOption(usize),
	/// A click off an open size dropdown - close it (eats the click).
	SizeClose,
}

/// One header control, in flow order. `Size` is the preview-size dropdown box;
/// the rest are command buttons.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ctrl {
	Save,
	Import,
	Delete,
	Duplicates,
	Rename,
	Explore,
	Size,
}

/// Header controls, left to right. They flow onto one row when there's width
/// and wrap to further rows only when needed.
const CONTROLS: [Ctrl; 7] =
	[Ctrl::Save, Ctrl::Import, Ctrl::Delete, Ctrl::Duplicates, Ctrl::Rename, Ctrl::Explore, Ctrl::Size];

impl Ctrl {
	fn width(self) -> f32 {
		match self {
			Ctrl::Save => 46.0,
			Ctrl::Import => 54.0,
			Ctrl::Delete => 52.0,
			Ctrl::Duplicates => 82.0,
			Ctrl::Rename => 58.0,
			Ctrl::Explore => 58.0,
			Ctrl::Size => 96.0,
		}
	}

	fn label(self) -> &'static str {
		match self {
			Ctrl::Save => "save",
			Ctrl::Import => "import",
			Ctrl::Delete => "delete",
			Ctrl::Duplicates => "duplicates",
			Ctrl::Rename => "rename",
			Ctrl::Explore => "explore",
			Ctrl::Size => "",
		}
	}

	/// The command-button action (the size dropdown is handled separately).
	fn action(self) -> Option<Action> {
		match self {
			Ctrl::Save => Some(Action::Save),
			Ctrl::Import => Some(Action::Import),
			Ctrl::Delete => Some(Action::Delete),
			Ctrl::Duplicates => Some(Action::Dedupe),
			Ctrl::Rename => Some(Action::Rename),
			Ctrl::Explore => Some(Action::Explore),
			Ctrl::Size => None,
		}
	}
}

/// Flow the header controls into rows within `body`'s width (wrap only when the
/// next control won't fit), calling `emit(ctrl, rect)` for each, and return the
/// total header height. The first row reserves [`COUNT_W`] on the right for the
/// count. Shared by [`header_layout`] (collects the rects) and [`header_height`]
/// (height only, no allocation - `emit` is a no-op).
fn flow_header(body: Rect, mut emit: impl FnMut(Ctrl, Rect)) -> f32 {
	let left = body.x + CTRL_MARGIN;
	let top = body.y + 2.0;
	let h = HEADER_ROW - 4.0;
	let mut x = left;
	let mut y = top;
	for &c in &CONTROLS {
		let cw = c.width();
		let row_right = body.x + body.w - CTRL_MARGIN - if y == top { COUNT_W } else { 0.0 };
		if x + cw > row_right && x > left {
			x = left;
			y += HEADER_ROW;
		}
		emit(c, Rect::new(x, y, cw, h));
		x += cw + CTRL_GAP;
	}
	(y - body.y) + HEADER_ROW
}

/// The header controls' rects and the total header height.
fn header_layout(body: Rect) -> (Vec<(Ctrl, Rect)>, f32) {
	let mut out = Vec::with_capacity(CONTROLS.len());
	let h = flow_header(body, |c, r| out.push((c, r)));
	(out, h)
}

/// Just the flowed header height (no allocation) - the grid sits below it.
fn header_height(body: Rect) -> f32 {
	flow_header(body, |_, _| {})
}

fn grid(body: Rect, cell: f32) -> crate::cellgrid::Grid {
	crate::cellgrid::Grid { body, cell, gap: GAP, pad: PAD, header: header_height(body), row_extra: NAME_H }
}

/// Grid slot `i`'s thumbnail box (the name strip hangs below it). `header_h`
/// is the flowed header height the grid starts below.
fn item_rect(i: usize, body: Rect, cell: f32, header_h: f32, scroll: f32) -> Rect {
	crate::cellgrid::Grid { body, cell, gap: GAP, pad: PAD, header: header_h, row_extra: NAME_H }.item_rect(i, scroll)
}

pub fn max_scroll(count: usize, body: Rect, cell: f32) -> f32 {
	grid(body, cell).max_scroll(count)
}

pub fn scissor(body: Rect) -> Rect {
	crate::cellgrid::scissor(body, header_height(body))
}

/// Hit-test a click. `count` is the visible-template count the view drew;
/// `cell` the thumbnail size; `size_open` the preview-size dropdown state.
pub fn click(count: usize, body: Rect, cell: f32, size_open: bool, scroll: f32, x: f32, y: f32) -> Option<Action> {
	let (controls, header_h) = header_layout(body);
	// The size dropdown floats over the grid while open - resolve it first.
	if let Some(size) = controls.iter().find(|(c, _)| *c == Ctrl::Size).map(|(_, r)| *r) {
		match crate::select::hit(size, size_open, PREVIEW_SIZES.len(), false, x, y) {
			Some(crate::select::Hit::Box) => return Some(Action::SizeBox),
			Some(crate::select::Hit::Option(i)) => return Some(Action::SizeOption(i)),
			None if size_open => return Some(Action::SizeClose),
			None => {}
		}
	}
	for (c, r) in &controls {
		if let Some(action) = c.action() {
			if r.contains(x, y) {
				return Some(action);
			}
		}
	}
	if y < body.y + header_h {
		return None;
	}
	(0..count)
		.find(|&i| {
			let r = item_rect(i, body, cell, header_h, scroll);
			// The name strip is part of the hit area.
			Rect::new(r.x, r.y, r.w, r.h + NAME_H).contains(x, y)
		})
		.map(Action::Pick)
}

pub struct View {
	/// Header strip + buttons + size box + count - drawn unclipped (the header
	/// sits above the grid scissor).
	pub header: UiQuads,
	/// Thumbnail wells - drawn *under* the tile quads, clipped to [`Self::scissor`]
	/// so scrolled/partial thumbnails never spill past the grid.
	pub wells: UiQuads,
	pub tiles: Vec<TileQuad>,
	/// Rings, names, scrollbar - drawn *over* the tile quads, clipped to the grid.
	pub labels: UiQuads,
	/// The open preview-size option list - drawn unclipped, on top.
	pub popup: UiQuads,
	pub scissor: Rect,
}

/// Build the explorer: header (buttons + count), then one thumbnail per
/// visible template - its cells drawn through the tile pass, scaled to fit
/// the box, name fitted below (stock entries dim, they can't be deleted).
#[allow(clippy::too_many_arguments)]
pub fn view(
	project: &Project,
	entries: &[&TemplateEntry],
	selected: Option<usize>,
	cell: f32,
	size_open: bool,
	scroll: f32,
	body: Rect,
	w: f32,
	h: f32,
	map: SteelMap,
	hot: Hot,
) -> View {
	let clip = scissor(body);
	let (controls, header_h) = header_layout(body);
	let mut header = UiQuads::with_steel_map(map);
	let mut wells = UiQuads::with_steel_map(map);
	let mut labels = UiQuads::with_steel_map(map);
	let mut popup = UiQuads::with_steel_map(map);
	let mut tiles = Vec::new();
	let scroll = scroll.clamp(0.0, max_scroll(entries.len(), body, cell));

	// Header: a steel strip whose controls flow onto as many rows as they need.
	header.material(body.strip_top(header_h), w, h, theme::TITLE);
	let size_idx = PREVIEW_SIZES.iter().position(|&(px, _)| px == cell);
	let mut size_box = None;
	for &(c, r) in &controls {
		if c == Ctrl::Size {
			crate::select::draw_box(&mut header, r, &size_label(cell), size_open, w, h, hot);
			size_box = Some(r);
		} else {
			header.button(r, w, h, hot);
			header.label_fit(c.label(), r, 6.0, crate::ui::FONT_SMALL, w, h, theme::INK);
		}
	}
	let count = format!("{}", entries.len());
	let cx = body.x + body.w - 6.0 - crate::text::label_width(&count, crate::ui::FONT_SMALL);
	header.label(&count, cx, body.y + 4.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
	// The size dropdown's option list floats over the grid (drawn last, on top).
	if let (Some(size), true) = (size_box, size_open) {
		let opts: Vec<&str> = PREVIEW_SIZES.iter().map(|&(_, name)| name).collect();
		crate::select::draw_popup(&mut popup, size, &opts, size_idx, false, w, h, hot);
	}

	if entries.is_empty() {
		// The "no templates" note lives in the clipped grid layer so it can't
		// spill past a short panel.
		wells.label_wrapped(
			"no templates match this map's tile packs - select tiles and press save",
			Rect::new(clip.x, clip.y + 4.0, clip.w, clip.h),
			PAD,
			crate::ui::FONT_SMALL,
			w,
			h,
			theme::INK_DIM,
		);
		return View { header, wells, tiles, labels, popup, scissor: clip };
	}

	for (i, entry) in entries.iter().enumerate() {
		let r = item_rect(i, body, cell, header_h, scroll);
		if r.y + r.h + NAME_H < clip.y || r.y > clip.y + clip.h {
			continue;
		}
		// The thumbnail well (under the tiles) + the cells scaled into it. Both
		// the well and the tiles are clipped to the grid scissor by the shell.
		wells.field(r, w, h);
		let t = &entry.template;
		let span = t.width.max(t.height).max(1) as f32;
		let px = (cell - 4.0) / span;
		let (ox, oy) = (
			r.x + 2.0 + (cell - 4.0 - t.width as f32 * px) / 2.0,
			r.y + 2.0 + (cell - 4.0 - t.height as f32 * px) / 2.0,
		);
		for dy in 0..t.height {
			for dx in 0..t.width {
				for tile in t.cell_layers(project, dx, dy).into_iter().flatten() {
					tiles.push(TileQuad {
						index: picker::global_index(project, tile),
						transform: tile.transform.bits(),
						rect: Rect::new(ox + dx as f32 * px, oy + dy as f32 * px, px, px),
					});
				}
			}
		}
		// Selection / hover ring + name (clipped to the grid).
		let ring = Rect::new(r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0);
		if selected == Some(i) {
			labels.border(ring, w, h, theme::ACCENT);
		} else if hot.hover(Rect::new(r.x, r.y, r.w, r.h + NAME_H)) {
			labels.border(ring, w, h, theme::INK_DIM);
		}
		let ink = if entry.stock { theme::INK_DIM } else { theme::INK };
		let name_rect = Rect::new(r.x, r.y + r.h, r.w, NAME_H);
		labels.label_fit(&entry.name, name_rect, 1.0, crate::ui::FONT_SMALL, w, h, ink);
		// A small WxH badge in the thumbnail's top-right - only when the preview
		// is medium or larger, so it stays legible (and has room).
		if cell >= 64.0 {
			let dims = format!("{}x{}", t.width, t.height);
			let tw = crate::text::label_width(&dims, crate::ui::FONT_SMALL);
			labels.label_emboss(&dims, r.x + r.w - tw - 3.0, r.y + 3.0, crate::ui::FONT_SMALL, w, h, theme::INK);
		}
	}

	labels.scrollbar(clip, max_scroll(entries.len(), body, cell) + clip.h, scroll, w, h, hot);
	View { header, wells, tiles, labels, popup, scissor: clip }
}

#[cfg(test)]
mod tests {
	use super::*;

	const CELL: f32 = 64.0;

	/// The flowed rect of a header control (for hit-test assertions).
	fn ctrl_rect(body: Rect, c: Ctrl) -> Rect {
		header_layout(body).0.iter().find(|(k, _)| *k == c).map(|(_, r)| *r).unwrap()
	}

	#[test]
	fn header_and_grid_hits_resolve() {
		let body = Rect::new(100.0, 50.0, 280.0, 400.0);
		let header_h = header_height(body);
		let hit = |x: f32, y: f32| click(5, body, CELL, false, 0.0, x, y);
		for (c, action) in [
			(Ctrl::Save, Action::Save),
			(Ctrl::Import, Action::Import),
			(Ctrl::Delete, Action::Delete),
			(Ctrl::Duplicates, Action::Dedupe),
			(Ctrl::Rename, Action::Rename),
			(Ctrl::Explore, Action::Explore),
		] {
			let r = ctrl_rect(body, c);
			assert_eq!(hit(r.x + 2.0, r.y + 2.0), Some(action), "{}", c.label());
		}
		// Grid slots hit by their thumbnail and their name strip.
		for i in 0..5 {
			let r = item_rect(i, body, CELL, header_h, 0.0);
			assert_eq!(hit(r.x + 2.0, r.y + 2.0), Some(Action::Pick(i)), "thumb {i}");
			assert_eq!(hit(r.x + 2.0, r.y + r.h + 2.0), Some(Action::Pick(i)), "name {i}");
		}
		// Past the list: nothing.
		let r = item_rect(5, body, CELL, header_h, 0.0);
		assert_eq!(hit(r.x + 2.0, r.y + 2.0), None);
	}

	#[test]
	fn header_flows_to_one_row_when_wide_and_wraps_when_narrow() {
		// A wide dock keeps every control (and the count) on one row.
		let wide = header_layout(Rect::new(0.0, 0.0, 700.0, 400.0)).0;
		let row0 = wide[0].1.y;
		assert!(wide.iter().all(|(_, r)| r.y == row0), "all on one row when wide");
		// A narrow dock wraps onto more than one row.
		let narrow = header_layout(Rect::new(0.0, 0.0, 180.0, 400.0)).0;
		assert!(narrow.iter().any(|(_, r)| r.y > narrow[0].1.y), "wraps when narrow");
	}

	#[test]
	fn size_dropdown_toggles_then_picks() {
		let body = Rect::new(100.0, 50.0, 280.0, 400.0);
		let size = ctrl_rect(body, Ctrl::Size);
		let header_h = header_height(body);
		// Closed: the box toggles; its options are inert.
		assert_eq!(click(5, body, CELL, false, 0.0, size.x + 2.0, size.y + 2.0), Some(Action::SizeBox));
		// Open: each option row resolves to its index; a click off the list closes.
		for i in 0..PREVIEW_SIZES.len() {
			let o = crate::select::option_rect(size, i, PREVIEW_SIZES.len(), false);
			assert_eq!(click(5, body, CELL, true, 0.0, o.x + 2.0, o.y + 2.0), Some(Action::SizeOption(i)));
		}
		// A grid click while open just closes the dropdown.
		let r = item_rect(0, body, CELL, header_h, 0.0);
		assert_eq!(click(5, body, CELL, true, 0.0, r.x + 2.0, r.y + 2.0), Some(Action::SizeClose));
	}

	#[test]
	fn scroll_clamps_to_content() {
		let body = Rect::new(0.0, 0.0, 200.0, 200.0);
		assert_eq!(max_scroll(2, body, CELL), 0.0, "one row never scrolls");
		assert!(max_scroll(40, body, CELL) > 0.0, "many rows do");
	}
}
