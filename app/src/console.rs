//! In-app console: line input + scrollback over the command parser
//!. Pure state - rendering lives in `text.rs`, key routing in
//! `main.rs`, execution in `state.rs`. Leaner than world-editor's original
//! (no autocomplete yet - `suggestions()` is wired but empty).

const MAX_LOG: usize = 500;

pub struct Console {
	open: bool,
	input: String,
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
			input: String::new(),
			log: vec![
				"M.A.X. Map Editor console - Enter runs, Up/Down history, PgUp/PgDn scroll".into(),
				"commands: new[!] new-map tile tool transform pick paint shore[ alt|fix] stroke".into(),
				"          mode pass-pick pass-paint  grid pass-overlay  resize[-modal]".into(),
				"          place erase undo redo open[!] save save-project save-copy export".into(),
				"          file-dialog fix-shore-modal color set-color hsl-block".into(),
				"          window dock picker minimap menu".into(),
				"          pan pan-to zoom zoom-at zoom-to fit set-tile set-pass".into(),
				"          animate tick console screenshot hash assert-* quit[!]".into(),
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
	pub fn input(&self) -> &str {
		&self.input
	}
	pub fn log(&self) -> &[String] {
		&self.log
	}
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

	/// Append printable characters to the input line.
	pub fn insert(&mut self, text: &str) {
		for ch in text.chars().filter(|c| !c.is_control()) {
			self.input.push(ch);
		}
	}

	pub fn backspace(&mut self) {
		self.input.pop();
	}

	/// Enter: echo the line into the log + history and return it for parsing.
	pub fn submit(&mut self) -> Option<String> {
		let line = std::mem::take(&mut self.input);
		self.hist_pos = None;
		self.scroll = 0;
		if line.trim().is_empty() {
			return None;
		}
		self.push_line(format!("] {line}"));
		self.history.push(line.clone());
		Some(line)
	}

	pub fn history_prev(&mut self) {
		if self.history.is_empty() {
			return;
		}
		let pos = match self.hist_pos {
			None => self.history.len() - 1,
			Some(0) => 0,
			Some(p) => p - 1,
		};
		self.hist_pos = Some(pos);
		self.input = self.history[pos].clone();
	}

	pub fn history_next(&mut self) {
		match self.hist_pos {
			None => {}
			Some(p) if p + 1 < self.history.len() => {
				self.hist_pos = Some(p + 1);
				self.input = self.history[p + 1].clone();
			}
			Some(_) => {
				self.hist_pos = None;
				self.input.clear();
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
		c.insert("fit");
		assert_eq!(c.submit().as_deref(), Some("fit"));
		assert_eq!(c.log().last().unwrap(), "] fit");
		assert_eq!(c.input(), "");
		assert!(c.submit().is_none(), "empty input does not submit");
		c.history_prev();
		assert_eq!(c.input(), "fit");
		c.history_next();
		assert_eq!(c.input(), "");
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
