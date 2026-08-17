//! The console's chrome as **one retained widget**: the plate, the scrollback,
//! the `] ` prompt, and a hosted `wgpu_ui::TextInput` for the input line.
//!
//! Before U4.5 the console was the last surface in the editor with no pointer
//! path at all — its field was hosted but never arranged, so it had no rect, so
//! nothing could click it — and the last one drawing its own glyphs, from a
//! baked 10×19 bitmap atlas composited in *physical* pixels. Both are gone: the
//! field is an ordinary [`wgpu_ui::TextInput`] (`role(Mono)`, `frameless`) with
//! click-to-caret, drag-select and Ctrl+A/C/V for free, and every glyph here
//! goes through `Theme::text_colored` at [`TextRole::Mono`], so the console
//! scales with the rest of the chrome.
//!
//! The scrollback is still a faithful draw (there is no toolkit log view yet —
//! gap G5); it is the *content* of this widget, fed one visible window per frame
//! by the shell, exactly like a panel's [`Snapshot`](crate::panel_ui).

use wgpu_ui::widget::{DrawCtx, EventCtx, LayoutCtx, Widget};
use wgpu_ui::{DrawList, Emboss, Event, Rect, ScrollDelta, Size, TextCommit, TextInput, TextRole, Vec2, WidgetId};

use crate::theme::{CONSOLE_BORDER, CONSOLE_ECHO, CONSOLE_ERROR, CONSOLE_INPUT, CONSOLE_LOG, CONSOLE_PANEL};
use crate::uikit_theme::rgba;

/// Console height as a fraction of the window.
const PANEL_FRAC: f32 = 0.55;
/// Inner margin.
const PAD: f32 = 8.0;
/// Between the log area and the input line.
const GAP: f32 = 6.0;
/// Wheel notch → scrollback lines.
const WHEEL_LINES: i32 = 3;

/// The console's screen rect (logical px) in a `w`×`h` window — the top band it
/// occupies. Shared by the render loop and the shell's pointer hit test, so a
/// press over the console can never disagree with what is drawn there.
pub fn console_rect(w: f32, h: f32) -> Rect {
	Rect::new(0.0, 0.0, w, (h * PANEL_FRAC).floor())
}

/// The log ink for one scrollback line, by content: echoed commands dim, errors
/// red, output the plain log ink. (Classified at render, so the model stays
/// plain strings — the headless script suite reads those verbatim.)
fn line_color(line: &str) -> [f32; 4] {
	if line.starts_with("] ") {
		CONSOLE_ECHO
	} else if line.starts_with("error") || line.starts_with("FAILED") {
		CONSOLE_ERROR
	} else {
		CONSOLE_LOG
	}
}

pub struct ConsoleView {
	id: WidgetId,
	/// The input line. Monospace and frameless: a terminal prompt, not a box.
	input: TextInput,
	/// The visible scrollback window, oldest first — synced per frame.
	lines: Vec<String>,
	rect: Rect,
	/// Line height + baseline offset of the mono face, measured at `arrange`.
	line_h: f32,
	ascent: f32,
	/// Rows of scrollback that fit above the input line (read back by the shell
	/// to clamp the model's scrolling, the U2 "build, then read" rule).
	rows: usize,
	/// Wheel notches accumulated since the last poll (positive = back in time).
	scroll_req: i32,
	/// A submitted line (Enter in the field), waiting to be run.
	submit: Option<String>,
}

impl Default for ConsoleView {
	fn default() -> Self {
		Self::new()
	}
}

impl ConsoleView {
	pub fn new() -> Self {
		Self {
			id: wgpu_ui::next_id(),
			input: TextInput::new().role(TextRole::Mono).frameless(true),
			lines: Vec::new(),
			rect: Rect::ZERO,
			line_h: 16.0,
			ascent: 12.0,
			rows: 10,
			scroll_req: 0,
			submit: None,
		}
	}

	/// The visible scrollback window for this frame (oldest first).
	pub fn sync(&mut self, lines: Vec<String>) {
		self.lines = lines;
	}

	/// How many scrollback rows fit — the shell feeds this to
	/// [`Console::set_view_rows`](crate::console::Console::set_view_rows) so
	/// paging clamps against what is actually drawn.
	pub fn rows(&self) -> usize {
		self.rows
	}

	/// The line Enter submitted, if any (the field is cleared with it).
	pub fn take_submit(&mut self) -> Option<String> {
		self.submit.take()
	}

	/// Wheel notches since the last poll → scrollback lines (positive scrolls
	/// back in time), for [`Console::scroll_lines`](crate::console::Console::scroll_lines).
	pub fn take_scroll(&mut self) -> i32 {
		std::mem::take(&mut self.scroll_req)
	}

	/// Replace the input line (history recall), caret at the end.
	pub fn set_input(&mut self, text: impl Into<String>) {
		self.input.set_text(text);
	}

	/// The current input text (for tests and history).
	#[cfg(test)]
	pub fn input_text(&self) -> &str {
		self.input.text()
	}

	/// The arranged input-line rect — where a click lands in the field.
	#[cfg(test)]
	pub fn input_rect(&self) -> Rect {
		self.input.rect()
	}

