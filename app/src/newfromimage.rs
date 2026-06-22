//! New from Image modal: configure how a decoded image maps onto
//! a new map - dimensions, coverage (crop/stretch/fill) + offset, dither and
//! dedupe method - then run a [`map_core::ConvertSession`] with a live progress
//! bar, stage label, elapsed + estimated-remaining time, and an Abort button.
//!
//! The run is stepped per frame by the shell (like the Auto Fix Shore modal),
//! so the UI stays responsive; this struct is pure UI state plus the owned
//! session. The shell drives `convert_start`/`convert_tick` through
//! `EditorState` and commits the result as a new tab.

use std::path::PathBuf;

use map_core::{ConvertOpts, ConvertSession, Coverage, Dedupe};

use crate::textinput::{Charset, TextInput};
use crate::theme;
use crate::ui::{self, Hot, Rect, UiQuads};

const W: f32 = 372.0;
const TITLE_H: f32 = 24.0;
const ROW: f32 = 33.0;
const FIELD_W: f32 = 52.0;
const BTN_H: f32 = 23.0;
const PAD: f32 = 16.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
	Width,
	Height,
	OffX,
	OffY,
	Threshold,
}

/// Dither algorithm (UI selector - only one option today, kept extensible).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DitherMethod {
	FloydSteinberg,
}

pub struct NewFromImage {
	/// The image file - only its pixels are read at Convert (the first stage),
	/// so opening the modal stays instant even for large PNGs.
	pub(crate) path: PathBuf,
	pub name: String,

	// Settings.
	pub width: TextInput,
	pub height: TextInput,
	pub coverage: Coverage,
	pub off_x: TextInput,
	pub off_y: TextInput,
	pub dither: DitherMethod,
	pub dedupe: Dedupe,
	pub threshold: TextInput,
	focus: Option<Field>,
	/// The field a mouse-drag selection is extending (press..release).
	drag_field: Option<Field>,

	// Run state (the shell drives these via `EditorState`).
	pub session: Option<ConvertSession>,
	pub running: bool,
	pub progress: f32,
	pub stage: String,
	pub elapsed: f32,

	/// A command button held down, waiting for release-inside
	/// - dragging off cancels.
	armed: Option<ArmedBtn>,
	pub(crate) drag_offset: (f32, f32),
}

/// The deferred command buttons (coverage/dedupe/fields stay press-fired).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArmedBtn {
	Cancel,
	/// Convert when idle / Abort while running - the same button.
	Convert,
}

#[derive(Debug, PartialEq)]
pub enum Press {
	Consumed,
	/// Close the modal (Cancel / click-away when idle).
	Cancel,
	/// Begin the conversion with the current settings.
	Convert,
	/// Stop a running conversion (back to settings).
	Abort,
	SetCoverage(Coverage),
	SetDedupe(Dedupe),
}

impl NewFromImage {
	/// `src_w`/`src_h` come from the PNG header (metadata only) and just seed the
	/// default dimensions; the pixels are decoded later, at Convert.
	pub fn new(path: PathBuf, src_w: u32, src_h: u32, name: String) -> Self {
		let opts = ConvertOpts::fit_source(src_w, src_h);
		Self {
			path,
			name,
			width: TextInput::new(&opts.width_tiles.to_string(), 5).charset(Charset::Digits),
			height: TextInput::new(&opts.height_tiles.to_string(), 5).charset(Charset::Digits),
			coverage: Coverage::Crop,
			off_x: TextInput::new("0", 5).charset(Charset::Signed),
			off_y: TextInput::new("0", 5).charset(Charset::Signed),
			dither: DitherMethod::FloydSteinberg,
			dedupe: Dedupe::Strict,
			threshold: TextInput::new("5", 5).charset(Charset::Digits),
			focus: None,
			drag_field: None,
			session: None,
			running: false,
			progress: 0.0,
			stage: String::new(),
			elapsed: 0.0,
			armed: None,
			drag_offset: (0.0, 0.0),
		}
	}

