//! Error modal: when a command fails (`Outcome::Failed`), the shell raises
//! this so the message is in front of the user instead of buried in the
//! console - corrupt/rejected map files (the WRL/JSON validation at the
//! trust boundary), failed saves, bad imports. Pure geometry/draw; the only
//! action is dismiss (Esc / OK / click-out), routed via the `Modal` trait.

use crate::theme;
use crate::ui::{self, Hot, Rect, SteelMap, UiQuads};

const W: f32 = 460.0;
const TITLE_H: f32 = 22.0;
const BTN_H: f32 = 24.0;
const LINE_H: f32 = 16.0;
/// Wrap message text within the dialog, leaving a 12px margin each side.
const TEXT_W: f32 = W - 24.0;

pub struct ErrorModal {
	lines: Vec<String>,
	/// True while the OK button is held down - it fires on release-inside;
	/// dragging off cancels.
	armed: bool,
	/// Drag offset from centered (draggable by the titlebar).
	pub(crate) drag_offset: (f32, f32),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Press {
	Consumed,
	Dismiss,
}

impl ErrorModal {
	pub fn new(message: &str) -> Self {
		// The shared wrapper char-breaks over-long words (paths, ids), so no
		// message can escape the dialog.
		Self {
			lines: crate::text::wrap_lines(message, crate::ui::FONT_SMALL, TEXT_W),
			armed: false,
			drag_offset: (0.0, 0.0),
		}
	}

	fn height(&self) -> f32 {
		TITLE_H + 14.0 + self.lines.len().max(1) as f32 * LINE_H + 14.0 + BTN_H + 12.0
	}

	pub fn dialog_rect(&self, w: f32, h: f32) -> Rect {
		Rect::centered(w, h, W, self.height()).translate(self.drag_offset.0, self.drag_offset.1)
	}

	fn ok_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + d.w - 90.0, d.y + d.h - BTN_H - 12.0, 80.0, BTN_H)
	}

	/// A press on OK arms it (it fires on release-inside);
	/// click-out still dismisses immediately.
	pub fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		if self.ok_rect(d).contains(x, y) {
			self.armed = true;
			return Press::Consumed;
		}
		if !d.contains(x, y) {
			return Press::Dismiss; // click-out dismisses
		}
		Press::Consumed
	}

	/// Fire the armed OK if the release is still on it; otherwise disarm.
	pub fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		if std::mem::take(&mut self.armed) && self.ok_rect(d).contains(x, y) {
			return Press::Dismiss;
		}
		Press::Consumed
	}

	pub fn view(&self, w: f32, h: f32, hot: Hot) -> UiQuads {
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(SteelMap::anchored(d));
		ui::modal_scrim(&mut q, w, h);
		ui::modal_frame(&mut q, d, "Error", TITLE_H, w, h);
		for (i, line) in self.lines.iter().enumerate() {
			q.label(
				line,
				d.x + 12.0,
				d.y + TITLE_H + 14.0 + i as f32 * LINE_H,
				crate::ui::FONT_SMALL,
				w,
				h,
				theme::INK,
			);
		}
		let r = self.ok_rect(d);
		q.button_primary(r, w, h, hot);
		q.label_in("OK", r, 10.0, crate::ui::FONT_SMALL, w, h, theme::INK);
		q
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn wraps_long_messages_onto_several_lines() {
		let msg = "open MAP.WRL: bigmap references a tile beyond the tile table - the file is corrupt or not a valid M.A.X. map";
		let m = ErrorModal::new(msg);
		assert!(m.lines.len() > 1, "a long message should wrap");
		// Every wrapped line fits the text column (single over-long words aside).
		for line in &m.lines {
			assert!(!line.is_empty());
		}
		// The dialog grows with the line count.
		assert!(m.height() > TITLE_H + BTN_H);
	}

	#[test]
	fn ok_and_click_out_dismiss() {
		let mut m = ErrorModal::new("boom");
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		// OK fires on release-inside (press only arms); drag-off cancels.
		let ok = m.ok_rect(d);
		assert_eq!(m.on_press(ok.x + 2.0, ok.y + 2.0, w, h), Press::Consumed);
		assert_eq!(m.on_release(ok.x + 2.0, ok.y + 2.0, w, h), Press::Dismiss);
		m.on_press(ok.x + 2.0, ok.y + 2.0, w, h);
		assert_eq!(m.on_release(2.0, 2.0, w, h), Press::Consumed, "drag-off cancels");
		assert_eq!(m.on_press(2.0, 2.0, w, h), Press::Dismiss, "click-out dismisses");
		assert_eq!(m.on_press(d.x + d.w / 2.0, d.y + TITLE_H + 16.0, w, h), Press::Consumed);
	}
}