	/// Top y of the input line within `rect`.
	fn input_top(&self) -> f32 {
		self.rect.bottom() - PAD - self.line_h
	}
}

impl Widget for ConsoleView {
	fn measure(&mut self, avail: Size, _ctx: &mut LayoutCtx) -> Size {
		avail
	}

	fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
		self.rect = rect;
		let px = ctx.theme.font_px(TextRole::Mono);
		let font = ctx.fonts.get(ctx.theme.font_for(TextRole::Mono));
		self.line_h = font.line_height(px).ceil();
		self.ascent = font.ascent() as f32 * font.scale(px);
		// The prompt is a *label* left of the field, not part of the text: the
		// field then scrolls its own content to keep the caret visible, which is
		// what retires the console's hand-rolled window-sliding arithmetic.
		let prompt_w = font.measure("] ", px);
		let top = self.input_top();
		self.rows = (((top - GAP - rect.y - PAD) / self.line_h).floor()).max(0.0) as usize;
		let x = rect.x + PAD + prompt_w;
		self.input.arrange(Rect::new(x, top, (rect.right() - PAD - x).max(0.0), self.line_h), ctx);
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		if !ctx.is_base() {
			return;
		}
		let r = self.rect;
		dl.fill_rect(r, rgba(CONSOLE_PANEL));
		dl.fill_rect(Rect::new(r.x, r.bottom() - 2.0, r.w, 2.0), rgba(CONSOLE_BORDER));
		dl.push_clip(r);

		// Prompt + the field beside it.
		let top = self.input_top();
		let base = |y: f32| Vec2::new(r.x + PAD, y + self.ascent);
		ctx.theme.text_colored(dl, ctx.fonts, base(top), "] ", TextRole::Mono, Emboss::Engraved, rgba(CONSOLE_INPUT));
		self.input.draw(dl, ctx);

		// Scrollback: newest just above the input, older lines stacking upward.
		let mut y = top - GAP - self.line_h;
		for line in self.lines.iter().rev() {
			ctx.theme.text_colored(
				dl,
				ctx.fonts,
				base(y),
				line,
				TextRole::Mono,
				Emboss::Engraved,
				rgba(line_color(line)),
			);
			y -= self.line_h;
		}
		dl.pop_clip();
	}

	fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
		// The field first: it owns the caret, the selection and the keyboard.
		let handled = self.input.event(ev, ctx);
		// Enter is the field's commit *and* the console's submit; a focus-out
		// commit is not (there is nothing to run, and the text stays for later).
		if self.input.take_commit() == Some(TextCommit::Enter) {
			self.submit = Some(self.input.text().to_string());
			self.input.set_text("");
		}
		if handled {
			return true;
		}
		match ev {
			Event::Scroll { delta, pos, .. } if self.rect.contains(*pos) => {
				let dy = match delta {
					ScrollDelta::Lines(v) => v.y * WHEEL_LINES as f32,
					ScrollDelta::Pixels(v) => v.y / self.line_h.max(1.0),
				};
				// Wheel *up* (negative dy) walks back through the scrollback.
				self.scroll_req -= dy.round() as i32;
				ctx.consume_pointer();
				true
			}
			// A press on the console plate belongs to the console (it covers the
			// chrome under it), and gives the field the caret back.
			Event::PointerButton { pressed: true, pos, .. } if self.rect.contains(*pos) => {
				ctx.request_focus(self.input.id());
				ctx.consume_pointer();
				true
			}
			_ => false,
		}
	}

	fn rect(&self) -> Rect {
		self.rect
	}

	fn id(&self) -> WidgetId {
		self.id
	}

	fn child_count(&self) -> usize {
		1
	}

	fn child(&self, i: usize) -> Option<&dyn Widget> {
		(i == 0).then_some(&self.input as &dyn Widget)
	}

	fn child_mut(&mut self, i: usize) -> Option<&mut dyn Widget> {
		(i == 0).then_some(&mut self.input as &mut dyn Widget)
	}

	fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
		if !self.rect.contains(pos) {
			return None;
		}
		// The field over its own line, the console plate everywhere else — the
		// `hit_test` override every host with a child owes its `Ui` (U3).
		self.input.hit_test(pos).or(Some(self.id))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Scrollback ink classification: echoed commands dim, errors red, output
	/// plain (the strings themselves stay verbatim for the script suite — only
	/// the render ink varies).
	#[test]
	fn log_lines_classify_by_content() {
		assert_eq!(line_color("] fit"), CONSOLE_ECHO);
		assert_eq!(line_color("error: bad verb"), CONSOLE_ERROR);
		assert_eq!(line_color("FAILED: quit: unsaved changes"), CONSOLE_ERROR);
		assert_eq!(line_color("auto-shore: 0 cells"), CONSOLE_LOG);
	}

	#[test]
	fn the_console_rect_is_the_top_band() {
		let r = console_rect(1280.0, 800.0);
		assert_eq!((r.x, r.y, r.w), (0.0, 0.0, 1280.0));
		assert_eq!(r.h, 440.0, "55% of the window height");
	}
}