	/// Parse the settings into [`ConvertOpts`] (validating ranges).
	pub fn opts(&self) -> Result<ConvertOpts, String> {
		let width_tiles: u32 = self.width.text().parse().map_err(|_| "width is not a number")?;
		let height_tiles: u32 = self.height.text().parse().map_err(|_| "height is not a number")?;
		if !(1..=1024).contains(&width_tiles) || !(1..=1024).contains(&height_tiles) {
			return Err(format!("map size {width_tiles}×{height_tiles} (1..=1024 tiles)"));
		}
		let off_x = parse_signed(self.off_x.text())?;
		let off_y = parse_signed(self.off_y.text())?;
		let threshold = match self.dedupe {
			Dedupe::Strict => 0.0,
			Dedupe::Relaxed => {
				let pct: f32 = self.threshold.text().parse().map_err(|_| "threshold is not a number")?;
				(pct / 100.0).clamp(0.0, 1.0)
			}
		};
		Ok(ConvertOpts {
			width_tiles,
			height_tiles,
			coverage: self.coverage,
			off_x,
			off_y,
			dedupe: self.dedupe,
			threshold,
		})
	}

	fn field_mut(&mut self, f: Field) -> &mut TextInput {
		match f {
			Field::Width => &mut self.width,
			Field::Height => &mut self.height,
			Field::OffX => &mut self.off_x,
			Field::OffY => &mut self.off_y,
			Field::Threshold => &mut self.threshold,
		}
	}

	fn field_ref(&self, f: Field) -> &TextInput {
		match f {
			Field::Width => &self.width,
			Field::Height => &self.height,
			Field::OffX => &self.off_x,
			Field::OffY => &self.off_y,
			Field::Threshold => &self.threshold,
		}
	}

	// ----- geometry ----------------------------------------------------------

	fn height_px(&self) -> f32 {
		TITLE_H + PAD + 5.0 * ROW + 52.0 + BTN_H + PAD
	}

	pub fn dialog_rect(&self, w: f32, h: f32) -> Rect {
		Rect::centered(w, h, W, self.height_px()).translate(self.drag_offset.0, self.drag_offset.1)
	}

	fn row_y(&self, d: Rect, row: usize) -> f32 {
		d.y + TITLE_H + PAD + row as f32 * ROW
	}

	fn field_rect(&self, d: Rect, f: Field) -> Rect {
		match f {
			Field::Width => Rect::new(d.x + 86.0, self.row_y(d, 0), FIELD_W, BTN_H),
			Field::Height => Rect::new(d.x + 86.0 + FIELD_W + 22.0, self.row_y(d, 0), FIELD_W, BTN_H),
			Field::OffX => Rect::new(d.x + 86.0, self.row_y(d, 2), FIELD_W, BTN_H),
			Field::OffY => Rect::new(d.x + 86.0 + FIELD_W + 22.0, self.row_y(d, 2), FIELD_W, BTN_H),
			Field::Threshold => Rect::new(d.x + W - PAD - 46.0, self.row_y(d, 4), 40.0, BTN_H),
		}
	}

	fn coverage_rect(&self, d: Rect, i: usize) -> Rect {
		Rect::new(d.x + 86.0 + i as f32 * 84.0, self.row_y(d, 1), 80.0, BTN_H)
	}

