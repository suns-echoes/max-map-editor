//! Unsaved-changes confirmation modal: closing a tab **or quitting the editor**
//! with unsaved work asks **Save / Discard / Cancel** instead of refusing or
//! losing it. One [`Purpose`] picks the prompt and the fired commands
//! (`save-and-close` / `close-project!` for a tab, `save-and-quit` / `quit!`
//! for the editor). Pure geometry/draw; the shell maps the buttons via the
//! `Modal` trait.

use crate::theme;
use crate::ui::{self, Hot, Rect, SteelMap, UiQuads};

const W: f32 = 380.0;
const TITLE_H: f32 = 22.0;
const BTN_H: f32 = 24.0;
const GAP: f32 = 8.0;

/// What the confirm guards: closing one tab, or quitting the whole editor.
/// Same Save/Discard/Cancel chrome; only the prompt and the fired commands
/// differ.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
	CloseTab,
	Quit,
}

pub struct ConfirmClose {
	/// The prompt text (a project label for a tab, or an unsaved-work summary
	/// for quit).
	prompt: String,
	purpose: Purpose,
	/// The button held down, waiting for release-inside:
	/// 0 Cancel, 1 Discard, 2 Save - dragging off cancels.
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
	/// Closing one tab: the prompt names the project.
	pub fn new(name: String) -> Self {
		Self {
			prompt: format!("\"{name}\" has unsaved changes."),
			purpose: Purpose::CloseTab,
			armed: None,
			drag_offset: (0.0, 0.0),
		}
	}

	/// Quitting the editor: the prompt summarizes the unsaved work.
	pub fn new_quit(summary: String) -> Self {
		Self { prompt: summary, purpose: Purpose::Quit, armed: None, drag_offset: (0.0, 0.0) }
	}

	/// The command the **Discard** button fires - drop changes and proceed.
	pub fn discard_line(&self) -> &'static str {
		match self.purpose {
			Purpose::CloseTab => "close-project!",
			Purpose::Quit => "quit!",
		}
	}

	/// The command the **Save** button (and Enter) fires - save, then proceed.
	pub fn save_line(&self) -> &'static str {
		match self.purpose {
			Purpose::CloseTab => "save-and-close",
			Purpose::Quit => "save-and-quit",
		}
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
		// Long prompts ellipsize inside the dialog.
		let prompt = crate::text::fit_label(&self.prompt, crate::ui::FONT_SMALL, d.w - 24.0);
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

#[cfg(test)]
mod tests {
	use super::*;

	fn center(r: Rect) -> (f32, f32) {
		(r.x + r.w / 2.0, r.y + r.h / 2.0)
	}

	#[test]
	fn purpose_picks_the_command_lines() {
		let tab = ConfirmClose::new("Map".into());
		assert_eq!((tab.discard_line(), tab.save_line()), ("close-project!", "save-and-close"));
		let quit = ConfirmClose::new_quit("3 tabs unsaved".into());
		assert_eq!((quit.discard_line(), quit.save_line()), ("quit!", "save-and-quit"));
	}

	#[test]
	fn arm_on_press_then_fire_on_release_inside() {
		let (w, h) = (800.0, 600.0);
		let mut m = ConfirmClose::new("Map".into());
		let d = m.dialog_rect(w, h);
		for (i, want) in [(0usize, Press::Cancel), (1, Press::Discard), (2, Press::Save)] {
			let (bx, by) = center(m.btn(d, i));
			assert_eq!(m.on_press(bx, by, w, h), Press::Consumed, "press arms button {i}");
			assert_eq!(m.on_release(bx, by, w, h), want, "release-inside fires button {i}");
		}
	}

	#[test]
	fn release_off_disarms_and_clickout_cancels() {
		let (w, h) = (800.0, 600.0);
		let mut m = ConfirmClose::new("Map".into());
		let d = m.dialog_rect(w, h);
		// Arm Save, then release elsewhere inside the dialog → disarmed no-op.
		let (sx, sy) = center(m.btn(d, 2));
		assert_eq!(m.on_press(sx, sy, w, h), Press::Consumed);
		assert_eq!(
			m.on_release(d.x + 4.0, d.y + TITLE_H + 4.0, w, h),
			Press::Consumed,
			"release off the button disarms"
		);
		// A press outside the dialog cancels immediately (the safe default).
		assert_eq!(m.on_press(1.0, 1.0, w, h), Press::Cancel);
	}
}
