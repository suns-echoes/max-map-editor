//! Bottom status bar: a context hint (by tool / editor mode) on the left, and
//! the cursor cell + selection size on the right. View-only - toggled by
//! View ▸ Status Bar. The shell reserves its strip via `Workspace::bottom` so
//! docked panels and the map never sit under it.

use crate::state::EditorState;
use crate::theme;
use crate::ui::{FONT_SMALL, Rect, UiQuads};

/// Status-bar height (px).
pub const BAR_H: f32 = 22.0;

/// Build the status-bar quads across the bottom of the viewport. `cursor_cell`
/// is the map cell under the pointer (`None` = off-map or over chrome).
pub fn draw(editor: &EditorState, cursor_cell: Option<(u16, u16)>, w: f32, h: f32) -> UiQuads {
	let mut q = UiQuads::default();
	let bar = Rect::new(0.0, h - BAR_H, w, BAR_H);
	q.material(bar, w, h, theme::TITLE);
	// Lit seam along the top edge (light from above).
	q.rect(Rect::new(0.0, bar.y, w, 1.0), w, h, theme::BEVEL.top);

	// Left: the context hint.
	q.label_in(editor.status_hint(), Rect::new(8.0, bar.y, w * 0.62, BAR_H), 0.0, FONT_SMALL, w, h, theme::INK);

	// Right: cursor cell, then selection size - right-aligned, space-separated.
	let text = right_text(cursor_cell, &editor.selection);
	if !text.is_empty() {
		let tw = crate::text::label_width(&text, FONT_SMALL);
		q.label_in(&text, Rect::new(w - tw - 8.0, bar.y, tw + 4.0, BAR_H), 0.0, FONT_SMALL, w, h, theme::INK_DIM);
	}
	q
}

/// The right-aligned status text: cursor cell + selection size, four-space
/// joined (empty when neither applies). Pure - the formatting the bar renders.
fn right_text(cursor_cell: Option<(u16, u16)>, selection: &map_core::Selection) -> String {
	let mut segs: Vec<String> = Vec::new();
	if let Some((cx, cy)) = cursor_cell {
		segs.push(format!("{cx}, {cy}"));
	}
	if let Some((x0, y0, x1, y1)) = selection.bounds() {
		segs.push(format!("selection {}x{} ({})", x1 - x0 + 1, y1 - y0 + 1, selection.count()));
	}
	segs.join("    ")
}

#[cfg(test)]
mod tests {
	use super::*;
	use map_core::{SelectMode, Selection};

	#[test]
	fn right_text_formats_cursor_and_selection() {
		let empty = Selection::new(8, 8);
		// Nothing to show.
		assert_eq!(right_text(None, &empty), "");
		// Cursor only.
		assert_eq!(right_text(Some((3, 7)), &empty), "3, 7");
		// Selection only: a 3×2 rect (1,1)..(3,2) → 6 cells.
		let mut sel = Selection::new(8, 8);
		sel.apply_rect(1, 1, 3, 2, SelectMode::Add);
		assert_eq!(right_text(None, &sel), "selection 3x2 (6)");
		// Both, four-space joined.
		assert_eq!(right_text(Some((3, 7)), &sel), "3, 7    selection 3x2 (6)");
	}
}