	fn dither_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + 86.0, self.row_y(d, 3), 168.0, BTN_H)
	}

	fn dedupe_rect(&self, d: Rect, i: usize) -> Rect {
		Rect::new(d.x + 86.0 + i as f32 * 84.0, self.row_y(d, 4), 80.0, BTN_H)
	}

	fn bar_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + PAD, self.row_y(d, 5) + 16.0, d.w - 2.0 * PAD, 12.0)
	}

	fn cancel_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + PAD, d.y + d.h - BTN_H - PAD, 90.0, BTN_H)
	}

	fn convert_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + d.w - PAD - 100.0, d.y + d.h - BTN_H - PAD, 100.0, BTN_H)
	}

	// ----- events -------------------------------------------------------------

	pub fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		// Buttons are live in both states - armed, firing on release-inside
		//.
		if self.cancel_rect(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Cancel);
			return Press::Consumed;
		}
		if self.convert_rect(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Convert);
			return Press::Consumed;
		}
		// Settings are frozen while a run is in flight.
		if !self.running {
			for f in [Field::Width, Field::Height, Field::OffX, Field::OffY] {
				let r = self.field_rect(d, f);
				if r.contains(x, y) {
					self.focus = Some(f);
					self.field_mut(f).on_press(x, y, r);
					self.drag_field = Some(f);
					return Press::Consumed;
				}
			}
			if self.dedupe == Dedupe::Relaxed {
				let r = self.field_rect(d, Field::Threshold);
				if r.contains(x, y) {
					self.focus = Some(Field::Threshold);
					self.field_mut(Field::Threshold).on_press(x, y, r);
					self.drag_field = Some(Field::Threshold);
					return Press::Consumed;
				}
			}
			for (i, c) in [Coverage::Crop, Coverage::Stretch, Coverage::Fill].into_iter().enumerate() {
				if self.coverage_rect(d, i).contains(x, y) {
					return Press::SetCoverage(c);
				}
			}
			for (i, dd) in [Dedupe::Strict, Dedupe::Relaxed].into_iter().enumerate() {
				if self.dedupe_rect(d, i).contains(x, y) {
					return Press::SetDedupe(dd);
				}
			}
			if !d.contains(x, y) {
				return Press::Cancel; // click-away closes when idle
			}
			self.focus = None;
		}
		Press::Consumed
	}

	/// Fire the armed command button if the release is still on it;
	/// a release anywhere else just disarms.
	pub fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		self.drag_field = None;
		let d = self.dialog_rect(w, h);
		match self.armed.take() {
			Some(ArmedBtn::Cancel) if self.cancel_rect(d).contains(x, y) => Press::Cancel,
			Some(ArmedBtn::Convert) if self.convert_rect(d).contains(x, y) => {
				if self.running {
					Press::Abort
				} else {
					Press::Convert
				}
			}
			_ => Press::Consumed,
		}
	}

	/// The focused field's edit state (none mid-run, when settings are frozen).
	pub fn edit_context(&self) -> Option<crate::modal::EditContext> {
		if self.running {
			return None;
		}
		let f = self.field_ref(self.focus?);
		Some(f.edit_context())
	}

	/// Route an editing key to the focused field (ignored while a run is live).
	pub fn key(&mut self, key: &crate::modal::ModalKey) {
		if self.running {
			return;
		}
		let Some(f) = self.focus else { return };
		self.field_mut(f).on_key(key);
	}

	/// Tab moves to the next field.
	pub fn focus_next(&mut self) {
		self.focus = Some(self.next_focus());
	}

	/// Mouse drag extends the active field's selection (after a press on it).
	pub fn on_drag(&mut self, x: f32, y: f32, w: f32, h: f32) {
		if let Some(f) = self.drag_field {
			let r = self.field_rect(self.dialog_rect(w, h), f);
			self.field_mut(f).on_drag(x, y, r);
		}
	}

	fn next_focus(&self) -> Field {
		let order = [Field::Width, Field::Height, Field::OffX, Field::OffY, Field::Threshold];
		let here = self.focus.and_then(|f| order.iter().position(|&o| o == f)).unwrap_or(usize::MAX);
		order[(here.wrapping_add(1)) % order.len()]
	}

	/// Estimated seconds remaining from the live progress + elapsed.
	fn eta(&self) -> Option<f32> {
		(self.running && self.progress > 0.02).then(|| self.elapsed * (1.0 - self.progress) / self.progress)
	}

	// ----- drawing -------------------------------------------------------------

	pub fn view(&self, w: f32, h: f32, hot: Hot) -> UiQuads {
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(ui::SteelMap::anchored(d));
		ui::modal_scrim(&mut q, w, h);
		ui::modal_frame(&mut q, d, "New from Image", TITLE_H, w, h);
		let f = crate::ui::FONT_SMALL;

		let lab = |q: &mut UiQuads, text: &str, row: usize| {
			q.label(text, d.x + PAD, self.row_y(d, row) + 5.0, f, w, h, theme::INK_DIM);
		};
		// The well + focus border; the editable text is drawn clipped by the shell
		// (see `field_contents`).
		let field = |q: &mut UiQuads, this: &Self, fld: Field| {
			let r = this.field_rect(d, fld);
			q.field(r, w, h);
			if this.focus == Some(fld) && !this.running {
				q.border(r, w, h, theme::INK);
			}
		};

		// Size.
		lab(&mut q, "size (tiles)", 0);
		field(&mut q, self, Field::Width);
		field(&mut q, self, Field::Height);
		let xr = self.field_rect(d, Field::Width);
		q.label_in("x", Rect::new(xr.x + FIELD_W + 6.0, xr.y, 12.0, BTN_H), 0.0, f, w, h, theme::INK_DIM);

		// Coverage.
		lab(&mut q, "coverage", 1);
		for (i, (c, name)) in
			[(Coverage::Crop, "Crop"), (Coverage::Stretch, "Stretch"), (Coverage::Fill, "Fill")].into_iter().enumerate()
		{
			let r = self.coverage_rect(d, i);
			q.toggle_button(r, name, self.coverage == c, !self.running, f, w, h, hot);
		}

		// Offset.
		lab(&mut q, "offset (px)", 2);
		field(&mut q, self, Field::OffX);
		field(&mut q, self, Field::OffY);

		// Dither.
		lab(&mut q, "dither", 3);
		let dither_name = match self.dither {
			// ASCII hyphen - the MAX atlas has no en-dash (it would vanish).
			DitherMethod::FloydSteinberg => "Floyd-Steinberg",
		};
		q.toggle_button(self.dither_rect(d), dither_name, true, !self.running, f, w, h, hot);

		// Dedupe + threshold.
		lab(&mut q, "dedupe", 4);
		for (i, (dd, name)) in [(Dedupe::Strict, "Strict"), (Dedupe::Relaxed, "Relaxed")].into_iter().enumerate() {
			let r = self.dedupe_rect(d, i);
			q.toggle_button(r, name, self.dedupe == dd, !self.running, f, w, h, hot);
		}
		if self.dedupe == Dedupe::Relaxed {
			field(&mut q, self, Field::Threshold);
			let tr = self.field_rect(d, Field::Threshold);
			q.label_in("%", Rect::new(tr.x + tr.w + 2.0, tr.y, 12.0, BTN_H), 0.0, f, w, h, theme::INK_DIM);
		}

		// Progress / stage (while running or once finished a pass).
		if self.running {
			let stage = crate::text::fit_label(&self.stage, f, d.w - 2.0 * PAD);
			q.label(&stage, d.x + PAD, self.row_y(d, 5), f, w, h, theme::INK);
			q.progress_bar(self.bar_rect(d), self.progress, None, f, w, h);
			let mut time = format!("{:.0}%   elapsed {:.1}s", self.progress * 100.0, self.elapsed);
			if let Some(eta) = self.eta() {
				time.push_str(&format!("   ~{eta:.1}s left"));
			}
			q.label(&time, d.x + PAD, self.row_y(d, 5) + 34.0, f, w, h, theme::INK_DIM);
		}

		// Buttons.
		q.button(self.cancel_rect(d), w, h, hot);
		q.label_in("Cancel", self.cancel_rect(d), 8.0, f, w, h, theme::INK_DIM);
		let cr = self.convert_rect(d);
		q.button_primary(cr, w, h, hot);
		q.label_in(if self.running { "Abort" } else { "Convert" }, cr, 8.0, f, w, h, theme::INK);
		q
	}

	/// Each editable field's text/caret/selection with its clip rect (Threshold
	/// only when the relaxed dedupe shows it). Drawn clipped to the wells.
	pub fn field_contents(&self, w: f32, h: f32) -> Vec<(UiQuads, Rect)> {
		let d = self.dialog_rect(w, h);
		let mut fields = vec![Field::Width, Field::Height, Field::OffX, Field::OffY];
		if self.dedupe == Dedupe::Relaxed {
			fields.push(Field::Threshold);
		}
		fields
			.into_iter()
			.map(|fld| {
				let r = self.field_rect(d, fld);
				(self.field_ref(fld).content_quads(r, self.focus == Some(fld) && !self.running, w, h), r)
			})
			.collect()
	}
}

