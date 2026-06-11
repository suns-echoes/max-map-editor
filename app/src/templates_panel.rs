//! Templates Explorer dockable: a picker-style grid of tile templates —
//! shipped stock ones and the user's own — rendered as live thumbnails
//! through the tile pass. Clicking a template arms it as the ghost stamp
//! under the cursor; the header holds Save (selection → template), Import,
//! and Delete. Pure geometry/draw — the shell routes clicks to commands
//! (`template-pick` / `template-save` / `template-delete` / file dialog).

use map_core::Project;

use crate::picker::{self, TileQuad};
use crate::state::TemplateEntry;
use crate::theme;
use crate::ui::{Hot, Rect, SteelMap, UiQuads};

const HEADER_H: f32 = 22.0;
const PAD: f32 = 4.0;
const GAP: f32 = 4.0;
/// Thumbnail box side.
const CELL: f32 = 64.0;
/// The name strip under each thumbnail.
const NAME_H: f32 = 14.0;
pub const WHEEL_STEP: f32 = 48.0;

/// What a click in the panel resolved to. `Pick` carries the index into the
/// **visible** (pack-compatible) list the view was built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
	Pick(usize),
	Save,
	Import,
	Delete,
}

fn cols(body: Rect) -> usize {
	let inner = body.w - PAD * 2.0 - crate::ui::SCROLLBAR_W;
	(((inner + GAP) / (CELL + GAP)).floor() as usize).max(1)
}

/// Grid slot `i`'s thumbnail box (the name strip hangs below it).
fn item_rect(i: usize, body: Rect, scroll: f32) -> Rect {
	let n = cols(body);
	let (row, col) = (i / n, i % n);
	Rect::new(
		body.x + PAD + col as f32 * (CELL + GAP),
		body.y + HEADER_H + PAD - scroll + row as f32 * (CELL + NAME_H + GAP),
		CELL,
		CELL,
	)
}

pub fn max_scroll(count: usize, body: Rect) -> f32 {
	let rows = count.div_ceil(cols(body)).max(1);
	let content = rows as f32 * (CELL + NAME_H + GAP) + PAD * 2.0 - GAP;
	crate::ui::scroll_max(content, body.h - HEADER_H)
}

/// Header buttons: `[save]` `[import]` `[delete]`.
fn header_buttons(body: Rect) -> (Rect, Rect, Rect) {
	let h = HEADER_H - 4.0;
	(
		Rect::new(body.x + 2.0, body.y + 2.0, 52.0, h),
		Rect::new(body.x + 56.0, body.y + 2.0, 58.0, h),
		Rect::new(body.x + 116.0, body.y + 2.0, 56.0, h),
	)
}

pub fn scissor(body: Rect) -> Rect {
	Rect::new(body.x, body.y + HEADER_H, body.w, (body.h - HEADER_H).max(0.0))
}

/// Hit-test a click. `count` is the visible-template count the view drew.
pub fn click(count: usize, body: Rect, scroll: f32, x: f32, y: f32) -> Option<Action> {
	let (save, import, delete) = header_buttons(body);
	if save.contains(x, y) {
		return Some(Action::Save);
	}
	if import.contains(x, y) {
		return Some(Action::Import);
	}
	if delete.contains(x, y) {
		return Some(Action::Delete);
	}
	if y < body.y + HEADER_H {
		return None;
	}
	(0..count)
		.find(|&i| {
			let r = item_rect(i, body, scroll);
			// The name strip is part of the hit area.
			Rect::new(r.x, r.y, r.w, r.h + NAME_H).contains(x, y)
		})
		.map(Action::Pick)
}

pub struct View {
	/// Header strip + thumbnail wells — drawn *under* the tile quads.
	pub underlay: UiQuads,
	pub tiles: Vec<TileQuad>,
	/// Rings, names, scrollbar — drawn *over* the tile quads.
	pub overlay: UiQuads,
	pub scissor: Rect,
}

