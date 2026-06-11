//! New from Image modal: configure how a decoded image maps onto
//! a new map — dimensions, coverage (crop/stretch/fill) + offset, dither and
//! dedupe method — then run a [`map_core::ConvertSession`] with a live progress
//! bar, stage label, elapsed + estimated-remaining time, and an Abort button.
//!
//! The run is stepped per frame by the shell (like the Auto Fix Shore modal),
//! so the UI stays responsive; this struct is pure UI state plus the owned
//! session. The shell drives `convert_start`/`convert_tick` through
//! `EditorState` and commits the result as a new tab.

use std::path::PathBuf;

use map_core::{ConvertOpts, ConvertSession, Coverage, Dedupe};

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

/// Dither algorithm (UI selector — only one option today, kept extensible).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DitherMethod {
	FloydSteinberg,
}

pub struct NewFromImage {
	/// The image file — only its pixels are read at Convert (the first stage),
	/// so opening the modal stays instant even for large PNGs.
	pub(crate) path: PathBuf,
	pub name: String,

	// Settings.
	pub width: String,
	pub height: String,
	pub coverage: Coverage,
	pub off_x: String,
	pub off_y: String,
	pub dither: DitherMethod,
	pub dedupe: Dedupe,
	pub threshold: String,
	focus: Option<Field>,

	// Run state (the shell drives these via `EditorState`).
	pub session: Option<ConvertSession>,
	pub running: bool,
	pub progress: f32,
	pub stage: String,
	pub elapsed: f32,

	/// A command button held down, waiting for release-inside
	/// — dragging off cancels.
	armed: Option<ArmedBtn>,
	pub(crate) drag_offset: (f32, f32),
}

/// The deferred command buttons (coverage/dedupe/fields stay press-fired).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArmedBtn {
	Cancel,
	/// Convert when idle / Abort while running — the same button.
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
			width: opts.width_tiles.to_string(),
			height: opts.height_tiles.to_string(),
			coverage: Coverage::Crop,
			off_x: "0".to_string(),
			off_y: "0".to_string(),
			dither: DitherMethod::FloydSteinberg,
			dedupe: Dedupe::Strict,
			threshold: "5".to_string(),
			focus: None,
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
		let width_tiles: u32 = self.width.parse().map_err(|_| "width is not a number")?;
		let height_tiles: u32 = self.height.parse().map_err(|_| "height is not a number")?;
		if !(1..=1024).contains(&width_tiles) || !(1..=1024).contains(&height_tiles) {
			return Err(format!("map size {width_tiles}×{height_tiles} (1..=1024 tiles)"));
		}
		let off_x = parse_signed(&self.off_x)?;
		let off_y = parse_signed(&self.off_y)?;
		let threshold = match self.dedupe {
			Dedupe::Strict => 0.0,
			Dedupe::Relaxed => {
				let pct: f32 = self.threshold.parse().map_err(|_| "threshold is not a number")?;
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
		// Buttons are live in both states — armed, firing on release-inside
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
				if self.field_rect(d, f).contains(x, y) {
					self.focus = Some(f);
					return Press::Consumed;
				}
			}
			if self.dedupe == Dedupe::Relaxed && self.field_rect(d, Field::Threshold).contains(x, y) {
				self.focus = Some(Field::Threshold);
				return Press::Consumed;
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

	pub fn on_key(&mut self, ch: Option<char>, backspace: bool, tab: bool) {
		if self.running {
			return;
		}
		if tab {
			self.focus = Some(self.next_focus());
			return;
		}
		let Some(f) = self.focus else { return };
		let field = match f {
			Field::Width => &mut self.width,
			Field::Height => &mut self.height,
			Field::OffX => &mut self.off_x,
			Field::OffY => &mut self.off_y,
			Field::Threshold => &mut self.threshold,
		};
		let signed = matches!(f, Field::OffX | Field::OffY);
		if backspace {
			field.pop();
		} else if let Some(c) = ch {
			let ok = c.is_ascii_digit() && field.len() < 5 || (signed && c == '-' && field.is_empty());
			if ok {
				field.push(c);
			}
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
		let dim = if self.running { theme::INK_DIM } else { theme::INK };

		let lab = |q: &mut UiQuads, text: &str, row: usize| {
			q.label(text, d.x + PAD, self.row_y(d, row) + 5.0, f, w, h, theme::INK_DIM);
		};
		let field = |q: &mut UiQuads, this: &Self, fld: Field, text: &str| {
			let r = this.field_rect(d, fld);
			q.field(r, w, h);
			let focused = this.focus == Some(fld) && !this.running;
			if focused {
				q.border(r, w, h, theme::INK);
			}
			q.label_in(text, r, 6.0, f, w, h, dim);
			if focused {
				let tw = crate::text::label_width(text, f);
				q.rect(Rect::new(r.x + 6.0 + tw + 1.0, r.y + 3.0, 2.0, r.h - 6.0), w, h, theme::INK);
			}
		};

		// Size.
		lab(&mut q, "size (tiles)", 0);
		field(&mut q, self, Field::Width, &self.width);
		field(&mut q, self, Field::Height, &self.height);
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
		field(&mut q, self, Field::OffX, &self.off_x);
		field(&mut q, self, Field::OffY, &self.off_y);

		// Dither.
		lab(&mut q, "dither", 3);
		let dither_name = match self.dither {
			// ASCII hyphen — the MAX atlas has no en-dash (it would vanish).
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
			field(&mut q, self, Field::Threshold, &self.threshold);
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
		assert_eq!((m.width.as_str(), m.height.as_str()), ("2", "1"));
		let opts = m.opts().unwrap();
		assert_eq!((opts.width_tiles, opts.height_tiles), (2, 1));
	}

	#[test]
	fn relaxed_threshold_parses_as_fraction() {
		let mut m = modal();
		m.dedupe = Dedupe::Relaxed;
		m.threshold = "25".into();
		assert!((m.opts().unwrap().threshold - 0.25).abs() < 1e-6);
		// Strict ignores the threshold field.
		m.dedupe = Dedupe::Strict;
		assert_eq!(m.opts().unwrap().threshold, 0.0);
	}

	#[test]
	fn offset_accepts_negative() {
		let mut m = modal();
		m.off_x = "-12".into();
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