/// Parse a possibly-negative integer field; empty = 0.
fn parse_signed(s: &str) -> Result<i32, String> {
	if s.is_empty() || s == "-" {
		return Ok(0);
	}
	s.parse().map_err(|_| format!("'{s}' is not a number"))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn modal() -> NewFromImage {
		NewFromImage::new(PathBuf::from("img.png"), 128, 64, "img".into())
	}

	#[test]
	fn defaults_fit_source_dimensions() {
		let m = modal();
		assert_eq!((m.width.text(), m.height.text()), ("2", "1"));
		let opts = m.opts().unwrap();
		assert_eq!((opts.width_tiles, opts.height_tiles), (2, 1));
	}

	#[test]
	fn relaxed_threshold_parses_as_fraction() {
		let mut m = modal();
		m.dedupe = Dedupe::Relaxed;
		m.threshold.set_text("25");
		assert!((m.opts().unwrap().threshold - 0.25).abs() < 1e-6);
		// Strict ignores the threshold field.
		m.dedupe = Dedupe::Strict;
		assert_eq!(m.opts().unwrap().threshold, 0.0);
	}

	#[test]
	fn offset_accepts_negative() {
		let mut m = modal();
		m.off_x.set_text("-12");
		assert_eq!(m.opts().unwrap().off_x, -12);
	}

	#[test]
	fn press_selects_coverage_and_dedupe_then_converts() {
		let mut m = modal();
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		let r = m.coverage_rect(d, 1);
		assert_eq!(m.on_press(r.x + 2.0, r.y + 2.0, w, h), Press::SetCoverage(Coverage::Stretch));
		let r = m.dedupe_rect(d, 1);
		assert_eq!(m.on_press(r.x + 2.0, r.y + 2.0, w, h), Press::SetDedupe(Dedupe::Relaxed));
		// Buttons fire on release-inside (press only arms); drag-off cancels.
		let c = m.convert_rect(d);
		assert_eq!(m.on_press(c.x + 2.0, c.y + 2.0, w, h), Press::Consumed);
		assert_eq!(m.on_release(c.x + 2.0, c.y + 2.0, w, h), Press::Convert);
		m.on_press(c.x + 2.0, c.y + 2.0, w, h);
		assert_eq!(m.on_release(2.0, 2.0, w, h), Press::Consumed, "drag-off cancels");
		let cancel = m.cancel_rect(d);
		m.on_press(cancel.x + 2.0, cancel.y + 2.0, w, h);
		assert_eq!(m.on_release(cancel.x + 2.0, cancel.y + 2.0, w, h), Press::Cancel);
	}

	#[test]
	fn running_freezes_settings_and_convert_means_abort() {
		let mut m = modal();
		m.running = true;
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		// Coverage clicks are swallowed while running.
		let r = m.coverage_rect(d, 2);
		assert_eq!(m.on_press(r.x + 2.0, r.y + 2.0, w, h), Press::Consumed);
		// The primary button aborts (press + release-inside).
		let c = m.convert_rect(d);
		m.on_press(c.x + 2.0, c.y + 2.0, w, h);
		assert_eq!(m.on_release(c.x + 2.0, c.y + 2.0, w, h), Press::Abort);
	}
}
