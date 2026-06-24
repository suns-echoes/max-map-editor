//! An editable text field: caret, selection, and the OS clipboard
//! (Ctrl+X/C/V). The character set is configurable ([`Charset`]); single-line
//! fields scroll horizontally, multiline fields word-wrap, accept literal LF
//! newlines (Enter), and grow a vertical scrollbar when they overflow. Carriage
//! returns (`\r`) are always stripped. Pure state + geometry; the shell clips
//! the content render to the field rect (`draw_ui_clipped`) so text stays in
//! the well. Hosted by the text-field modals.

use crate::modal::ModalKey;
use crate::theme;
use crate::ui::{self, FONT_SMALL, Hot, Rect, SCROLLBAR_W, UiQuads};

/// Which characters a field accepts. ASCII only, so a char index is a byte
/// index (a `\n`, when permitted, is also a single byte).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charset {
	/// Printable ASCII - space..`~`.
	Text,
	/// Digits only (`0`..`9`).
	Digits,
	/// Digits plus a single leading `-` (the minus rule is enforced in `insert`).
	Signed,
	/// Digits plus a single `.` (the decimal-point rule is enforced in `insert`).
	Decimal,
	/// Identifier chars - alphanumeric or `_`.
	Identifier,
}

impl Charset {
	/// Whether `c` is allowed by this set (ignoring positional rules - the single
	/// leading `-` of [`Charset::Signed`] / the single `.` of [`Charset::Decimal`]
	/// - which `insert` enforces).
	fn permits(self, c: char) -> bool {
		match self {
			Charset::Text => (' '..='~').contains(&c),
			Charset::Digits => c.is_ascii_digit(),
			Charset::Signed => c.is_ascii_digit() || c == '-',
			Charset::Decimal => c.is_ascii_digit() || c == '.',
			Charset::Identifier => c.is_ascii_alphanumeric() || c == '_',
		}
	}
}

pub(crate) fn clipboard_get() -> Option<String> {
	arboard::Clipboard::new().ok()?.get_text().ok()
}

pub(crate) fn clipboard_set(s: &str) {
	if let Ok(mut cb) = arboard::Clipboard::new() {
		let _ = cb.set_text(s.to_string());
	}
}

/// Wrapped-line spacing (px) for a multiline field.
const LINE_H: f32 = 15.0;
/// Left/right text padding inside the well (px).
const PAD: f32 = 6.0;

pub struct TextInput {
	/// The field's text - ASCII only (so a char index is a byte index).
	value: String,
	/// Caret position, `0..=len`.
	cursor: usize,
	/// Selection anchor; `None` = caret only. The selection is the range between
	/// `anchor` and `cursor`.
	anchor: Option<usize>,
	max_len: usize,
	/// Visible rows: `1` = single-line (horizontal scroll); `>1` = word-wrapped
	/// multiline that accepts literal `\n` newlines.
	rows: u16,
	/// Which characters typing accepts.
	charset: Charset,
	/// Vertical scroll (px) for an overflowing multiline field.
	scroll_y: f32,
	/// While set, a press in the scrollbar gutter is dragging the thumb (so a
	/// drag scrolls instead of extending a selection). Self-clears on the next
	/// press; no release event reaches the field.
	bar_drag: bool,
}

impl TextInput {
	pub fn new(value: &str, max_len: usize) -> Self {
		Self::with_rows(value, max_len, 1)
	}

	/// A word-wrapped, `rows`-line-tall field that accepts literal newlines
	/// (e.g. a description box).
	pub fn new_multiline(value: &str, max_len: usize, rows: u16) -> Self {
		Self::with_rows(value, max_len, rows.max(1))
	}

	/// Builder: restrict typing to `charset` (default [`Charset::Text`]).
	pub fn charset(mut self, charset: Charset) -> Self {
		self.charset = charset;
		self
	}

