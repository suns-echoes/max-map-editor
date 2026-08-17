//! In-app console: scrollback + history over the command parser. The **input
//! line is a `wgpu_ui::TextInput`** (hosted in a `PanelUi`, events routed by
//! the shell like any layer's); this type owns only the log / history / scroll
//! model around it. Rendering is `console_view.rs` (the retained
//! `ConsoleView` widget), routing `main.rs`, execution `state.rs`. Leaner than
//! world-editor's original (no autocomplete yet - `suggestions()` is wired but
//! empty).

const MAX_LOG: usize = 500;

pub struct Console {
	open: bool,
	log: Vec<String>,
	history: Vec<String>,
	/// Index into `history` while browsing with Up/Down, newest-last.
	hist_pos: Option<usize>,
	/// Lines scrolled up from the newest (0 = live tail).
	scroll: usize,
	view_rows: usize,
}

impl Console {
	pub fn new() -> Self {
		Self {
			open: false,
			log: vec![
				// Grouped by family rather than enumerated: the verb list is ~200
				// long and grows with every feature, so a flat list here goes stale
				// silently (it had lost template-*, scenery-*, unit-* and generate
				// entirely). Families stay true; the manual carries the arguments.
				"M.A.X. Map Editor console - Enter runs, Up/Down history, PgUp/PgDn scroll".into(),
				"files:    new[!] open[!] save save-as save-project save-copy export-wrl export-save".into(),
				"edit:     tile tool pick paint place erase undo redo stroke select-* copy cut paste".into(),
				"terrain:  shore fix-shore-modal paint-mask resize[-modal] generate[-modal] resource-*".into(),
				"content:  unit-* object-* scenery-* template-* tile-* palette-* match-*".into(),
				"view:     mode grid pass-overlay resources ingame crt zoom[-at|-to] pan[-to] fit".into(),
				"layout:   window dock picker minimap menu tab console status-bar ui-scale".into(),
				"testing:  screenshot hash assert-* animate tick quit[!]".into(),
				"Help > User Manual lists every verb with its arguments.".into(),
			],
			history: Vec::new(),
			hist_pos: None,
			scroll: 0,
			view_rows: 10,
		}
	}

	pub fn is_open(&self) -> bool {
		self.open
	}
	pub fn set_open(&mut self, open: bool) {
		self.open = open;
	}
	/// The whole scrollback. Only the tests read it now — the view is handed
	/// [`visible_lines`](Self::visible_lines) instead, so nothing outside this
	/// module has to know about `scroll`.
	#[cfg(test)]
	pub fn log(&self) -> &[String] {
		&self.log
	}
	#[cfg(test)]
	pub fn scroll(&self) -> usize {
		self.scroll
	}
	/// Autocomplete entries (name, help) - not implemented yet; hooks for
	/// the world-editor-style dropdown.
	#[allow(dead_code)]
	pub fn suggestions(&self) -> &[(String, String)] {
		&[]
	}
	#[allow(dead_code)]
	pub fn sel(&self) -> usize {
		0
	}

	pub fn set_view_rows(&mut self, rows: usize) {
		self.view_rows = rows.max(1);
	}

	/// The `rows` scrollback lines the view should show, oldest first — the
	/// window `scroll` names, ending at the live tail when it is 0. The console
	/// view is handed exactly this and draws it bottom-up.
	pub fn visible_lines(&self, rows: usize) -> Vec<String> {
		let end = self.log.len() - self.scroll.min(self.log.len());
		let start = end.saturating_sub(rows);
		self.log[start..end].to_vec()
	}

	/// Enter: echo `line` into the log + history. Returns the line to parse when
	/// it is non-empty; the caller clears the input `TextInput` itself. Also
	/// resets history browsing and snaps the scrollback to the live tail.
	pub fn submit(&mut self, line: &str) -> Option<String> {
		self.hist_pos = None;
		self.scroll = 0;
		if line.trim().is_empty() {
			return None;
		}
		self.push_line(format!("] {line}"));
		self.history.push(line.to_string());
		Some(line.to_string())
	}

	/// Up: recall the previous history entry into the input; `None` leaves the
	/// input untouched (empty history). The caller feeds the text into the field.
	pub fn history_prev(&mut self) -> Option<String> {
		if self.history.is_empty() {
			return None;
		}
		let pos = match self.hist_pos {
			None => self.history.len() - 1,
			Some(0) => 0,
			Some(p) => p - 1,
		};
		self.hist_pos = Some(pos);
		Some(self.history[pos].clone())
	}

