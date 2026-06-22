//! Auto Fix Shore modal: a UI over the resumable
//! [`map_core::FixSession`] with three run modes and live stats. Fast caps
//! at ~1s; Aggressive runs until done or Stop and may permute adjacent land;
//! Destructive has total freedom over water/shore/land until the shore is
//! fixed. The session steps a bounded slice per frame (driven by the shell),
//! so the UI never freezes; the result is committed as one undo unit.
//!
//! Pure UI state here (plus the owned session); the shell drives `step` and
//! `apply` through `EditorState` so it can borrow the project.

use map_core::{FixSession, FixStrength};

use crate::theme;
use crate::ui::{self, Hot, Rect, UiQuads};

const W: f32 = 446.0;
const TITLE_H: f32 = 22.0;
const ROW: f32 = 20.0;
const BTN_H: f32 = 22.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixMode {
	/// ~1 second wall-clock cap, guaranteed non-freezing.
	Fast,
	/// Unbounded time, Stop button; bigger search - may also permute
	/// adjacent land when that closes a seam.
	Aggressive,
	/// Unbounded time, total freedom: water, shore, and land all rewritable
	/// until the shore is fixed.
	Destructive,
}

impl FixMode {
	pub fn strength(self) -> FixStrength {
		match self {
			FixMode::Fast => FixStrength::Shore,
			FixMode::Aggressive => FixStrength::Mangle,
			FixMode::Destructive => FixStrength::Destructive,
		}
	}
}

pub struct AutoFix {
	pub mode: FixMode,
	pub running: bool,
	/// The live session (created on Start; kept for stats afterwards).
	pub session: Option<FixSession>,
	pub found: usize,
	pub fixed: usize,
	pub remaining: usize,
	pub elapsed: f32,
	/// Cells changed once applied (set when a run finishes or is stopped).
	pub applied: Option<usize>,
	/// A command button held down, waiting for release-inside
	/// - dragging off cancels.
	armed: Option<ArmedBtn>,
	/// Drag offset from centered (draggable by the titlebar).
	pub(crate) drag_offset: (f32, f32),
}

/// The deferred command buttons (mode selection stays press-fired).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArmedBtn {
	Close,
	/// Start when idle / Stop while running - the same button.
	Start,
}

#[derive(Debug, PartialEq)]
pub enum Press {
	Consumed,
	Close,
	Start,
	Stop,
	SetMode(FixMode),
}

impl AutoFix {
	/// Open with the initial broken-seam count (a throwaway session counts
	/// it without running).
	pub fn new(found: usize) -> Self {
		Self {
			mode: FixMode::Fast,
			running: false,
			session: None,
			found,
			fixed: 0,
			remaining: found,
			elapsed: 0.0,
			applied: None,
			armed: None,
			drag_offset: (0.0, 0.0),
		}
	}

	// ----- geometry ----------------------------------------------------------

	fn height(&self) -> f32 {
		TITLE_H + 8.0 + ROW + 8.0 + 4.0 * ROW + 10.0 + BTN_H + 12.0
	}

	pub fn dialog_rect(&self, w: f32, h: f32) -> Rect {
		let dh = self.height();
		Rect::centered(w, h, W, dh).translate(self.drag_offset.0, self.drag_offset.1)
	}

	fn mode_rect(&self, d: Rect, i: usize) -> Rect {
		Rect::new(d.x + 60.0 + i as f32 * 126.0, d.y + TITLE_H + 8.0, 120.0, BTN_H)
	}

