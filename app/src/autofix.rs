//! Fix Shore modal: a UI over the resumable [`map_core::FixSession`] with a
//! "fast -> fully accurate" method ladder. Every method first **lays the
//! missing coast** (an auto-shore pass), then - for the accurate tiers -
//! resolves the leftover broken/misplaced seams:
//!
//! * **Sweep** / **Loop-Walk** - place + greedy repair, instant (uniform vs
//!   varied coastline).
//! * **Aggressive** - place, then permute seams and adjacent land (Mangle)
//!   until done.
//! * **Destructive** - place, then reshape water/shore/land until the coast is
//!   100% clean.
//!
//! The fix tiers step a bounded slice per frame (driven by the shell), so the
//! UI never freezes; placement is instant. Pure UI state here (plus the owned
//! session); the shell drives `step`/`apply` through `EditorState` so it can
//! borrow the project.

use map_core::{FixSession, FixStrength};

use crate::select;
use crate::theme;
use crate::ui::{self, Hot, Rect, UiQuads};

const W: f32 = 446.0;
const TITLE_H: f32 = 22.0;
const ROW: f32 = 20.0;
const BTN_H: f32 = 22.0;

/// A point on the fix ladder, fast (top) -> fully accurate (bottom). Each lays
/// missing shore; the `*Fix` / `Destructive` tiers also resolve broken/misplaced
/// seams. Placement is sweep (uniform) or loop-walk (varied).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
	/// Sweep auto-shore: place + greedy repair, uniform coast. Instant.
	Sweep,
	/// Loop-walk auto-shore: place + repair, varied coast. Instant.
	LoopWalk,
	/// Sweep, then Mangle (permute seams + adjacent land) to completion.
	SweepFix,
	/// Loop-walk, then Mangle to completion (varied coast).
	LoopFix,
	/// Sweep, then Destructive (reshape water/shore/land) to a perfect coast.
	Destructive,
}

impl Method {
	pub const ALL: [Method; 5] =
		[Method::Sweep, Method::LoopWalk, Method::SweepFix, Method::LoopFix, Method::Destructive];

	pub fn label(self) -> &'static str {
		match self {
			Method::Sweep => "Sweep (fast)",
			Method::LoopWalk => "Loop-Walk (fast, varied)",
			Method::SweepFix => "Sweep + Fix (accurate)",
			Method::LoopFix => "Loop-Walk + Fix (varied)",
			Method::Destructive => "Destructive (full)",
		}
	}

	/// One-line description shown under the select.
	fn hint(self) -> &'static str {
		match self {
			Method::Sweep => "Lay + repair the coast in one pass - uniform, instant.",
			Method::LoopWalk => "Lay + repair the coast - varied coastline, instant.",
			Method::SweepFix => "Lay (sweep), then fix stubborn seams (may adjust nearby land).",
			Method::LoopFix => "Lay (loop-walk), then fix stubborn seams - varied coast.",
			Method::Destructive => "Lay, then reshape terrain until the coast is perfect.",
		}
	}

	/// Loop-walk placement instead of sweep.
	pub fn loop_walk(self) -> bool {
		matches!(self, Method::LoopWalk | Method::LoopFix)
	}

	/// The follow-up fix strength, or `None` for the placement-only (instant)
	/// tiers.
	pub fn fix_strength(self) -> Option<FixStrength> {
		match self {
			Method::Sweep | Method::LoopWalk => None,
			Method::SweepFix | Method::LoopFix => Some(FixStrength::Mangle),
			Method::Destructive => Some(FixStrength::Destructive),
		}
	}

	/// Parse the preset word a menu item / command passes (`sweep-fix`, ...).
	pub fn parse(s: &str) -> Option<Method> {
		Some(match s {
			"sweep" => Method::Sweep,
			"loop-walk" | "loop" => Method::LoopWalk,
			"sweep-fix" => Method::SweepFix,
			"loop-fix" => Method::LoopFix,
			"full" | "destructive" => Method::Destructive,
			_ => return None,
		})
	}
}