	/// Down: walk forward through history, finally restoring the empty prompt
	/// (`Some("")`); `None` when nothing is being browsed (leave the input alone).
	pub fn history_next(&mut self) -> Option<String> {
		match self.hist_pos {
			None => None,
			Some(p) if p + 1 < self.history.len() => {
				self.hist_pos = Some(p + 1);
				Some(self.history[p + 1].clone())
			}
			Some(_) => {
				self.hist_pos = None;
				Some(String::new())
			}
		}
	}

	pub fn scroll_lines(&mut self, delta: i32) {
		let max = self.log.len().saturating_sub(self.view_rows);
		self.scroll = (self.scroll as i64 + delta as i64).clamp(0, max as i64) as usize;
	}

	/// Append an output line and snap the view back to the live tail.
	pub fn push_line(&mut self, line: impl Into<String>) {
		self.log.push(line.into());
		if self.log.len() > MAX_LOG {
			self.log.remove(0);
		}
		self.scroll = 0;
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn submit_echoes_and_records_history() {
		let mut c = Console::new();
		assert_eq!(c.submit("fit").as_deref(), Some("fit"));
		assert_eq!(c.log().last().unwrap(), "] fit");
		assert!(c.submit("   ").is_none(), "blank input does not submit");
		assert!(c.submit("").is_none(), "empty input does not submit");
		// The one submitted line is the only history entry.
		assert_eq!(c.history_prev().as_deref(), Some("fit"), "Up recalls the last line");
		assert_eq!(c.history_next().as_deref(), Some(""), "Down past the newest restores the empty prompt");
	}

	#[test]
	fn scroll_clamps_to_log() {
		let mut c = Console::new();
		c.set_view_rows(2);
		for i in 0..10 {
			c.push_line(format!("line {i}"));
		}
		c.scroll_lines(1000);
		assert_eq!(c.scroll(), c.log().len() - 2);
		c.scroll_lines(-1000);
		assert_eq!(c.scroll(), 0);
		c.push_line("new");
		assert_eq!(c.scroll(), 0, "new output snaps to tail");
	}

	/// The autocomplete hooks are wired but intentionally empty stubs until the
	/// dropdown lands - they must report "nothing" rather than panic.
	#[test]
	fn suggestion_stubs_report_nothing() {
		let c = Console::new();
		assert!(c.suggestions().is_empty(), "no autocomplete entries yet");
		assert_eq!(c.sel(), 0, "no selection without entries");
	}

	/// History browsing clamps at both ends and returns the text the caller
	/// pushes into the `TextInput`: Up with no history is `None` (leave the input
	/// alone), Up pins at the oldest entry, Down walks forward and finally
	/// restores the empty prompt.
	#[test]
	fn history_navigation_clamps_and_walks_both_ways() {
		// No history yet: Up/Down leave the (typed) input alone (None).
		let mut c = Console::new();
		assert!(c.history_next().is_none(), "next with empty history is a no-op");
		assert!(c.history_prev().is_none(), "prev with empty history is a no-op");

		let mut c = Console::new();
		c.submit("first");
		c.submit("second");
		assert_eq!(c.history_prev().as_deref(), Some("second"));
		assert_eq!(c.history_prev().as_deref(), Some("first"));
		assert_eq!(c.history_prev().as_deref(), Some("first"), "Up clamps at the oldest entry");
		assert_eq!(c.history_next().as_deref(), Some("second"), "Down walks forward through history");
		assert_eq!(c.history_next().as_deref(), Some(""), "Down past the newest restores the empty prompt");
	}

	/// The scrollback is bounded: old lines fall off the front once the log
	/// exceeds its cap, so a long session can't grow without limit.
	#[test]
	fn log_drops_oldest_lines_past_the_cap() {
		let mut c = Console::new();
		for i in 0..600 {
			c.push_line(format!("line {i}"));
		}
		assert_eq!(c.log().len(), MAX_LOG, "log capped");
		assert_eq!(c.log().last().unwrap(), "line 599", "newest kept");
		assert_eq!(c.log().first().unwrap(), "line 100", "oldest dropped from the front");
	}

	#[test]
	fn home_and_end_jump_to_oldest_and_newest() {
		let mut c = Console::new();
		c.set_view_rows(2);
		for i in 0..10 {
			c.push_line(format!("line {i}"));
		}
		// Home (a huge positive delta) pins the view to the oldest visible page.
		c.scroll_lines(i32::MAX);
		assert_eq!(c.scroll(), c.log().len() - 2);
		// End (a huge negative delta) returns to the live tail.
		c.scroll_lines(i32::MIN);
		assert_eq!(c.scroll(), 0);
	}
}
