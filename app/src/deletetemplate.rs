//! Delete Template confirmation modal: a live thumbnail of the template about
//! to be removed, its name and footprint, and a Delete/Cancel pair. Delete
//! emits `template-delete!` (remove the selected user template); the same
//! preview chrome as the Rename modal, minus the editable field.
//!
//! Pure state/geometry. The thumbnail is drawn through the tile pass by the
//! shell. Template removal is not undoable, so the modal spells that out.

use map_core::{Project, Template};

use crate::picker::{self, TileQuad};
use crate::theme;
use crate::ui::{self, Hot, Rect, SteelMap, UiQuads};

const W: f32 = 360.0;
const TITLE_H: f32 = 22.0;
/// Thumbnail well side (matches the Rename modal's big preview).
const PREVIEW: f32 = 264.0;
const ROW_H: f32 = 22.0;
const LINE_H: f32 = 15.0;
const BTN_H: f32 = 24.0;
const PAD: f32 = 12.0;
const GAP: f32 = 10.0;
/// Left column for the row labels ("name", "size").
const LABEL_W: f32 = 44.0;
/// Breathing room between the preview and the first info row.
const PREVIEW_GAP: f32 = 16.0;
/// Gap between the name row and the size row.
const ROW_GAP: f32 = 6.0;

pub struct DeleteTemplate {
	/// The template's display name.
	name: String,
	/// A copy for the live preview.
	template: Template,
	/// Held button: 0 Cancel, 1 Delete - dragging off cancels the click.
	armed: Option<usize>,
	pub(crate) drag_offset: (f32, f32),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Press {
	Consumed,
	Cancel,
	Delete,
}

impl DeleteTemplate {
	pub fn new(name: &str, template: Template) -> Self {
		Self { name: name.to_string(), template, armed: None, drag_offset: (0.0, 0.0) }
	}

	// ----- geometry -----------------------------------------------------------

	fn height() -> f32 {
		// title | preview | gap | name | gap | size | gap | prompt | gap | buttons
		TITLE_H + PAD + PREVIEW + PREVIEW_GAP + ROW_H + ROW_GAP + ROW_H + GAP + LINE_H + GAP + BTN_H + PAD
	}

	pub fn dialog_rect(&self, w: f32, h: f32) -> Rect {
		Rect::centered(w, h, W, Self::height()).translate(self.drag_offset.0, self.drag_offset.1)
	}