	fn start_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + d.w - 110.0, d.y + d.h - BTN_H - 10.0, 100.0, BTN_H)
	}

	fn close_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + 10.0, d.y + d.h - BTN_H - 10.0, 80.0, BTN_H)
	}

	// ----- events -------------------------------------------------------------

	pub fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		if !self.running {
			for (i, m) in [FixMode::Fast, FixMode::Aggressive, FixMode::Destructive].into_iter().enumerate() {
				if self.mode_rect(d, i).contains(x, y) {
					return Press::SetMode(m);
				}
			}
		}
		// Command buttons arm and fire on release-inside.
		if self.start_rect(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Start);
			return Press::Consumed;
		}
		if self.close_rect(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Close);
			return Press::Consumed;
		}
		if !d.contains(x, y) && !self.running {
			return Press::Close; // click-away closes when idle
		}
		Press::Consumed
	}

	/// Fire the armed command button if the release is still on it;
	/// a release anywhere else just disarms.
	pub fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		match self.armed.take() {
			Some(ArmedBtn::Start) if self.start_rect(d).contains(x, y) => {
				if self.running {
					Press::Stop
				} else {
					Press::Start
				}
			}
			Some(ArmedBtn::Close) if self.close_rect(d).contains(x, y) => Press::Close,
			_ => Press::Consumed,
		}
	}

	// ----- drawing -------------------------------------------------------------

	pub fn view(&self, w: f32, h: f32, hot: Hot) -> UiQuads {
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(ui::SteelMap::anchored(d));
		ui::modal_scrim(&mut q, w, h);
		ui::modal_frame(&mut q, d, "Auto Fix Shore", TITLE_H, w, h);

		// Mode buttons. Aggressive includes the old "permute adjacent land"
		// option; Destructive may rewrite water/shore/land outright.
		q.label("mode", d.x + 12.0, d.y + TITLE_H + 8.0 + 5.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		for (i, (m, label)) in
			[(FixMode::Fast, "Fast (1s)"), (FixMode::Aggressive, "Aggressive"), (FixMode::Destructive, "Destructive")]
				.into_iter()
				.enumerate()
		{
			let r = self.mode_rect(d, i);
			q.toggle_button(r, label, self.mode == m, !self.running, crate::ui::FONT_SMALL, w, h, hot);
		}

		// Live stats.
		let sy = d.y + TITLE_H + 8.0 + ROW + 8.0;
		let stat = |q: &mut UiQuads, i: usize, label: &str, value: String| {
			q.label(label, d.x + 12.0, sy + i as f32 * ROW, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
			q.label(&value, d.x + 130.0, sy + i as f32 * ROW, crate::ui::FONT_SMALL, w, h, theme::INK);
		};
		stat(&mut q, 0, "found bugs", self.found.to_string());
		stat(&mut q, 1, "fixed", self.fixed.to_string());
		stat(&mut q, 2, "remaining", self.remaining.to_string());
		// ASCII only - the MAX atlas has no em-dash (it would silently vanish).
		let elapsed =
			if self.running || self.applied.is_some() { format!("{:.1}s", self.elapsed) } else { "-".to_string() };
		stat(&mut q, 3, "elapsed", elapsed);
		if let Some(n) = self.applied {
			q.label(
				&format!("applied - {n} cells changed"),
				d.x + 12.0,
				sy + 4.0 * ROW - 2.0,
				crate::ui::FONT_SMALL,
				w,
				h,
				theme::INK_DIM,
			);
		}

		// Buttons.
		q.button(self.close_rect(d), w, h, hot);
		q.label_in("Close", self.close_rect(d), 8.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		let sr = self.start_rect(d);
		q.button_primary(sr, w, h, hot);
		let label = if self.running { "Stop" } else { "Start" };
		q.label_in(label, sr, 8.0, crate::ui::FONT_SMALL, w, h, theme::INK);
		q
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn press_selects_modes_and_starts() {
		let mut m = AutoFix::new(7);
		assert_eq!((m.found, m.remaining), (7, 7));
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		// Pick each mode by its button.
		for (i, mode) in [FixMode::Fast, FixMode::Aggressive, FixMode::Destructive].into_iter().enumerate() {
			let r = m.mode_rect(d, i);
			assert_eq!(m.on_press(r.x + 2.0, r.y + 2.0, w, h), Press::SetMode(mode));
		}
		// Start / Close fire on release-inside (press only arms).
		let s = m.start_rect(d);
		assert_eq!(m.on_press(s.x + 2.0, s.y + 2.0, w, h), Press::Consumed);
		assert_eq!(m.on_release(s.x + 2.0, s.y + 2.0, w, h), Press::Start);
		let c = m.close_rect(d);
		m.on_press(c.x + 2.0, c.y + 2.0, w, h);
		assert_eq!(m.on_release(c.x + 2.0, c.y + 2.0, w, h), Press::Close);
		// Dragging off before release cancels the click.
		m.on_press(s.x + 2.0, s.y + 2.0, w, h);
		assert_eq!(m.on_release(2.0, 2.0, w, h), Press::Consumed);
	}

	#[test]
	fn running_start_button_means_stop() {
		let mut m = AutoFix::new(3);
		m.running = true;
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		let s = m.start_rect(d);
		m.on_press(s.x + 2.0, s.y + 2.0, w, h);
		assert_eq!(m.on_release(s.x + 2.0, s.y + 2.0, w, h), Press::Stop);
		// While running, click-away does NOT close (must Stop first).
		assert_eq!(m.on_press(2.0, 2.0, w, h), Press::Consumed);
	}
}