	fn with_rows(value: &str, max_len: usize, rows: u16) -> Self {
		let mut t = Self {
			value: String::new(),
			cursor: 0,
			anchor: None,
			max_len,
			rows,
			charset: Charset::Text,
			scroll_y: 0.0,
			bar_drag: false,
		};
		// Seed through the same filter so the initial value obeys rows/CR rules.
		t.value = value.chars().filter(|&c| t.keeps(c)).take(max_len).collect();
		t.cursor = t.value.len();
		t
	}

	/// Whether `c` may be stored at all: never `\r`; `\n` only when multiline;
	/// otherwise per the charset. (Positional rules - the single leading `-` of
	/// [`Charset::Signed`] - are applied in `insert`, not here.)
	fn keeps(&self, c: char) -> bool {
		match c {
			'\r' => false,
			'\n' => self.rows > 1,
			_ => self.charset.permits(c),
		}
	}

	/// Word-wrap the value into char-index line ranges that fit `width` px,
	/// splitting first on stored `\n` (the separator is excluded from the
	/// ranges) and then breaking at spaces where possible (else a hard break).
	/// An empty paragraph yields one empty `(p, p)` line. ASCII only, so a char
	/// index is a byte index.
	fn wrap(&self, width: f32) -> Vec<(usize, usize)> {
		let v = &self.value;
		let n = v.len();
		let mut lines = Vec::new();
		let mut para_start = 0;
		loop {
			// This paragraph runs to the next '\n' (excluded) or the buffer end.
			let para_end = v[para_start..].find('\n').map(|i| para_start + i).unwrap_or(n);
			if para_start == para_end {
				lines.push((para_start, para_start)); // empty line
			}
			let mut start = para_start;
			while start < para_end {
				let mut end = start;
				let mut last_space = None;
				while end < para_end {
					let next = end + 1;
					if crate::text::label_width(&v[start..next], FONT_SMALL) > width && end > start {
						break;
					}
					if v.as_bytes()[end] == b' ' {
						last_space = Some(next);
					}
					end = next;
				}
				let brk = if end < para_end { last_space.unwrap_or(end).max(start + 1) } else { end };
				lines.push((start, brk));
				start = brk;
			}
			if para_end >= n {
				break;
			}
			para_start = para_end + 1; // skip the '\n'
		}
		if lines.is_empty() {
			lines.push((0, 0));
		}
		lines
	}

	/// The visual content width and wrapped lines for a multiline `rect`,
	/// reserving the scrollbar gutter only when the text overflows (the `bool`).
	fn multiline_layout(&self, rect: Rect) -> (Vec<(usize, usize)>, bool) {
		let full = (rect.w - 2.0 * PAD).max(1.0);
		let lines = self.wrap(full);
		if lines.len() as f32 * LINE_H > rect.h {
			let narrow = (rect.w - 2.0 * PAD - SCROLLBAR_W).max(1.0);
			(self.wrap(narrow), true)
		} else {
			(lines, false)
		}
	}

	/// Total content height (px) of `lines`, including top/bottom padding.
	fn content_h(lines: &[(usize, usize)]) -> f32 {
		lines.len() as f32 * LINE_H + 2.0 * PAD
	}

	/// `(line, column-x px)` of the caret within the wrapped layout. A caret
	/// sitting on a `\n` boundary lands on the preceding visual line.
	fn caret_line(&self, lines: &[(usize, usize)]) -> usize {
		lines.iter().rposition(|&(s, _)| s <= self.cursor).unwrap_or(0)
	}

	pub fn text(&self) -> &str {
		&self.value
	}

	/// Replace the whole value (filtered through the charset/rows rules) and put
	/// the caret at the end - the programmatic counterpart to typing.
	pub fn set_text(&mut self, s: &str) {
		self.value = s.chars().filter(|&c| self.keeps(c)).take(self.max_len).collect();
		self.cursor = self.value.len();
		self.anchor = None;
		self.scroll_y = 0.0;
	}

	/// Multiline fields accept literal newlines; the host inserts one on Enter.
	pub fn wants_newline(&self) -> bool {
		self.rows > 1
	}