	fn preview_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + (W - PREVIEW) / 2.0, d.y + TITLE_H + PAD, PREVIEW, PREVIEW)
	}

	fn name_row_y(&self, d: Rect) -> f32 {
		self.preview_rect(d).y + PREVIEW + PREVIEW_GAP
	}

	fn size_row_y(&self, d: Rect) -> f32 {
		self.name_row_y(d) + ROW_H + ROW_GAP
	}

	fn prompt_y(&self, d: Rect) -> f32 {
		self.size_row_y(d) + ROW_H + GAP
	}

	fn cancel_rect(&self, d: Rect) -> Rect {
		crate::ui::button_pair(d, W, PAD, BTN_H).0
	}

	fn delete_rect(&self, d: Rect) -> Rect {
		crate::ui::button_pair(d, W, PAD, BTN_H).1
	}

	// ----- events -------------------------------------------------------------

	pub fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		if self.cancel_rect(d).contains(x, y) {
			self.armed = Some(0);
			return Press::Consumed;
		}
		if self.delete_rect(d).contains(x, y) {
			self.armed = Some(1);
			return Press::Consumed;
		}
		if !d.contains(x, y) {
			return Press::Cancel; // click-out cancels (the safe default)
		}
		Press::Consumed
	}

	pub fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		match self.armed.take() {
			Some(0) if self.cancel_rect(d).contains(x, y) => Press::Cancel,
			Some(1) if self.delete_rect(d).contains(x, y) => Press::Delete,
			_ => Press::Consumed,
		}
	}

	// ----- drawing ------------------------------------------------------------

	pub fn view(&self, w: f32, h: f32, hot: Hot) -> UiQuads {
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(SteelMap::anchored(d));
		ui::modal_scrim(&mut q, w, h);
		ui::modal_frame(&mut q, d, "Delete Template", TITLE_H, w, h);

		// Preview well (tiles blit over it).
		q.field(self.preview_rect(d), w, h);

		// Read-only name + size rows, laid out like the Rename modal.
		let row_x = d.x + PAD + LABEL_W;
		let row_w = W - 2.0 * PAD - LABEL_W;
		let ny = self.name_row_y(d);
		q.label("name", d.x + PAD, ny + (ROW_H - 12.0) / 2.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		q.label_fit(&self.name, Rect::new(row_x, ny, row_w, ROW_H), 2.0, crate::ui::FONT_SMALL, w, h, theme::INK);
		let sy = self.size_row_y(d);
		q.label("size", d.x + PAD, sy + (ROW_H - 12.0) / 2.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		let dims = format!("{} x {}", self.template.width, self.template.height);
		q.label(&dims, row_x + 2.0, sy + (ROW_H - 12.0) / 2.0, crate::ui::FONT_SMALL, w, h, theme::INK);

		// Confirmation prompt (template removal is not undoable).
		q.label(
			"Delete this template? This cannot be undone.",
			d.x + PAD,
			self.prompt_y(d),
			crate::ui::FONT_SMALL,
			w,
			h,
			theme::INK_DIM,
		);

		q.button(self.cancel_rect(d), w, h, hot);
		q.label_in("Cancel", self.cancel_rect(d), 8.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		q.button_primary(self.delete_rect(d), w, h, hot);
		q.label_in("Delete", self.delete_rect(d), 8.0, crate::ui::FONT_SMALL, w, h, theme::INK);
		q
	}

	/// The template's cells as tile quads scaled into the preview well, plus the
	/// clip rect to scissor them to. (Same as the Rename modal's preview.)
	pub fn preview_tiles(&self, project: &Project, w: f32, h: f32) -> (Vec<TileQuad>, Rect) {
		let pr = self.preview_rect(self.dialog_rect(w, h));
		let t = &self.template;
		let span = t.width.max(t.height).max(1) as f32;
		let px = (PREVIEW - 8.0) / span;
		let (ox, oy) = (
			pr.x + 4.0 + (PREVIEW - 8.0 - t.width as f32 * px) / 2.0,
			pr.y + 4.0 + (PREVIEW - 8.0 - t.height as f32 * px) / 2.0,
		);
		let mut tiles = Vec::new();
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
		(tiles, pr)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn center(r: Rect) -> (f32, f32) {
		(r.x + r.w / 2.0, r.y + r.h / 2.0)
	}

	fn modal() -> DeleteTemplate {
		// A minimal 1×1 all-empty template (no packs needed for empty cells).
		let t =
			Template::from_str(r#"{"version":"1","name":"Ridge","width":1,"height":1,"use":[],"map":[[""]]}"#).unwrap();
		DeleteTemplate::new("Ridge", t)
	}

	#[test]
	fn delete_and_cancel_arm_then_fire_on_release_inside() {
		let (w, h) = (800.0, 600.0);
		let mut m = modal();
		let d = m.dialog_rect(w, h);
		// Delete (button 1): arms on press, fires on release-inside.
		let (dx, dy) = center(m.delete_rect(d));
		assert_eq!(m.on_press(dx, dy, w, h), Press::Consumed);
		assert_eq!(m.on_release(dx, dy, w, h), Press::Delete);
		// Cancel (button 0).
		let (cx, cy) = center(m.cancel_rect(d));
		assert_eq!(m.on_press(cx, cy, w, h), Press::Consumed);
		assert_eq!(m.on_release(cx, cy, w, h), Press::Cancel);
	}

	#[test]
	fn release_off_disarms_and_clickout_cancels() {
		let (w, h) = (800.0, 600.0);
		let mut m = modal();
		let d = m.dialog_rect(w, h);
		let (dx, dy) = center(m.delete_rect(d));
		m.on_press(dx, dy, w, h);
		// Release on the title area (inside the dialog, off the button) disarms.
		assert_eq!(m.on_release(d.x + 4.0, d.y + 4.0, w, h), Press::Consumed);
		assert_eq!(m.on_press(1.0, 1.0, w, h), Press::Cancel, "click-out cancels");
	}
}
