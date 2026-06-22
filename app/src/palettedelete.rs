//! Confirm deleting a saved palette. Like the Delete Template modal, minus the
//! preview: it shows the name + an irreversible-action warning and, on confirm,
//! emits `palette-delete "<path>"`.

use std::path::PathBuf;

use crate::theme;
use crate::ui::{self, Hot, Rect, SteelMap, UiQuads};

const W: f32 = 340.0;
const TITLE_H: f32 = 22.0;
const BTN_H: f32 = 24.0;
const PAD: f32 = 12.0;
const LINE_H: f32 = 18.0;

pub struct PaletteDelete {
	name: String,
	path: PathBuf,
	/// Held button: 0 Cancel, 1 Delete - dragging off cancels the click.
	armed: Option<usize>,
	pub(crate) drag_offset: (f32, f32),
}

#[derive(Debug, PartialEq)]
pub enum Press {
	Consumed,
	Cancel,
	Delete,
}

impl PaletteDelete {
	pub fn new(name: &str, path: PathBuf) -> Self {
		Self { name: name.to_string(), path, armed: None, drag_offset: (0.0, 0.0) }
	}

	/// The resolved `palette-delete "<path>"` command line.
	pub fn command(&self) -> String {
		format!("palette-delete \"{}\"", self.path.display())
	}

	// ----- geometry -----------------------------------------------------------

	fn height() -> f32 {
		TITLE_H + PAD + 2.0 * LINE_H + PAD + BTN_H + PAD
	}

	pub fn dialog_rect(&self, w: f32, h: f32) -> Rect {
		Rect::centered(w, h, W, Self::height()).translate(self.drag_offset.0, self.drag_offset.1)
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
			return Press::Cancel; // click-out cancels
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
		ui::modal_frame(&mut q, d, "Delete Palette", TITLE_H, w, h);

		let y = d.y + TITLE_H + PAD;
		let prompt = format!("Delete \"{}\"?", self.name);
		q.label_fit(
			&prompt,
			Rect::new(d.x + PAD, y, W - 2.0 * PAD, LINE_H),
			0.0,
			crate::ui::FONT_SMALL,
			w,
			h,
			theme::INK,
		);
		q.label("This cannot be undone.", d.x + PAD, y + LINE_H, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);

		q.button(self.cancel_rect(d), w, h, hot);
		q.label_in("Cancel", self.cancel_rect(d), 8.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		q.button_primary(self.delete_rect(d), w, h, hot);
		q.label_in("Delete", self.delete_rect(d), 8.0, crate::ui::FONT_SMALL, w, h, theme::INK);
		q
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn delete_arms_and_fires_on_release_inside() {
		let mut m = PaletteDelete::new("swamp", PathBuf::from("/u/swamp.json"));
		assert_eq!(m.command(), "palette-delete \"/u/swamp.json\"");
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		let del = m.delete_rect(d);
		assert_eq!(m.on_press(del.x + 2.0, del.y + 2.0, w, h), Press::Consumed);
		assert_eq!(m.on_release(del.x + 2.0, del.y + 2.0, w, h), Press::Delete);
		// Drag-off cancels the click.
		m.on_press(del.x + 2.0, del.y + 2.0, w, h);
		assert_eq!(m.on_release(2.0, 2.0, w, h), Press::Consumed);
		// Click-out cancels.
		assert_eq!(m.on_press(2.0, 2.0, w, h), Press::Cancel);
	}
}