	/// Insert a literal newline at the caret (multiline only - a no-op otherwise
	/// via the `\n` filter).
	pub fn insert_newline(&mut self) {
		self.insert("\n");
	}

	/// Whether a (non-empty) selection exists - gates the edit context menu.
	pub fn has_selection(&self) -> bool {
		self.selection().is_some()
	}

	/// The edit state the right-click context menu needs (selection present +
	/// whether the field is empty) - every text modal's `edit_context` builds
	/// this from its focused field.
	pub fn edit_context(&self) -> crate::modal::EditContext {
		crate::modal::EditContext { has_selection: self.has_selection(), is_empty: self.text().is_empty() }
	}

	/// Ordered selection range `(start, end)`, or `None` when nothing is selected.
	/// Both ends are clamped to the current length so the range is always a valid
	/// slice of `value` - the render/measure paths index `value` by it, and a
	/// stale `anchor` left past the end must never panic them.
	fn selection(&self) -> Option<(usize, usize)> {
		let len = self.value.len();
		let a = self.anchor?.min(len);
		let c = self.cursor.min(len);
		(a != c).then(|| (a.min(c), a.max(c)))
	}

	fn delete_selection(&mut self) -> bool {
		if let Some((s, e)) = self.selection() {
			self.value.replace_range(s..e, "");
			self.cursor = s;
			self.anchor = None;
			true
		} else {
			false
		}
	}

	fn insert(&mut self, text: &str) {
		self.delete_selection();
		for c in text.chars() {
			if !self.keeps(c) {
				continue;
			}
			// A signed field permits only a single leading '-'; a decimal field
			// only a single '.'.
			if c == '-' && self.charset == Charset::Signed && (self.cursor != 0 || self.value.starts_with('-')) {
				continue;
			}
			if c == '.' && self.charset == Charset::Decimal && self.value.contains('.') {
				continue;
			}
			if self.value.len() >= self.max_len {
				break;
			}
			self.value.insert(self.cursor, c);
			self.cursor += 1;
		}
		self.anchor = None;
	}

	/// Move the caret to `pos`, extending the selection when `extend` (Shift).
	fn move_to(&mut self, pos: usize, extend: bool) {
		if extend {
			self.anchor.get_or_insert(self.cursor);
		} else {
			self.anchor = None;
		}
		self.cursor = pos.min(self.value.len());
	}

	/// Apply a decoded modal key. Caller routes keys only to the focused field.
	/// (Single-line fields keep the caret visible by horizontal scroll; multiline
	/// hosts should use [`Self::on_key_in`] so vertical scroll follows the caret.)
	pub fn on_key(&mut self, key: &ModalKey) {
		match key {
			ModalKey::Char(c) => self.insert(&c.to_string()),
			ModalKey::Backspace => {
				if !self.delete_selection() && self.cursor > 0 {
					self.cursor -= 1;
					self.value.remove(self.cursor);
				}
				// Collapse any selection: an edit always ends with a bare caret.
				// (A press arms `anchor` for drag-select; leaving it set after a
				// delete would strand it past the now-shorter value - a phantom
				// selection whose range slices `value` out of bounds on render.)
				self.anchor = None;
			}
			ModalKey::Delete => {
				if !self.delete_selection() && self.cursor < self.value.len() {
					self.value.remove(self.cursor);
				}
				self.anchor = None;
			}
			ModalKey::Left { shift } => {
				// Collapse a selection to its left edge instead of stepping.
				if !shift {
					if let Some((s, _)) = self.selection() {
						self.move_to(s, false);
						return;
					}
				}
				self.move_to(self.cursor.saturating_sub(1), *shift);
			}
			ModalKey::Right { shift } => {
				if !shift {
					if let Some((_, e)) = self.selection() {
						self.move_to(e, false);
						return;
					}
				}
				self.move_to(self.cursor + 1, *shift);
			}
			ModalKey::Home { shift } => self.move_to(0, *shift),
			ModalKey::End { shift } => self.move_to(self.value.len(), *shift),
			ModalKey::SelectAll => {
				self.anchor = Some(0);
				self.cursor = self.value.len();
			}
			ModalKey::Copy => {
				if let Some((s, e)) = self.selection() {
					clipboard_set(&self.value[s..e]);
				}
			}
			ModalKey::Cut => {
				if let Some((s, e)) = self.selection() {
					clipboard_set(&self.value[s..e]);
					self.delete_selection();
				}
			}
			ModalKey::Paste => {
				if let Some(t) = clipboard_get() {
					self.insert(&t);
				}
			}
			_ => {}
		}
	}

