//! The Fix Shore dialog: the one non-blocking tool window (Start/Stop/
//! Abort over a live run), synced each frame from the shell's run state.

use super::*;

#[derive(Clone, Copy)]
pub(super) struct AutoFixIds {
	/// The four live stat value labels + the applied-result line.
	pub(super) found: WidgetId,
	pub(super) fixed: WidgetId,
	pub(super) remaining: WidgetId,
	pub(super) elapsed: WidgetId,
	pub(super) applied: WidgetId,
	/// Close (idle) / Abort (running).
	pub(super) left: WidgetId,
	/// Start (idle) / Stop (running).
	pub(super) right: WidgetId,
}

impl Overlay {
	/// Opens the non-blocking Fix Shore window: four live stat rows, the
	/// applied-result line, and Close/Abort + Start/Stop. It floats over the
	/// live map (no scrim; pointer input outside falls through) — the shell
	/// steps the run and re-syncs the numbers via
	/// [`sync_autofix`](Self::sync_autofix) each frame.
	pub fn open_autofix(&mut self, found: usize) {
		let stat_row = |label: &str, value: Label| {
			Linear::row()
				.spacing(8.0)
				.cross_align(CrossAlign::Center)
				.child(Label::new(label).small().muted(), Length::Fixed(130.0))
				.child(value, Length::Flex(1.0))
		};
		let found_l = Label::new(found.to_string()).small().with_id();
		let fixed_l = Label::new("0").small().with_id();
		let remaining_l = Label::new(found.to_string()).small().with_id();
		let elapsed_l = Label::new("-").small().with_id();
		let applied_l = Label::new("").small().muted().with_id();
		let left = Button::new("Close");
		let right = Button::new("Start").primary();
		let ids = AutoFixIds {
			found: found_l.id(),
			fixed: fixed_l.id(),
			remaining: remaining_l.id(),
			elapsed: elapsed_l.id(),
			applied: applied_l.id(),
			left: left.id(),
			right: right.id(),
		};
		let content = column()
			.push(width_strut(300.0))
			.push(stat_row("broken seams", found_l))
			.push(stat_row("fixed", fixed_l))
			.push(stat_row("remaining", remaining_l))
			.push(stat_row("elapsed", elapsed_l))
			.push(applied_l)
			.push(buttons(left, right));
		let win = dialog("Fix Shore", content);
		self.win_id = Some(win.id());
		// No `modal(..)` scrim: the window is the whole tree, floating over the
		// live map.
		self.ui = Ui::new(win);
		self.dialog = Dialog::AutoFix(ids);
		self.blocking = false;
		self.af_running = false;
		self.events.clear();
		self.visible = true;
	}

	/// Pushes the live Fix Shore numbers into the window (the shell calls this
	/// every frame while it's open): stat values, the applied line, and the
	/// Close/Abort + Start/Stop captions that flip with `running`.
	pub fn sync_autofix(
		&mut self,
		running: bool,
		found: usize,
		fixed: usize,
		remaining: usize,
		elapsed: &str,
		applied: Option<usize>,
	) {
		let Dialog::AutoFix(ids) = self.dialog else { return };
		self.af_running = running;
		self.set_label(ids.found, &found.to_string());
		self.set_label(ids.fixed, &fixed.to_string());
		self.set_label(ids.remaining, &remaining.to_string());
		self.set_label(ids.elapsed, elapsed);
		// ASCII only - the MAX atlas has no em-dash (it would silently vanish).
		self.set_label(ids.applied, &applied.map(|n| format!("applied - {n} cells changed")).unwrap_or_default());
		if let Some(b) = self.ui.get_mut::<Button>(ids.left) {
			b.set_label(if running { "Abort" } else { "Close" });
			b.set_role(if running { wgpu_ui::Role::Secondary } else { wgpu_ui::Role::Neutral });
		}
		if let Some(b) = self.ui.get_mut::<Button>(ids.right) {
			b.set_label(if running { "Stop" } else { "Start" });
		}
	}

	/// One frame of the dialog's control dispatch (the render match arm).
	pub(super) fn dispatch_autofix(&mut self, ids: AutoFixIds) -> Outcome {
		let mut outcome = Outcome::Idle;
		if self.ui.fired(ids.left) {
			if self.af_running {
				outcome = Outcome::FixAbort;
			} else {
				outcome = Outcome::FixClose;
				self.hide();
			}
		} else if self.ui.fired(ids.right) {
			outcome = if self.af_running { Outcome::FixStop } else { Outcome::FixStart };
		}
		outcome
	}
}