/// Build the explorer: header (buttons + count), then one thumbnail per
/// visible template — its cells drawn through the tile pass, scaled to fit
/// the box, name fitted below (stock entries dim, they can't be deleted).
#[allow(clippy::too_many_arguments)]
pub fn view(
	project: &Project,
	entries: &[&TemplateEntry],
	selected: Option<usize>,
	scroll: f32,
	body: Rect,
	w: f32,
	h: f32,
	map: SteelMap,
	hot: Hot,
) -> View {
	let clip = scissor(body);
	let mut underlay = UiQuads::with_steel_map(map);
	let mut overlay = UiQuads::with_steel_map(map);
	let mut tiles = Vec::new();
	let scroll = scroll.clamp(0.0, max_scroll(entries.len(), body));

	// Header: a steel strip with the three actions + the count. Scrolled
	// thumbnails clip below it (the scissor), so it can live in the underlay.
	underlay.material(body.strip_top(HEADER_H), w, h, theme::TITLE);
	let (save, import, delete) = header_buttons(body);
	for (r, label) in [(save, "save"), (import, "import"), (delete, "delete")] {
		underlay.button(r, w, h, hot);
		underlay.label_fit(label, r, 6.0, crate::ui::FONT_SMALL, w, h, theme::INK);
	}
	let count = format!("{}", entries.len());
	let cx = body.x + body.w - 6.0 - crate::text::label_width(&count, crate::ui::FONT_SMALL);
	underlay.label(&count, cx, body.y + 4.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);

	if entries.is_empty() {
		underlay.label_wrapped(
			"no templates match this map's tile packs - select tiles and press save",
			Rect::new(body.x, body.y + HEADER_H + 4.0, body.w, body.h - HEADER_H),
			PAD,
			crate::ui::FONT_SMALL,
			w,
			h,
			theme::INK_DIM,
		);
		return View { underlay, tiles, overlay, scissor: clip };
	}

	for (i, entry) in entries.iter().enumerate() {
		let r = item_rect(i, body, scroll);
		if r.y + r.h + NAME_H < clip.y || r.y > clip.y + clip.h {
			continue;
		}
		// The thumbnail well (under the tiles) + the cells scaled into it.
		underlay.field(r, w, h);
		let t = &entry.template;
		let span = t.width.max(t.height).max(1) as f32;
		let px = (CELL - 4.0) / span;
		let (ox, oy) = (
			r.x + 2.0 + (CELL - 4.0 - t.width as f32 * px) / 2.0,
			r.y + 2.0 + (CELL - 4.0 - t.height as f32 * px) / 2.0,
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
		// Selection / hover ring (only when fully below the header).
		let ring = Rect::new(r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0);
		if selected == Some(i) {
			overlay.border(ring, w, h, theme::ACCENT);
		} else if hot.hover(Rect::new(r.x, r.y, r.w, r.h + NAME_H)) && r.y >= clip.y {
			overlay.border(ring, w, h, theme::INK_DIM);
		}
		let ink = if entry.stock { theme::INK_DIM } else { theme::INK };
		let name_rect = Rect::new(r.x, r.y + r.h, r.w, NAME_H);
		overlay.label_fit(&entry.name, name_rect, 1.0, crate::ui::FONT_SMALL, w, h, ink);
	}

	overlay.scrollbar(clip, max_scroll(entries.len(), body) + clip.h, scroll, w, h, hot);
	View { underlay, tiles, overlay, scissor: clip }
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn header_and_grid_hits_resolve() {
		let body = Rect::new(100.0, 50.0, 280.0, 400.0);
		let (save, import, delete) = header_buttons(body);
		assert_eq!(click(5, body, 0.0, save.x + 2.0, save.y + 2.0), Some(Action::Save));
		assert_eq!(click(5, body, 0.0, import.x + 2.0, import.y + 2.0), Some(Action::Import));
		assert_eq!(click(5, body, 0.0, delete.x + 2.0, delete.y + 2.0), Some(Action::Delete));
		// Grid slots hit by their thumbnail and their name strip.
		for i in 0..5 {
			let r = item_rect(i, body, 0.0);
			assert_eq!(click(5, body, 0.0, r.x + 2.0, r.y + 2.0), Some(Action::Pick(i)), "thumb {i}");
			assert_eq!(click(5, body, 0.0, r.x + 2.0, r.y + r.h + 2.0), Some(Action::Pick(i)), "name {i}");
		}
		// Past the list: nothing.
		let r = item_rect(5, body, 0.0);
		assert_eq!(click(5, body, 0.0, r.x + 2.0, r.y + 2.0), None);
	}

	#[test]
	fn scroll_clamps_to_content() {
		let body = Rect::new(0.0, 0.0, 200.0, 200.0);
		assert_eq!(max_scroll(2, body), 0.0, "one row never scrolls");
		assert!(max_scroll(40, body) > 0.0, "many rows do");
	}
}