	/// [`Self::on_key`] followed by scrolling the caret into view - the path a
	/// multiline host uses so vertical scroll tracks edits and caret moves.
	pub fn on_key_in(&mut self, key: &ModalKey, rect: Rect) {
		self.on_key(key);
		self.scroll_caret_into_view(rect);
	}

	/// Keep the caret's visual line within the multiline viewport (called after
	/// a caret-affecting edit/move; the wheel scrolls freely outside this).
	pub fn scroll_caret_into_view(&mut self, rect: Rect) {
		if self.rows <= 1 {
			return;
		}
		let (lines, _) = self.multiline_layout(rect);
		let cl = self.caret_line(&lines);
		let (top, bot) = (cl as f32 * LINE_H, cl as f32 * LINE_H + LINE_H);
		let view = (rect.h - 2.0 * PAD).max(LINE_H);
		if self.scroll_y > top {
			self.scroll_y = top;
		} else if self.scroll_y + view < bot {
			self.scroll_y = bot - view;
		}
		self.scroll_y = self.scroll_y.clamp(0.0, ui::scroll_max(Self::content_h(&lines), rect.h));
	}

	/// Wheel over a focused multiline field scrolls its text.
	pub fn on_wheel(&mut self, steps: f32, rect: Rect) {
		if self.rows <= 1 {
			return;
		}
		let (lines, _) = self.multiline_layout(rect);
		let max = ui::scroll_max(Self::content_h(&lines), rect.h);
		self.scroll_y = (self.scroll_y - steps * LINE_H * 3.0).clamp(0.0, max);
	}

	/// Set the multiline scroll so the track position under `y` is shown
	/// (track-click / thumb-drag).
	fn scroll_to_pointer(&mut self, y: f32, rect: Rect, lines: &[(usize, usize)]) {
		let max = ui::scroll_max(Self::content_h(lines), rect.h);
		let t = ((y - rect.y) / rect.h.max(1.0)).clamp(0.0, 1.0);
		self.scroll_y = (t * max).clamp(0.0, max);
	}

	/// Nearest char gap to the click - within the row picked by `y` for a
	/// multiline field (honouring vertical scroll), else along the single line
	/// (honouring its horizontal scroll).
	fn caret_at(&self, x: f32, y: f32, rect: Rect) -> usize {
		let nearest = |lo: usize, hi: usize, local: f32| {
			let mut best = (lo, f32::MAX);
			for n in lo..=hi {
				let d = (crate::text::label_width(&self.value[lo..n], FONT_SMALL) - local).abs();
				if d < best.1 {
					best = (n, d);
				}
			}
			best.0
		};
		if self.rows <= 1 {
			let window = (rect.w - 2.0 * PAD).max(1.0);
			let caret_x = crate::text::label_width(&self.value[..self.cursor], FONT_SMALL);
			let scroll = (caret_x - window).max(0.0);
			nearest(0, self.value.len(), x - rect.x - PAD + scroll)
		} else {
			let (lines, _) = self.multiline_layout(rect);
			let row = (y - rect.y - PAD + self.scroll_y) / LINE_H;
			let li = (row.floor() as i32).clamp(0, lines.len() as i32 - 1) as usize;
			let (s, e) = lines[li];
			nearest(s, e, x - rect.x - PAD)
		}
	}