pub struct AutoFix {
	pub method: Method,
	/// The method select's popup open flag.
	pub select_open: bool,
	pub running: bool,
	/// The live session for the current pass (the fix tiers loop passes).
	pub session: Option<FixSession>,
	/// The strength of the current pass. Aggressive starts at `Mangle` and
	/// escalates to `Destructive` when re-tiling plateaus (mirrors `shore_repair`).
	pub cur_strength: FixStrength,
	/// Cumulative cells changed across every pass (placement + fixes).
	pub total_changed: usize,
	/// Faithful defect count after the first placement (the baseline).
	pub found: usize,
	pub fixed: usize,
	pub remaining: usize,
	/// Lowest defect count seen, passes completed, and stalled passes - the
	/// multi-pass loop's convergence bookkeeping.
	pub best: usize,
	pub passes: u32,
	pub stall: u32,
	pub elapsed: f32,
	/// Cells changed once applied (set when a run finishes or is stopped).
	pub applied: Option<usize>,
	/// A command button held down, waiting for release-inside
	/// - dragging off cancels.
	armed: Option<ArmedBtn>,
	/// Drag offset from centered (draggable by the titlebar).
	pub(crate) drag_offset: (f32, f32),
}

/// The deferred command buttons (the method select fires on press).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArmedBtn {
	Close,
	/// Start when idle / Stop while running - the same button.
	Start,
	Undo,
}

#[derive(Debug, PartialEq)]
pub enum Press {
	Consumed,
	Close,
	Start,
	Stop,
	Undo,
	SetMethod(Method),
}

impl AutoFix {
	/// Open with the initial broken-seam count (a throwaway session counts it
	/// without running).
	pub fn new(found: usize) -> Self {
		Self {
			method: Method::SweepFix,
			select_open: false,
			running: false,
			session: None,
			cur_strength: FixStrength::Mangle,
			total_changed: 0,
			found,
			fixed: 0,
			remaining: found,
			best: usize::MAX,
			passes: 0,
			stall: 0,
			elapsed: 0.0,
			applied: None,
			armed: None,
			drag_offset: (0.0, 0.0),
		}
	}

	// ----- geometry ----------------------------------------------------------

	fn height(&self) -> f32 {
		// title, method select, hint, gap, 4 stat rows, the applied row, gap, buttons.
		TITLE_H + 8.0 + ROW + ROW + 8.0 + 4.0 * ROW + ROW + 10.0 + BTN_H + 12.0
	}

	pub fn dialog_rect(&self, w: f32, h: f32) -> Rect {
		let dh = self.height();
		Rect::centered(w, h, W, dh).translate(self.drag_offset.0, self.drag_offset.1)
	}

