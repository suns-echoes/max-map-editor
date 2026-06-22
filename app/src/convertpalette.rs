//! Convert to Compatible Palette modal (Tools ▸ Palette). Picks the method -
//! **best match** (remap each used color to its nearest compatible slot) or
//! **rasterize** (render the map through its internal palette and re-import
//! it like New-from-Image) - plus the water-preservation and dedupe options,
//! then resolves to a `convert-palette …` command line (the same path
//! scripts use).
//!
//! Pure state/geometry, drawn through the shared [`UiQuads`].

use crate::textinput::{Charset, TextInput};
use crate::theme;
use crate::ui::{self, Hot, Rect, UiQuads};

const W: f32 = 380.0;
const TITLE_H: f32 = 22.0;
const ROW_H: f32 = 24.0;
const NOTE_H: f32 = 16.0;
const BTN_H: f32 = 22.0;
const PAD: f32 = 10.0;
const FONT: f32 = crate::ui::FONT_SMALL;
/// Stage line + progress bar + time line while a run is live.
const PROGRESS_H: f32 = 52.0;

/// The conversion method (mirrors the `convert-palette` command's first arg).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
	Match,
	Rasterize,
}

pub struct ConvertPalette {
	pub method: Method,
	/// Keep the water cycle blocks (96-127) animated with the map's colors.
	pub water: bool,
	/// Rasterize-only: relaxed tile dedupe (else strict).
	pub relaxed: bool,
	/// Relaxed similarity threshold, percent (text field, like New-from-Image).
	threshold: TextInput,
	threshold_focus: bool,
	/// True between a press inside the threshold field and the release (mouse-select).
	dragging_threshold: bool,
	armed: Option<ArmedBtn>,
	/// Drag offset from centered (draggable by the titlebar).
	pub(crate) drag_offset: (f32, f32),
	/// A rasterize run is in flight - the shell steps `session` per frame;
	/// the options freeze and the dialog shows stage/progress/Abort.
	pub running: bool,
	pub session: Option<map_core::PaletteReimport>,
	pub progress: f32,
	pub stage: String,
	/// Wall-clock seconds since Convert (display + ETA), stamped by the shell.
	pub elapsed: f32,
}

/// The deferred command buttons (method/option toggles stay press-fired).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArmedBtn {
	Cancel,
	Convert,
}

#[derive(Debug, PartialEq)]
pub enum Press {
	Consumed,
	Close,
	/// Validated `convert-palette …` command line (the best-match method -
	/// instant, runs straight through the command path).
	Convert(String),
	/// Begin the stepped rasterize run - the shell drives it per frame.
	StartRasterize,
	/// Abort the running rasterize run (back to the options).
	Abort,
	Invalid(String),
}

impl ConvertPalette {
	pub fn new() -> Self {
		Self {
			method: Method::Match,
			water: true,
			relaxed: false,
			threshold: TextInput::new("5", 5).charset(Charset::Decimal),
			threshold_focus: false,
			dragging_threshold: false,
			armed: None,
			drag_offset: (0.0, 0.0),
			running: false,
			session: None,
			progress: 0.0,
			stage: String::new(),
			elapsed: 0.0,
		}
	}

	/// The rasterize dedupe settings, validated: `(relaxed, threshold 0..1)`.
	pub fn dedupe_opts(&self) -> Result<(bool, f32), String> {
		if !self.relaxed {
			return Ok((false, 0.0));
		}
		let pct: f32 = self.threshold.text().parse().map_err(|_| "threshold is not a number".to_string())?;
		if !(0.0..=100.0).contains(&pct) {
			return Err(format!("threshold {pct}% (0..=100)"));
		}
		Ok((true, pct / 100.0))
	}

	/// Estimated seconds remaining from the live progress + elapsed.
	fn eta(&self) -> Option<f32> {
		(self.running && self.progress > 0.02).then(|| self.elapsed * (1.0 - self.progress) / self.progress)
	}

	pub fn command(&self) -> Result<String, String> {
		let mut line = String::from("convert-palette ");
		line.push_str(match self.method {
			Method::Match => "match",
			Method::Rasterize => "rasterize",
		});
		line.push_str(if self.water { " water=keep" } else { " water=drop" });
		if self.method == Method::Rasterize {
			line.push_str(if self.relaxed { " dedupe=relaxed" } else { " dedupe=strict" });
			if self.relaxed {
				let pct: f32 = self.threshold.text().parse().map_err(|_| "threshold is not a number")?;
				if !(0.0..=100.0).contains(&pct) {
					return Err(format!("threshold {pct}% (0..=100)"));
				}
				line.push_str(&format!(" threshold={pct}"));
			}
		}
		Ok(line)
	}

