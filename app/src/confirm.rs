//! Close-project confirmation modal: closing a
//! tab with unsaved changes asks **Save / Discard / Cancel** instead of just
//! refusing. Pure geometry/draw; the shell maps the buttons to commands
//! (`save-and-close` / `close-project!` / dismiss) via the `Modal` trait.

use crate::theme;
use crate::ui::{self, Hot, Rect, SteelMap, UiQuads};

const W: f32 = 380.0;
const TITLE_H: f32 = 22.0;
const BTN_H: f32 = 24.0;
const GAP: f32 = 8.0;

pub struct ConfirmClose {
	/// The project label shown in the prompt.
	name: String,
	/// The button held down, waiting for release-inside:
	/// 0 Cancel, 1 Discard, 2 Save — dragging off cancels.
	armed: Option<usize>,
	/// Drag offset from centered (draggable by the titlebar).
	pub(crate) drag_offset: (f32, f32),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Press {
	Consumed,
	Cancel,
	Discard,
	Save,
}

impl ConfirmClose {
	pub fn new(name: String) -> Self {
		Self { name, armed: None, drag_offset: (0.0, 0.0) }
	}

	fn height() -> f32 {
		TITLE_H + 16.0 + 18.0 + 16.0 + BTN_H + 12.0
	}

	pub fn dialog_rect(&self, w: f32, h: f32) -> Rect {
		Rect::centered(w, h, W, Self::height()).translate(self.drag_offset.0, self.drag_offset.1)
	}

	/// The three bottom buttons: 0 Cancel, 1 Discard, 2 Save (primary).
	fn btn(&self, d: Rect, i: usize) -> Rect {
		let bw = (d.w - 20.0 - 2.0 * GAP) / 3.0;
		Rect::new(d.x + 10.0 + i as f32 * (bw + GAP), d.y + d.h - BTN_H - 12.0, bw, BTN_H)
	}

	/// A press arms a button (it fires on release-inside);
	/// click-out still cancels immediately (the safe default).
	pub fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		for i in 0..3 {
			if self.btn(d, i).contains(x, y) {
				self.armed = Some(i);
				return Press::Consumed;
			}
		}
		if !d.contains(x, y) {
			return Press::Cancel; // click-out cancels (the safe default)
		}
		Press::Consumed
	}

	/// Fire the armed button if the release is still on it; a release
	/// anywhere else just disarms.
	pub fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		match self.armed.take() {
			Some(i) if self.btn(d, i).contains(x, y) => {
				[Press::Cancel, Press::Discard, Press::Save].into_iter().nth(i).unwrap_or(Press::Consumed)
			}
			_ => Press::Consumed,
		}
	}

	pub fn view(&self, w: f32, h: f32, hot: Hot) -> UiQuads {
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(SteelMap::anchored(d));
		ui::modal_scrim(&mut q, w, h);
		ui::modal_frame(&mut q, d, "Unsaved Changes", TITLE_H, w, h);
		// Long project names ellipsize inside the dialog.
		let prompt = crate::text::fit_label(
			&format!("\"{}\" has unsaved changes.", self.name),
			crate::ui::FONT_SMALL,
			d.w - 24.0,
		);
		q.label(&prompt, d.x + 12.0, d.y + TITLE_H + 14.0, crate::ui::FONT_SMALL, w, h, theme::INK);
		for (i, label) in ["Cancel", "Discard", "Save"].into_iter().enumerate() {
			let r = self.btn(d, i);
			if i == 2 {
				q.button_primary(r, w, h, hot);
			} else {
				q.button(r, w, h, hot);
			}
			let ink = if i == 0 { theme::INK_DIM } else { theme::INK };
			q.label_in(label, r, 10.0, crate::ui::FONT_SMALL, w, h, ink);
		}
		q
	}
}