	/// Mouse press: place the caret and start a (possibly empty) selection - a
	/// drag then extends it. A press in an overflowing multiline field's
	/// scrollbar gutter instead grabs the thumb (track-click jumps to it).
	pub fn on_press(&mut self, x: f32, y: f32, rect: Rect) {
		if self.rows > 1 {
			let (lines, overflow) = self.multiline_layout(rect);
			if overflow && x >= rect.x + rect.w - SCROLLBAR_W {
				self.bar_drag = true;
				self.scroll_to_pointer(y, rect, &lines);
				return;
			}
			self.bar_drag = false;
		}
		self.cursor = self.caret_at(x, y, rect);
		self.anchor = Some(self.cursor);
	}

	/// Mouse drag: extend the selection to the cursor (the anchor stays put), or
	/// scroll when a scrollbar drag is in progress.
	pub fn on_drag(&mut self, x: f32, y: f32, rect: Rect) {
		if self.bar_drag {
			let (lines, _) = self.multiline_layout(rect);
			self.scroll_to_pointer(y, rect, &lines);
			return;
		}
		self.cursor = self.caret_at(x, y, rect);
	}

	/// The text + selection highlight + caret. Single-line fields scroll
	/// horizontally to keep the caret visible; multiline fields word-wrap into
	/// rows, scroll vertically, and draw a scrollbar when they overflow. The
	/// caller clips this to `rect`.
	pub fn content_quads(&self, rect: Rect, focused: bool, w: f32, h: f32) -> UiQuads {
		let mut q = UiQuads::default();
		let upto = |lo: usize, n: usize| crate::text::label_width(&self.value[lo..n], FONT_SMALL);
		if self.rows <= 1 {
			let window = (rect.w - 2.0 * PAD).max(1.0);
			let caret_x = upto(0, self.cursor);
			let scroll = (caret_x - window).max(0.0);
			let base = rect.x + PAD - scroll;
			let ty = rect.y + (rect.h - FONT_SMALL) / 2.0;
			if focused {
				if let Some((s, e)) = self.selection() {
					let (x0, x1) = (base + upto(0, s), base + upto(0, e));
					q.rect(Rect::new(x0, rect.y + 2.0, (x1 - x0).max(1.0), rect.h - 4.0), w, h, theme::TEXT_SELECTION);
				}
			}
			q.label(&self.value, base, ty, FONT_SMALL, w, h, theme::INK);
			if focused {
				q.rect(Rect::new(base + caret_x, rect.y + 3.0, 2.0, rect.h - 6.0), w, h, theme::ACCENT);
			}
			return q;
		}
		// Multiline: one label per wrapped line, with the selection band and the
		// caret on their own rows, offset by the vertical scroll.
		let (lines, overflow) = self.multiline_layout(rect);
		let scroll = self.scroll_y.clamp(0.0, ui::scroll_max(Self::content_h(&lines), rect.h));
		let sel = self.selection();
		let cline = self.caret_line(&lines);
		for (li, &(s, e)) in lines.iter().enumerate() {
			let ly = rect.y + PAD + li as f32 * LINE_H - scroll;
			let lx = rect.x + PAD;
			if focused {
				if let Some((ss, se)) = sel {
					let (a, b) = (ss.max(s), se.min(e));
					if a < b {
						let x0 = lx + upto(s, a);
						let x1 = lx + upto(s, b);
						q.rect(Rect::new(x0, ly, (x1 - x0).max(1.0), LINE_H), w, h, theme::TEXT_SELECTION);
					}
				}
			}
			q.label(&self.value[s..e], lx, ly, FONT_SMALL, w, h, theme::INK);
			if focused && li == cline {
				q.rect(Rect::new(lx + upto(s, self.cursor), ly, 2.0, LINE_H - 2.0), w, h, theme::ACCENT);
			}
		}
		if overflow {
			q.scrollbar(rect, Self::content_h(&lines), scroll, w, h, Hot::NONE);
		}
		q
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn typing_filters_to_ascii_and_caps_at_max_len() {
		let mut t = TextInput::new("", 5);
		for c in "abñc!".chars() {
			t.on_key(&ModalKey::Char(c)); // 'ñ' is non-ASCII → dropped
		}
		assert_eq!(t.text(), "abc!");
		for c in "xyz".chars() {
			t.on_key(&ModalKey::Char(c)); // max_len 5 caps it
		}
		assert_eq!(t.text(), "abc!x");
	}

	#[test]
	fn digits_charset_rejects_non_digits() {
		let mut t = TextInput::new("", 4).charset(Charset::Digits);
		for c in "1a2-3".chars() {
			t.on_key(&ModalKey::Char(c));
		}
		assert_eq!(t.text(), "123", "only digits survive");
	}

	#[test]
	fn signed_charset_allows_one_leading_minus_only() {
		let mut t = TextInput::new("", 6).charset(Charset::Signed);
		for c in "-12".chars() {
			t.on_key(&ModalKey::Char(c));
		}
		assert_eq!(t.text(), "-12");
		// A second '-' anywhere is rejected; a '-' not at the front is rejected.
		t.on_key(&ModalKey::Char('-'));
		assert_eq!(t.text(), "-12");
		t.on_key(&ModalKey::Home { shift: false });
		t.on_key(&ModalKey::Right { shift: false }); // caret after the '-'
		t.on_key(&ModalKey::Char('-'));
		assert_eq!(t.text(), "-12", "interior '-' rejected");
	}

	#[test]
	fn identifier_charset_allows_alnum_and_underscore() {
		let mut t = TextInput::new("", 16).charset(Charset::Identifier);
		for c in "GLa_007!?".chars() {
			t.on_key(&ModalKey::Char(c));
		}
		assert_eq!(t.text(), "GLa_007");
	}

	#[test]
	fn cr_always_stripped_and_lf_only_when_multiline() {
		let mut single = TextInput::new("", 32);
		single.on_key(&ModalKey::Char('a'));
		single.insert("\r\n"); // CR dropped; LF dropped (single-line)
		single.on_key(&ModalKey::Char('b'));
		assert_eq!(single.text(), "ab");

		let mut multi = TextInput::new_multiline("", 32, 4);
		multi.on_key(&ModalKey::Char('a'));
		multi.insert("\r\n"); // CR dropped; LF kept (multiline)
		multi.on_key(&ModalKey::Char('b'));
		assert_eq!(multi.text(), "a\nb");
		// Seeding through the constructor obeys the same rule.
		assert_eq!(TextInput::new_multiline("x\r\ny", 32, 3).text(), "x\ny");
		assert_eq!(TextInput::new("x\r\ny", 32).text(), "xy");
	}

	#[test]
	fn shift_arrows_select_and_backspace_deletes_the_run() {
		let mut t = TextInput::new("hello", 32);
		t.on_key(&ModalKey::Home { shift: false });
		t.on_key(&ModalKey::Right { shift: true });
		t.on_key(&ModalKey::Right { shift: true });
		assert_eq!(t.selection(), Some((0, 2)));
		assert!(t.has_selection());
		t.on_key(&ModalKey::Backspace);
		assert_eq!(t.text(), "llo");
		assert!(!t.has_selection());
		t.on_key(&ModalKey::Delete);
		assert_eq!(t.text(), "lo");
	}

	#[test]
	fn delete_clears_a_stale_click_anchor_no_oob_render() {
		// Regression: clicking a field arms `anchor` for drag-select (anchor ==
		// caret). A plain Backspace then shrank `value` and moved the caret but
		// left `anchor` pointing past the new end, so `selection()` reported a
		// range whose end sliced `value` out of bounds on the next render → panic.
		// Repro: "new map → click width → Backspace".
		let rect = Rect::new(0.0, 0.0, 56.0, 20.0);
		let mut t = TextInput::new("112", 4).charset(Charset::Digits);
		t.on_press(1.0e6, 10.0, rect); // click far right → caret at end, anchor armed
		assert_eq!(t.cursor, 3);
		t.on_key(&ModalKey::Backspace);
		assert_eq!(t.text(), "11");
		assert!(t.selection().is_none(), "an edit collapses the armed anchor");
		// The render path must not slice `value` out of bounds.
		let _ = t.content_quads(rect, true, 100.0, 100.0);

		// Same for Delete from an interior click.
		t.on_press(0.0, 10.0, rect); // caret at start, anchor armed
		t.on_key(&ModalKey::Delete);
		assert_eq!(t.text(), "1");
		assert!(t.selection().is_none());
		let _ = t.content_quads(rect, true, 100.0, 100.0);
	}

	#[test]
	fn multiline_wrap_excludes_newline_but_reconstructs_with_it() {
		let mut t = TextInput::new_multiline("", 64, 5);
		t.insert("ab\ncd");
		let lines = t.wrap(1000.0); // wide → only the hard break splits
		assert_eq!(lines, vec![(0, 2), (3, 5)], "the '\\n' at index 2 is excluded");
		// Rejoining the pieces with a '\n' between paragraphs gives the buffer.
		let joined: String = lines.iter().map(|&(s, e)| &t.text()[s..e]).collect::<Vec<_>>().join("\n");
		assert_eq!(joined, t.text());
	}

	#[test]
	fn multiline_wraps_on_spaces_within_a_paragraph() {
		let t = TextInput::new_multiline("the quick brown fox jumps over", 160, 5);
		let lines = t.wrap(60.0); // narrow → forces several lines
		assert!(lines.len() > 1, "wrapped into multiple lines");
		// Within one paragraph (no '\n'), the pieces rejoin to the original.
		let joined: String = lines.iter().map(|&(s, e)| &t.text()[s..e]).collect();
		assert_eq!(joined, t.text(), "the wrapped pieces are the original text");
	}

	#[test]
	fn empty_paragraph_yields_a_blank_line_and_caret_lands_on_it() {
		let mut t = TextInput::new_multiline("", 64, 5);
		t.insert("a\n\nb"); // "a", empty line, "b"
		let lines = t.wrap(1000.0);
		assert_eq!(lines, vec![(0, 1), (2, 2), (3, 4)]);
		// Caret at byte 2 (the second '\n' / start of the empty para) is on line 1.
		t.move_to(2, false);
		assert_eq!(t.caret_line(&lines), 1);
		// Caret on the first '\n' (byte 1) stays on the preceding line 0.
		t.move_to(1, false);
		assert_eq!(t.caret_line(&lines), 0);
	}

	#[test]
	fn multiline_wheel_and_caret_scroll_clamp() {
		let mut t = TextInput::new_multiline("", 256, 3);
		for _ in 0..40 {
			t.insert("x\n");
		}
		let rect = Rect::new(0.0, 0.0, 200.0, 60.0); // ~4 visible rows
		// Wheeling down moves toward the bottom and clamps at the max.
		t.on_wheel(-1000.0, rect);
		let max = ui::scroll_max(TextInput::content_h(&t.wrap(200.0 - 2.0 * PAD)), rect.h);
		assert!(max > 0.0 && (t.scroll_y - max).abs() < 0.5, "clamped to max");
		// Home jumps the caret to the top; scroll-into-view follows it back up.
		t.on_key_in(&ModalKey::Home { shift: false }, rect);
		t.scroll_caret_into_view(rect);
		assert_eq!(t.scroll_y, 0.0, "caret at top scrolls to top");
	}

	#[test]
	fn mouse_press_then_drag_selects_a_run() {
		let mut t = TextInput::new("hello world", 32);
		let rect = Rect::new(0.0, 0.0, 200.0, 20.0);
		t.on_press(0.0, 5.0, rect);
		assert_eq!(t.selection(), None, "a press alone is a bare caret");
		t.on_drag(1000.0, 5.0, rect); // drag past the end
		assert_eq!(t.selection(), Some((0, t.text().len())), "drag selects to the end");
	}

	#[test]
	fn select_all_then_type_replaces_everything() {
		let mut t = TextInput::new("old value", 32);
		t.on_key(&ModalKey::SelectAll);
		t.on_key(&ModalKey::Char('N'));
		assert_eq!(t.text(), "N");
	}
}