	// ----- geometry ----------------------------------------------------------

	/// method row + 2 note lines + water row + (rasterize: dedupe row) +
	/// (running: stage/bar/time strip) + buttons.
	fn height(&self) -> f32 {
		let dedupe = if self.method == Method::Rasterize { ROW_H } else { 0.0 };
		let progress = if self.running { PROGRESS_H } else { 0.0 };
		TITLE_H + 8.0 + ROW_H + 2.0 * NOTE_H + 6.0 + ROW_H + dedupe + progress + 14.0 + BTN_H + 12.0
	}

	/// The progress strip (stage line + bar + time line) while running.
	fn progress_origin(&self, d: Rect) -> f32 {
		self.dedupe_rect(d, false).y + ROW_H
	}

	pub fn dialog_rect(&self, w: f32, h: f32) -> Rect {
		Rect::centered(w, h, W, self.height()).translate(self.drag_offset.0, self.drag_offset.1)
	}

	fn method_rect(&self, d: Rect, m: Method) -> Rect {
		let y = d.y + TITLE_H + 8.0;
		let w = (d.w - 2.0 * PAD - 70.0 - 6.0) / 2.0;
		match m {
			Method::Match => Rect::new(d.x + PAD + 70.0, y, w, BTN_H),
			Method::Rasterize => Rect::new(d.x + PAD + 70.0 + w + 6.0, y, w, BTN_H),
		}
	}

	fn water_rect(&self, d: Rect) -> Rect {
		let y = d.y + TITLE_H + 8.0 + ROW_H + 2.0 * NOTE_H + 6.0;
		Rect::new(d.x + PAD + 70.0, y, d.w - 2.0 * PAD - 70.0, BTN_H)
	}

	fn dedupe_rect(&self, d: Rect, relaxed: bool) -> Rect {
		let y = self.water_rect(d).y + ROW_H;
		let w = 70.0;
		let x = d.x + PAD + 70.0;
		if relaxed { Rect::new(x + w + 6.0, y, w, BTN_H) } else { Rect::new(x, y, w, BTN_H) }
	}

	fn threshold_rect(&self, d: Rect) -> Rect {
		let r = self.dedupe_rect(d, true);
		Rect::new(r.x + r.w + 6.0, r.y, 44.0, BTN_H)
	}