	/// The method select's closed value box.
	fn method_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + 70.0, d.y + TITLE_H + 8.0, 220.0, BTN_H)
	}

	fn stats_y(&self, d: Rect) -> f32 {
		d.y + TITLE_H + 8.0 + ROW + ROW + 8.0
	}

	fn start_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + d.w - 110.0, d.y + d.h - BTN_H - 10.0, 100.0, BTN_H)
	}

	fn close_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + 10.0, d.y + d.h - BTN_H - 10.0, 80.0, BTN_H)
	}

	fn undo_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + d.w / 2.0 - 45.0, d.y + d.h - BTN_H - 10.0, 90.0, BTN_H)
	}

	/// Undo is offered once a run has applied a result and isn't running.
	fn can_undo(&self) -> bool {
		self.applied.is_some() && !self.running
	}

	// ----- events -------------------------------------------------------------

	pub fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		// The method select (only while idle): the box toggles the popup, an
		// option picks it, a miss closes it (and is swallowed so it can't also
		// hit a button beneath the floating list).
		if !self.running {
			let br = self.method_rect(d);
			match select::hit(br, self.select_open, Method::ALL.len(), false, x, y) {
				Some(select::Hit::Box) => {
					self.select_open = !self.select_open;
					return Press::Consumed;
				}
				Some(select::Hit::Option(i)) => {
					self.method = Method::ALL[i];
					self.select_open = false;
					return Press::SetMethod(self.method);
				}
				None if self.select_open => {
					self.select_open = false;
					return Press::Consumed;
				}
				None => {}
			}
		}
		// Command buttons arm and fire on release-inside.
		if self.can_undo() && self.undo_rect(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Undo);
			return Press::Consumed;
		}
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

	/// Fire the armed command button if the release is still on it; a release
	/// anywhere else just disarms.
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
			Some(ArmedBtn::Undo) if self.can_undo() && self.undo_rect(d).contains(x, y) => Press::Undo,
			Some(ArmedBtn::Close) if self.close_rect(d).contains(x, y) => Press::Close,
			_ => Press::Consumed,
		}
	}

	// ----- drawing -------------------------------------------------------------

	pub fn view(&self, w: f32, h: f32, hot: Hot) -> UiQuads {
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(ui::SteelMap::anchored(d));
		ui::modal_scrim(&mut q, w, h);
		ui::modal_frame(&mut q, d, "Fix Shore", TITLE_H, w, h);

		// Method select (closed box; the popup floats, drawn by `popup`).
		q.label("method", d.x + 12.0, d.y + TITLE_H + 8.0 + 5.0, ui::FONT_SMALL, w, h, theme::INK_DIM);
		select::draw_box(&mut q, self.method_rect(d), self.method.label(), self.select_open, w, h, hot);
		// One-line "what it does" hint below the select.
		q.label(self.method.hint(), d.x + 12.0, d.y + TITLE_H + 8.0 + ROW + 4.0, ui::FONT_SMALL, w, h, theme::INK_DIM);

		// Live stats.
		let sy = self.stats_y(d);
		let stat = |q: &mut UiQuads, i: usize, label: &str, value: String| {
			q.label(label, d.x + 12.0, sy + i as f32 * ROW, ui::FONT_SMALL, w, h, theme::INK_DIM);
			q.label(&value, d.x + 150.0, sy + i as f32 * ROW, ui::FONT_SMALL, w, h, theme::INK);
		};
		stat(&mut q, 0, "broken seams", self.found.to_string());
		stat(&mut q, 1, "fixed", self.fixed.to_string());
		stat(&mut q, 2, "remaining", self.remaining.to_string());
		// ASCII only - the MAX atlas has no em-dash (it would silently vanish).
		let elapsed =
			if self.running || self.applied.is_some() { format!("{:.1}s", self.elapsed) } else { "-".to_string() };
		stat(&mut q, 3, "elapsed", elapsed);
		if let Some(n) = self.applied {
			// Its own row below the stats, so it never overlaps the elapsed value.
			q.label(
				&format!("applied - {n} cells changed"),
				d.x + 12.0,
				sy + 4.0 * ROW,
				ui::FONT_SMALL,
				w,
				h,
				theme::INK_DIM,
			);
		}

		// Buttons.
		q.button(self.close_rect(d), w, h, hot);
		q.label_in("Close", self.close_rect(d), 8.0, ui::FONT_SMALL, w, h, theme::INK_DIM);
		// Undo the applied result (greyed until a run has applied one).
		let ur = self.undo_rect(d);
		q.button(ur, w, h, if self.can_undo() { hot } else { Hot::default() });
		let undo_ink = if self.can_undo() { theme::INK } else { theme::INK_DIM };
		q.label_in("Undo", ur, 8.0, ui::FONT_SMALL, w, h, undo_ink);
		let sr = self.start_rect(d);
		q.button_primary(sr, w, h, hot);
		let label = if self.running { "Stop" } else { "Start" };
		q.label_in(label, sr, 8.0, ui::FONT_SMALL, w, h, theme::INK);
		q
	}

	/// The open method-select popup, drawn last so it floats over the stats.
	pub fn popup(&self, w: f32, h: f32, hot: Hot) -> Option<UiQuads> {
		if !self.select_open {
			return None;
		}
		let d = self.dialog_rect(w, h);
		let labels: Vec<&str> = Method::ALL.iter().map(|m| m.label()).collect();
		let sel = Method::ALL.iter().position(|&m| m == self.method);
		let mut q = UiQuads::with_steel_map(ui::SteelMap::anchored(d));
		select::draw_popup(&mut q, self.method_rect(d), &labels, sel, false, w, h, hot);
		Some(q)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn select_picks_methods_and_start_fires() {
		let mut m = AutoFix::new(7);
		assert_eq!((m.found, m.remaining), (7, 7));
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		// Clicking the closed box opens the popup.
		let br = m.method_rect(d);
		assert_eq!(m.on_press(br.x + 2.0, br.y + 2.0, w, h), Press::Consumed);
		assert!(m.select_open);
		// Picking the last option (Destructive) sets it and closes the popup.
		let last = Method::ALL.len() - 1;
		let o = select::option_rect(br, last, Method::ALL.len(), false);
		assert_eq!(m.on_press(o.x + 2.0, o.y + 2.0, w, h), Press::SetMethod(Method::Destructive));
		assert_eq!(m.method, Method::Destructive);
		assert!(!m.select_open);
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
