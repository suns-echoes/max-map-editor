//! Bottom status bar: a context hint (by tool / editor mode) on the left, and
//! the cursor cell + selection size on the right. View-only - toggled by
//! View ▸ Status Bar. The shell reserves its strip via `Workspace::bottom` so
//! docked panels and the map never sit under it.
//!
//! Converted to a retained `wgpu_ui` widget tree (a `Linear` row of two
//! `Label`s) hosted by [`crate::panel_ui::PanelUi`]; the steel strip + lit seam
//! stay a `SteelTheme` material fill (chrome the theme owns). This is the pilot
//! for the per-panel conversion (see `WGPU-UI-CONVERSION-BACKLOG.md`).

use wgpu_ui::layout::{Linear, Spacer};
use wgpu_ui::widget::{CrossAlign, Length};
use wgpu_ui::{DrawList, Label, Theme, WidgetId};

use crate::panel_ui::PanelUi;
use crate::state::EditorState;
use crate::ui::Rect;
use crate::uikit_menu::MenuChrome;
use crate::uikit_theme::Edge;

/// Status-bar height (px).
pub const BAR_H: f32 = 22.0;

/// Horizontal inset of the text from the strip edges (px).
const PAD: f32 = 8.0;

/// The retained status bar: a hint label (left) + a readout label (right),
/// synced from editor state each frame.
pub struct StatusBar {
	panel: PanelUi,
	hint: WidgetId,
	readout: WidgetId,
}

impl Default for StatusBar {
	fn default() -> Self {
		Self::new()
	}
}

impl StatusBar {
	pub fn new() -> Self {
		let hint = Label::new("").small().with_id();
		let readout = Label::new("").small().with_id();
		let (h, r) = (hint.id(), readout.id());
		// Hint at the left; a flex spacer pushes the readout to the right edge.
		let row = Linear::row()
			.cross_align(CrossAlign::Center)
			.child(hint, Length::Fit)
			.child(Spacer::new(), Length::Flex(1.0))
			.child(readout, Length::Fit);
		Self { panel: PanelUi::new(row), hint: h, readout: r }
	}

	/// Build the status-bar `DrawList` across the bottom of the `wf`×`hf`
	/// (logical) viewport: the steel strip + lit top seam, then the two synced
	/// labels. `cursor_cell` is the map cell under the pointer (`None` = off-map).
	/// `hover_hint` is a hovered control's tooltip text, mirrored into the hint
	/// slot for as long as the hover holds — `None` (the norm) shows the tool
	/// hint, so leaving a key restores it by simple recomputation.
	pub fn build(
		&mut self,
		chrome: &MenuChrome,
		editor: &EditorState,
		hover_hint: Option<&str>,
		cursor_cell: Option<(u16, u16)>,
		wf: f32,
		hf: f32,
		scale: f32,
	) -> DrawList {
		// Sync the labels from editor state.
		if let Some(l) = self.panel.ui.get_mut::<Label>(self.hint) {
			l.set_text(hover_hint.unwrap_or_else(|| editor.status_hint()));
		}
		if let Some(l) = self.panel.ui.get_mut::<Label>(self.readout) {
			let tile_id = cursor_cell.and_then(|(cx, cy)| editor.hovered_tile_id(cx, cy));
			let cargo = cursor_cell.and_then(|(cx, cy)| editor.resource_readout(cx, cy));
			l.set_text(right_text(cursor_cell, tile_id, cargo.as_deref(), &editor.selection));
		}

		let bar = Rect::new(0.0, hf - BAR_H, wf, BAR_H);
		let mut dl = DrawList::new();
		// Steel strip (the theme's header band) + a lit seam along the top edge.
		chrome.theme().header_band(&mut dl, bar);
		chrome.theme().seam(&mut dl, bar, Edge::Top);
		// Labels inside a horizontal inset.
		let inner = Rect::new(bar.x + PAD, bar.y, (bar.w - 2.0 * PAD).max(0.0), bar.h);
		self.panel.build(chrome, inner, scale, &[], &mut dl, &mut DrawList::new());
		dl
	}
}

/// The right-aligned status text: cursor cell (1-based) + hovered tile id +
/// selection size, four-space joined (empty when none apply). Pure - the
/// formatting the bar renders. Coordinates display 1-based (the top-left cell
/// reads `1, 1`) though the model is 0-based.
fn right_text(
	cursor_cell: Option<(u16, u16)>,
	tile_id: Option<&str>,
	cargo: Option<&str>,
	selection: &map_core::Selection,
) -> String {
	let mut segs: Vec<String> = Vec::new();
	if let Some((cx, cy)) = cursor_cell {
		let mut seg = format!("{}, {}", cx as u32 + 1, cy as u32 + 1);
		if let Some(id) = tile_id {
			seg.push_str("  ");
			seg.push_str(id);
		}
		segs.push(seg);
	}
	// The resource at the hovered cell (only while in a resource mode, S5.4).
	if let Some(cargo) = cargo {
		segs.push(format!("res: {cargo}"));
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

	/// `Default` builds the same retained tree as `new()`: both label ids
	/// resolve to live `Label` widgets in the hosted `Ui`, ready to sync.
	#[test]
	fn default_builds_the_two_synced_labels() {
		let mut bar = StatusBar::default();
		let (hint, readout) = (bar.hint, bar.readout);
		assert_ne!(hint, readout, "hint and readout are distinct widgets");
		for (id, name) in [(hint, "hint"), (readout, "readout")] {
			let label = bar.panel.ui.get_mut::<Label>(id).unwrap_or_else(|| panic!("{name} label resolvable"));
			label.set_text("probe");
		}
	}

	#[test]
	fn right_text_formats_cursor_and_selection() {
		let empty = Selection::new(8, 8);
		// Nothing to show.
		assert_eq!(right_text(None, None, None, &empty), "");
		// Cursor only - displayed 1-based (model (3,7) → "4, 8").
		assert_eq!(right_text(Some((3, 7)), None, None, &empty), "4, 8");
		// Cursor + hovered tile id.
		assert_eq!(right_text(Some((3, 7)), Some("GLa000"), None, &empty), "4, 8  GLa000");
		// Cursor + a resource readout (S5.4): appended as its own segment.
		assert_eq!(right_text(Some((3, 7)), None, Some("fuel 16"), &empty), "4, 8    res: fuel 16");
		// Selection only: a 3×2 rect (1,1)..(3,2) → 6 cells.
		let mut sel = Selection::new(8, 8);
		sel.apply_rect(1, 1, 3, 2, SelectMode::Add);
		assert_eq!(right_text(None, None, None, &sel), "selection 3x2 (6)");
		// All three, four-space joined.
		assert_eq!(
			right_text(Some((3, 7)), Some("GLa000"), Some("raw 4"), &sel),
			"4, 8  GLa000    res: raw 4    selection 3x2 (6)"
		);
	}
}