	fn cancel_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + PAD, d.y + d.h - BTN_H - 10.0, 90.0, BTN_H)
	}

	fn convert_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + d.w - PAD - 90.0, d.y + d.h - BTN_H - 10.0, 90.0, BTN_H)
	}

	// ----- events -------------------------------------------------------------

	pub fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		// Buttons stay live in both states - armed, firing on release-inside.
		if self.cancel_rect(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Cancel);
			return Press::Consumed;
		}
		if self.convert_rect(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Convert);
			return Press::Consumed;
		}
		// Options are frozen while a run is in flight.
		if !self.running {
			for m in [Method::Match, Method::Rasterize] {
				if self.method_rect(d, m).contains(x, y) {
					self.method = m;
					return Press::Consumed;
				}
			}
			if self.water_rect(d).contains(x, y) {
				self.water = !self.water;
				return Press::Consumed;
			}
			if self.method == Method::Rasterize {
				for relaxed in [false, true] {
					if self.dedupe_rect(d, relaxed).contains(x, y) {
						self.relaxed = relaxed;
						return Press::Consumed;
					}
				}
				if self.relaxed && self.threshold_rect(d).contains(x, y) {
					self.threshold_focus = true;
					self.threshold.on_press(x, y, self.threshold_rect(d));
					self.dragging_threshold = true;
					return Press::Consumed;
				}
			}
			if !d.contains(x, y) {
				return Press::Close; // click-away closes when idle
			}
			self.threshold_focus = false;
		}
		Press::Consumed
	}

	/// Fire the armed command button if the release is still on it;
	/// a release anywhere else just disarms.
	pub fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		self.dragging_threshold = false;
		let d = self.dialog_rect(w, h);
		match self.armed.take() {
			Some(ArmedBtn::Cancel) if self.cancel_rect(d).contains(x, y) => Press::Close,
			Some(ArmedBtn::Convert) if self.convert_rect(d).contains(x, y) => {
				if self.running {
					Press::Abort
				} else {
					self.confirm()
				}
			}
			_ => Press::Consumed,
		}
	}

	/// The threshold field's edit state when it's focused (and not mid-run).
	pub fn edit_context(&self) -> Option<crate::modal::EditContext> {
		(!self.running && self.threshold_focus).then(|| self.threshold.edit_context())
	}

	/// Route an editing key to the threshold field (when focused and idle).
	pub fn key(&mut self, key: &crate::modal::ModalKey) {
		if self.running || !self.threshold_focus {
			return;
		}
		self.threshold.on_key(key);
	}

	/// Mouse drag extends the threshold field's selection (after a press on it).
	pub fn on_drag(&mut self, x: f32, y: f32, w: f32, h: f32) {
		if self.dragging_threshold {
			let r = self.threshold_rect(self.dialog_rect(w, h));
			self.threshold.on_drag(x, y, r);
		}
	}

	pub fn confirm(&self) -> Press {
		match (self.method, self.command()) {
			// Rasterize is a stepped run the shell drives (progress + Abort);
			// best match is instant and goes through the command path.
			(Method::Rasterize, Ok(_)) => Press::StartRasterize,
			(_, Ok(line)) => Press::Convert(line),
			(_, Err(e)) => Press::Invalid(e),
		}
	}

	// ----- drawing -------------------------------------------------------------

	pub fn view(&self, w: f32, h: f32, hot: Hot) -> UiQuads {
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(ui::SteelMap::anchored(d));
		ui::modal_scrim(&mut q, w, h);
		ui::modal_frame(&mut q, d, "Convert to Compatible Palette", TITLE_H, w, h);

		// Method toggles + a short note on what the chosen one does.
		let live = !self.running;
		let my = self.method_rect(d, Method::Match).y;
		q.label("method", d.x + PAD, my + 5.0, FONT, w, h, theme::INK_DIM);
		for (m, name) in [(Method::Match, "best match"), (Method::Rasterize, "rasterize")] {
			q.toggle_button(self.method_rect(d, m), name, self.method == m, live, FONT, w, h, hot);
		}
		let note: [&str; 2] = match self.method {
			Method::Match => ["remap each used color to its", "nearest game-legal slot"],
			Method::Rasterize => ["render with the internal palette,", "re-import like New from Image"],
		};
		for (i, line) in note.iter().enumerate() {
			q.label(line, d.x + PAD + 70.0, my + BTN_H + 4.0 + i as f32 * NOTE_H, FONT, w, h, theme::INK_DIM);
		}

		// Water preservation checkbox.
		let wr = self.water_rect(d);
		q.label("water", d.x + PAD, wr.y + 5.0, FONT, w, h, theme::INK_DIM);
		q.toggle_button(wr, "keep animated water colors", self.water, live, FONT, w, h, hot);

		// Rasterize-only: dedupe + relaxed threshold (percent).
		if self.method == Method::Rasterize {
			let dr = self.dedupe_rect(d, false);
			q.label("dedupe", d.x + PAD, dr.y + 5.0, FONT, w, h, theme::INK_DIM);
			for (relaxed, name) in [(false, "strict"), (true, "relaxed")] {
				q.toggle_button(self.dedupe_rect(d, relaxed), name, self.relaxed == relaxed, live, FONT, w, h, hot);
			}
			if self.relaxed {
				let r = self.threshold_rect(d);
				q.field(r, w, h);
				if self.threshold_focus && live {
					q.border(r, w, h, theme::INK);
				}
				// The threshold value/caret are drawn clipped by `field_contents`.
				q.label_in("%", Rect::new(r.x + r.w + 4.0, r.y, 14.0, r.h), 0.0, FONT, w, h, theme::INK_DIM);
			}
		}

		// Live run: stage + progress bar + elapsed/ETA (the shell steps the
		// session per frame and stamps `progress`/`stage`/`elapsed`).
		if self.running {
			let py = self.progress_origin(d);
			let stage = crate::text::fit_label(&self.stage, FONT, d.w - 2.0 * PAD);
			q.label(&stage, d.x + PAD, py, FONT, w, h, theme::INK);
			q.progress_bar(Rect::new(d.x + PAD, py + 16.0, d.w - 2.0 * PAD, 12.0), self.progress, None, FONT, w, h);
			let mut time = format!("{:.0}%   elapsed {:.1}s", self.progress * 100.0, self.elapsed);
			if let Some(eta) = self.eta() {
				time.push_str(&format!("   ~{eta:.1}s left"));
			}
			q.label(&time, d.x + PAD, py + 34.0, FONT, w, h, theme::INK_DIM);
		}

		q.button(self.cancel_rect(d), w, h, hot);
		q.label_in("Cancel", self.cancel_rect(d), 8.0, FONT, w, h, theme::INK_DIM);
		q.button_primary(self.convert_rect(d), w, h, hot);
		q.label_in(if self.running { "Abort" } else { "Convert" }, self.convert_rect(d), 8.0, FONT, w, h, theme::INK);
		q
	}

	/// The threshold field's text/caret/selection with its clip rect - present
	/// only when the relaxed-rasterize threshold field is shown.
	pub fn field_contents(&self, w: f32, h: f32) -> Vec<(UiQuads, Rect)> {
		if self.method != Method::Rasterize || !self.relaxed {
			return Vec::new();
		}
		let r = self.threshold_rect(self.dialog_rect(w, h));
		vec![(self.threshold.content_quads(r, self.threshold_focus && !self.running, w, h), r)]
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::modal::ModalKey;

	#[test]
	fn command_lines_cover_the_option_space() {
		let mut m = ConvertPalette::new();
		assert_eq!(m.command().unwrap(), "convert-palette match water=keep");
		m.water = false;
		assert_eq!(m.command().unwrap(), "convert-palette match water=drop");
		m.method = Method::Rasterize;
		assert_eq!(m.command().unwrap(), "convert-palette rasterize water=drop dedupe=strict");
		m.relaxed = true;
		m.water = true;
		assert_eq!(m.command().unwrap(), "convert-palette rasterize water=keep dedupe=relaxed threshold=5");
		m.threshold.set_text("x");
		assert!(m.command().is_err());
		m.threshold.set_text("250");
		assert!(m.command().is_err());
	}

	#[test]
	fn presses_toggle_options_and_confirm_arms() {
		let mut m = ConvertPalette::new();
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		let r = m.method_rect(d, Method::Rasterize);
		assert_eq!(m.on_press(r.x + 2.0, r.y + 2.0, w, h), Press::Consumed);
		assert_eq!(m.method, Method::Rasterize);
		// The rasterize variant grows the dialog (dedupe row) - re-resolve.
		let d = m.dialog_rect(w, h);
		let wr = m.water_rect(d);
		m.on_press(wr.x + 2.0, wr.y + 2.0, w, h);
		assert!(!m.water);
		let dd = m.dedupe_rect(d, true);
		m.on_press(dd.x + 2.0, dd.y + 2.0, w, h);
		assert!(m.relaxed);
		// Threshold field takes digits only once focused (caret to End, then type).
		let tf = m.threshold_rect(d);
		m.on_press(tf.x + 2.0, tf.y + 2.0, w, h);
		m.key(&ModalKey::End { shift: false });
		m.key(&ModalKey::Char('0'));
		assert_eq!(m.threshold.text(), "50");
		// Convert fires on release-inside; drag-off cancels. Rasterize is a
		// stepped run, so confirming it starts the job rather than running a
		// command line.
		let c = m.convert_rect(d);
		assert_eq!(m.on_press(c.x + 2.0, c.y + 2.0, w, h), Press::Consumed);
		assert_eq!(m.on_release(c.x + 2.0, c.y + 2.0, w, h), Press::StartRasterize);
		m.method = Method::Match;
		let d = m.dialog_rect(w, h);
		let c = m.convert_rect(d);
		m.on_press(c.x + 2.0, c.y + 2.0, w, h);
		assert!(matches!(m.on_release(c.x + 2.0, c.y + 2.0, w, h), Press::Convert(_)));
		m.on_press(c.x + 2.0, c.y + 2.0, w, h);
		assert_eq!(m.on_release(2.0, 2.0, w, h), Press::Consumed, "drag-off cancels");
		// Click-out closes.
		assert_eq!(m.on_press(2.0, 2.0, w, h), Press::Close);
	}

	#[test]
	fn running_freezes_options_and_converts_the_button_to_abort() {
		let mut m = ConvertPalette::new();
		m.method = Method::Rasterize;
		m.running = true;
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		// The running dialog grows a progress strip.
		let mut idle = ConvertPalette::new();
		idle.method = Method::Rasterize;
		assert!(d.h > idle.dialog_rect(w, h).h);
		// Options are frozen: clicking a method toggle changes nothing.
		let r = m.method_rect(d, Method::Match);
		assert_eq!(m.on_press(r.x + 2.0, r.y + 2.0, w, h), Press::Consumed);
		assert_eq!(m.method, Method::Rasterize);
		// Click-out doesn't close a live run.
		assert_eq!(m.on_press(2.0, 2.0, w, h), Press::Consumed);
		// The primary button aborts instead of confirming.
		let c = m.convert_rect(d);
		m.on_press(c.x + 2.0, c.y + 2.0, w, h);
		assert_eq!(m.on_release(c.x + 2.0, c.y + 2.0, w, h), Press::Abort);
		// Typing is ignored while running.
		m.threshold_focus = true;
		m.key(&ModalKey::Char('9'));
		assert_eq!(m.threshold.text(), "5");
	}
}
