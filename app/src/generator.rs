//! Generate Random Terrain modal: pattern pick + water /
//! obstruction / decoration %, optional seed, shore method. Generate starts
//! a stepped [`map_core::GenSession`] the shell drives per frame - a
//! progress bar fills, the Generate button becomes Abort, and the UI never
//! freezes. The modal **stays open**, so seeds can be rerolled until the
//! map looks right (every run is one undo unit; leave the seed field empty
//! for fresh randomness each press).
//!
//! Pure UI state here (plus the owned session); the shell drives `step`
//! and abort through `EditorState` so it can borrow the project.

use map_core::{GenParams, GenPattern, GenSession};

use crate::textinput::{Charset, TextInput};
use crate::theme;
use crate::ui::{self, Hot, Rect, UiQuads};

const W: f32 = 380.0;
const TITLE_H: f32 = 22.0;
const ROW_H: f32 = 24.0;
const FIELD_W: f32 = 64.0;
const SEED_W: f32 = 180.0;
const BTN_H: f32 = 20.0;
/// Pattern buttons: 2×2 grid.
const PAT_W: f32 = 122.0;
/// Line spacing of the post-run status report (FONT_SMALL + leading).
const STATUS_LINE_H: f32 = 16.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
	Water,
	Obstructions,
	Decorations,
	Seed,
}

pub struct Generator {
	pub pattern: GenPattern,
	pub water: TextInput,
	pub obstructions: TextInput,
	pub decorations: TextInput,
	/// Empty = fresh random seed every Generate.
	pub seed: TextInput,
	/// Shore the coastlines with the loop-walk pass (Auto Shore ALT).
	pub alt_shore: bool,
	pub focus: Option<Field>,
	/// The field being mouse-drag-selected (press..release).
	drag_field: Option<Field>,
	pub running: bool,
	/// The live generation run (owned here; stepped by the shell).
	pub session: Option<GenSession>,
	/// The running/last run's params (seed resolved) - progress + report.
	pub started: Option<GenParams>,
	/// Result report under the controls (seed used, stats, abort note) - one
	/// entry per line, so the dialog grows instead of cropping a long line.
	pub status: Vec<String>,
	/// A command button held down, waiting for its release-inside to fire
	/// - dragging off cancels.
	armed: Option<ArmedBtn>,
	/// Drag offset from centered (draggable by the titlebar).
	pub(crate) drag_offset: (f32, f32),
}

/// The deferred command buttons (selections and fields stay press-fired).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArmedBtn {
	Close,
	/// Generate when idle / Abort while running - the same button.
	Generate,
}

/// What a press resolved to (everything is consumed while a modal is open).
#[derive(Debug, PartialEq)]
pub enum Press {
	Consumed,
	Close,
	/// Start a run with the current settings. The modal stays open -
	/// progress while running, ready for the next reroll once done.
	Start,
	/// Abort the live run (the same button, relabelled while running).
	Abort,
	/// Validation failed - show this in the console, keep the modal open.
	Invalid(String),
}

impl Generator {
	pub fn new() -> Self {
		Self {
			pattern: GenPattern::Islands,
			water: TextInput::new("45", 3).charset(Charset::Digits),
			obstructions: TextInput::new("10", 3).charset(Charset::Digits),
			decorations: TextInput::new("5", 3).charset(Charset::Digits),
			seed: TextInput::new("", 20).charset(Charset::Digits),
			alt_shore: false,
			focus: None,
			drag_field: None,
			running: false,
			session: None,
			started: None,
			status: Vec::new(),
			armed: None,
			drag_offset: (0.0, 0.0),
		}
	}

	/// The validated settings (`None` seed = the caller rolls a fresh one),
	/// or what's wrong with them.
	pub fn params(&self) -> Result<(GenParams, Option<u64>), String> {
		let water: u8 = self.water.text().parse().map_err(|_| "water % is not a number".to_string())?;
		let obstructions: u8 =
			self.obstructions.text().parse().map_err(|_| "obstructions % is not a number".to_string())?;
		let decorations: u8 =
			self.decorations.text().parse().map_err(|_| "decorations % is not a number".to_string())?;
		if water > 100 || obstructions > 100 || decorations > 100 {
			return Err("percentages are 0..=100".into());
		}
		let seed = match self.seed.text().is_empty() {
			true => None,
			false => Some(self.seed.text().parse::<u64>().map_err(|_| "seed is not a number (u64)".to_string())?),
		};
		let params = GenParams {
			pattern: self.pattern,
			water,
			obstructions,
			decorations,
			seed: seed.unwrap_or(0),
			alt_shore: self.alt_shore,
		};
		Ok((params, seed))
	}

	// ----- geometry ----------------------------------------------------------

	/// Rows: 0-1 pattern grid, 2 water, 3 obstructions, 4 decorations,
	/// 5 seed, 6 shore method. Row 7 holds the progress bar / first status
	/// line; further status lines grow the dialog.
	pub fn dialog_rect(&self, w: f32, h: f32) -> Rect {
		let extra = self.status.len().saturating_sub(1) as f32 * STATUS_LINE_H;
		let dh = TITLE_H + 8.0 + 8.0 * (ROW_H + 4.0) + extra + BTN_H + 18.0;
		Rect::centered(w, h, W, dh).translate(self.drag_offset.0, self.drag_offset.1)
	}

	fn row_y(d: Rect, row: usize) -> f32 {
		d.y + TITLE_H + 8.0 + row as f32 * (ROW_H + 4.0)
	}

	fn pattern_rect(d: Rect, i: usize) -> Rect {
		Rect::new(d.x + 110.0 + (i % 2) as f32 * (PAT_W + 6.0), Self::row_y(d, i / 2), PAT_W, BTN_H)
	}

	fn field_rect(d: Rect, f: Field) -> Rect {
		match f {
			Field::Water => Rect::new(d.x + 110.0, Self::row_y(d, 2), FIELD_W, BTN_H),
			Field::Obstructions => Rect::new(d.x + 110.0, Self::row_y(d, 3), FIELD_W, BTN_H),
			Field::Decorations => Rect::new(d.x + 110.0, Self::row_y(d, 4), FIELD_W, BTN_H),
			Field::Seed => Rect::new(d.x + 110.0, Self::row_y(d, 5), SEED_W, BTN_H),
		}
	}

	/// The two shore-method buttons (row 6): sweep, loop-walk.
	fn shore_rect(d: Rect, alt: bool) -> Rect {
		Rect::new(d.x + 110.0 + if alt { PAT_W + 6.0 } else { 0.0 }, Self::row_y(d, 6), PAT_W, BTN_H)
	}

	fn close_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + 10.0, d.y + d.h - BTN_H - 10.0, 90.0, BTN_H)
	}

	fn generate_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + d.w - 110.0, d.y + d.h - BTN_H - 10.0, 100.0, BTN_H)
	}

	/// "Copy Seed" button (centred in the button row) — shown once a run has
	/// reported a seed, so the exact seed is one click from the clipboard.
	fn copy_seed_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + d.w / 2.0 - 50.0, d.y + d.h - BTN_H - 10.0, 100.0, BTN_H)
	}

	/// The seed of the last/running generation, once one exists.
	fn reported_seed(&self) -> Option<u64> {
		self.started.as_ref().map(|p| p.seed)
	}

	fn field_mut(&mut self, f: Field) -> &mut TextInput {
		match f {
			Field::Water => &mut self.water,
			Field::Obstructions => &mut self.obstructions,
			Field::Decorations => &mut self.decorations,
			Field::Seed => &mut self.seed,
		}
	}

	fn field_ref(&self, f: Field) -> &TextInput {
		match f {
			Field::Water => &self.water,
			Field::Obstructions => &self.obstructions,
			Field::Decorations => &self.decorations,
			Field::Seed => &self.seed,
		}
	}

	// ----- events --------------------------------------------------------------

	pub fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		if self.running {
			// Only the (relabelled) Abort button works mid-run - armed, so a
			// drag-off can still cancel the click.
			if self.generate_rect(d).contains(x, y) {
				self.armed = Some(ArmedBtn::Generate);
			}
			return Press::Consumed;
		}
		for (i, pattern) in GenPattern::ALL.into_iter().enumerate() {
			if Self::pattern_rect(d, i).contains(x, y) {
				self.pattern = pattern;
				return Press::Consumed;
			}
		}
		for f in [Field::Water, Field::Obstructions, Field::Decorations, Field::Seed] {
			let r = Self::field_rect(d, f);
			if r.contains(x, y) {
				self.focus = Some(f);
				self.field_mut(f).on_press(x, y, r);
				self.drag_field = Some(f);
				return Press::Consumed;
			}
		}
		for alt in [false, true] {
			if Self::shore_rect(d, alt).contains(x, y) {
				self.alt_shore = alt;
				return Press::Consumed;
			}
		}
		if let Some(seed) = self.reported_seed() {
			if self.copy_seed_rect(d).contains(x, y) {
				crate::textinput::clipboard_set(&seed.to_string());
				return Press::Consumed;
			}
		}
		if self.close_rect(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Close);
			return Press::Consumed;
		}
		if self.generate_rect(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Generate);
			return Press::Consumed;
		}
		self.focus = None;
		Press::Consumed // modal: everything else is swallowed
	}

	/// Fire the armed command button if the release is still on it;
	/// a release anywhere else just disarms.
	pub fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		self.drag_field = None;
		let d = self.dialog_rect(w, h);
		match self.armed.take() {
			Some(ArmedBtn::Close) if self.close_rect(d).contains(x, y) && !self.running => Press::Close,
			Some(ArmedBtn::Generate) if self.generate_rect(d).contains(x, y) => {
				if self.running {
					Press::Abort
				} else {
					match self.params() {
						Ok(_) => Press::Start,
						Err(e) => Press::Invalid(e),
					}
				}
			}
			_ => Press::Consumed,
		}
	}

	/// Mouse drag extends the active field's selection (after a press on it).
	pub fn on_drag(&mut self, x: f32, y: f32, w: f32, h: f32) {
		if let Some(f) = self.drag_field {
			let r = Self::field_rect(self.dialog_rect(w, h), f);
			self.field_mut(f).on_drag(x, y, r);
		}
	}

	/// The focused field's edit state (none mid-run, when fields are frozen).
	pub fn edit_context(&self) -> Option<crate::modal::EditContext> {
		if self.running {
			return None;
		}
		Some(self.field_ref(self.focus?).edit_context())
	}

	/// Route an editing key to the focused field (digits only; ignored mid-run).
	pub fn key(&mut self, key: &crate::modal::ModalKey) {
		if self.running {
			return;
		}
		let Some(f) = self.focus else { return };
		self.field_mut(f).on_key(key);
	}

	/// Tab advances focus through the fields (water → … → seed → water).
	pub fn focus_next(&mut self) {
		self.focus = Some(match self.focus {
			Some(Field::Water) => Field::Obstructions,
			Some(Field::Obstructions) => Field::Decorations,
			Some(Field::Decorations) => Field::Seed,
			_ => Field::Water,
		});
	}

	// ----- drawing --------------------------------------------------------------

	pub fn view(&self, w: f32, h: f32, hot: Hot) -> UiQuads {
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(ui::SteelMap::anchored(d));
		ui::modal_scrim(&mut q, w, h);
		ui::modal_frame(&mut q, d, "Generate Random Terrain", TITLE_H, w, h);

		let label_x = d.x + 10.0;
		for (name, row) in
			[("pattern", 0usize), ("water %", 2), ("obstruct %", 3), ("decor %", 4), ("seed", 5), ("shore", 6)]
		{
			q.label(name, label_x, Self::row_y(d, row) + 4.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		}

		for (i, pattern) in GenPattern::ALL.into_iter().enumerate() {
			let r = Self::pattern_rect(d, i);
			q.toggle_button(
				r,
				pattern.label(),
				self.pattern == pattern,
				!self.running,
				crate::ui::FONT_SMALL,
				w,
				h,
				hot,
			);
		}

		for f in [Field::Water, Field::Obstructions, Field::Decorations, Field::Seed] {
			let r = Self::field_rect(d, f);
			q.field(r, w, h);
			let focused = self.focus == Some(f);
			if focused {
				q.border(r, w, h, theme::INK);
			}
			// An empty, unfocused seed shows a "random" hint; every field's value
			// text + caret is drawn (clipped) by `field_contents`.
			if f == Field::Seed && self.seed.text().is_empty() && !focused {
				q.label_in("random", r, 6.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
			}
		}

		for (alt, label) in [(false, "Auto Shore"), (true, "Auto Shore ALT")] {
			let r = Self::shore_rect(d, alt);
			q.toggle_button(r, label, self.alt_shore == alt, !self.running, crate::ui::FONT_SMALL, w, h, hot);
		}

		// Row 7: the live progress bar, or the last run's status line.
		let py = Self::row_y(d, 7);
		if let (true, Some(session)) = (self.running, &self.session) {
			let (label, frac) = session.progress();
			q.label(label, d.x + 12.0, py + 4.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
			let bar = Rect::new(d.x + 110.0, py + 2.0, PAT_W * 2.0 + 6.0, BTN_H - 4.0);
			q.progress_bar(bar, frac, Some(&format!("{:.0}%", frac * 100.0)), crate::ui::FONT_SMALL, w, h);
		} else {
			// The post-run report, one line each - the dialog grew to fit
			// (dialog_rect), each line still ellipsis-guarded.
			for (i, line) in self.status.iter().enumerate() {
				let line = crate::text::fit_label(line, crate::ui::FONT_SMALL, d.w - 24.0);
				let ly = py + 4.0 + i as f32 * STATUS_LINE_H;
				q.label(&line, d.x + 12.0, ly, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
			}
		}

		// Close is locked mid-run (Abort first) - show it that way.
		if self.running {
			q.button_disabled(self.close_rect(d), w, h);
		} else {
			q.button(self.close_rect(d), w, h, hot);
		}
		q.label_in("Close", self.close_rect(d), 8.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		// Copy Seed: available once a run has reported a seed (and not mid-run).
		if !self.running && self.reported_seed().is_some() {
			let r = self.copy_seed_rect(d);
			q.button(r, w, h, hot);
			q.label_in("Copy Seed", r, 8.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		}
		q.button_primary(self.generate_rect(d), w, h, hot);
		let label = if self.running { "Abort" } else { "Generate" };
		q.label_in(label, self.generate_rect(d), 8.0, crate::ui::FONT_SMALL, w, h, theme::INK);
		q
	}

	/// Each field's text/caret/selection, with the clip rect the shell scissors
	/// it to (so long values stay in the well).
	pub fn field_contents(&self, w: f32, h: f32) -> Vec<(UiQuads, Rect)> {
		let d = self.dialog_rect(w, h);
		[Field::Water, Field::Obstructions, Field::Decorations, Field::Seed]
			.into_iter()
			.map(|f| {
				let r = Self::field_rect(d, f);
				(self.field_ref(f).content_quads(r, self.focus == Some(f), w, h), r)
			})
			.collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::modal::ModalKey;

	#[test]
	fn params_build_and_validate() {
		let mut m = Generator::new();
		let (p, seed) = m.params().unwrap();
		assert_eq!(
			(p.pattern, p.water, p.obstructions, p.decorations, p.alt_shore),
			(GenPattern::Islands, 45, 10, 5, false)
		);
		assert_eq!(seed, None, "empty seed field = roll a fresh one");
		m.pattern = GenPattern::RiverRaid;
		m.water.set_text("30");
		m.seed.set_text("42");
		m.alt_shore = true;
		let (p, seed) = m.params().unwrap();
		assert_eq!((p.pattern, p.water, p.alt_shore), (GenPattern::RiverRaid, 30, true));
		assert_eq!(seed, Some(42));
		m.water.set_text("101");
		assert!(m.params().is_err());
		m.water.set_text("");
		assert!(m.params().is_err());
	}

	#[test]
	fn press_flow_pattern_fields_generate() {
		let mut m = Generator::new();
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		// Each pattern button selects.
		for (i, pattern) in GenPattern::ALL.into_iter().enumerate() {
			let r = Generator::pattern_rect(d, i);
			assert_eq!(m.on_press(r.x + 2.0, r.y + 2.0, w, h), Press::Consumed);
			assert_eq!(m.pattern, pattern);
		}
		// Focus + type into the water field.
		let r = Generator::field_rect(d, Field::Water);
		m.on_press(r.x + 2.0, r.y + 2.0, w, h);
		assert_eq!(m.focus, Some(Field::Water));
		m.water.set_text("");
		for c in "60x".chars() {
			m.key(&ModalKey::Char(c)); // non-digit ignored
		}
		assert_eq!(m.water.text(), "60");
		m.focus_next(); // tab → obstructions
		assert_eq!(m.focus, Some(Field::Obstructions));
		m.focus_next(); // tab → decorations
		assert_eq!(m.focus, Some(Field::Decorations));
		// The shore-method buttons toggle the pass.
		let alt = Generator::shore_rect(d, true);
		assert_eq!(m.on_press(alt.x + 2.0, alt.y + 2.0, w, h), Press::Consumed);
		assert!(m.alt_shore);
		// The Generate button fires on release-inside (press only arms it).
		let g = m.generate_rect(d);
		assert_eq!(m.on_press(g.x + 2.0, g.y + 2.0, w, h), Press::Consumed);
		assert_eq!(m.on_release(g.x + 2.0, g.y + 2.0, w, h), Press::Start);
		// Dragging off before release cancels the click.
		assert_eq!(m.on_press(g.x + 2.0, g.y + 2.0, w, h), Press::Consumed);
		assert_eq!(m.on_release(2.0, 2.0, w, h), Press::Consumed, "drag-off cancels");
		m.water.set_text("x");
		m.on_press(g.x + 2.0, g.y + 2.0, w, h);
		assert!(matches!(m.on_release(g.x + 2.0, g.y + 2.0, w, h), Press::Invalid(_)));
		m.water.set_text("60");
		// Close bubbles (on release); clicks outside are swallowed (focus drops).
		let c = m.close_rect(d);
		m.on_press(c.x + 2.0, c.y + 2.0, w, h);
		assert_eq!(m.on_release(c.x + 2.0, c.y + 2.0, w, h), Press::Close);
		assert_eq!(m.on_press(2.0, 2.0, w, h), Press::Consumed);
		assert_eq!(m.focus, None);
	}

	#[test]
	fn running_locks_the_controls_and_offers_abort() {
		let mut m = Generator::new();
		m.running = true;
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		// The (relabelled) Generate button aborts (press + release-inside);
		// everything else is inert.
		let g = m.generate_rect(d);
		assert_eq!(m.on_press(g.x + 2.0, g.y + 2.0, w, h), Press::Consumed);
		assert_eq!(m.on_release(g.x + 2.0, g.y + 2.0, w, h), Press::Abort);
		let c = m.close_rect(d);
		m.on_press(c.x + 2.0, c.y + 2.0, w, h);
		assert_eq!(m.on_release(c.x + 2.0, c.y + 2.0, w, h), Press::Consumed, "no Close mid-run");
		let r = Generator::pattern_rect(d, 1);
		m.on_press(r.x + 2.0, r.y + 2.0, w, h);
		assert_eq!(m.pattern, GenPattern::Islands, "pattern locked mid-run");
	}

	#[test]
	fn status_lines_grow_the_dialog() {
		let mut m = Generator::new();
		let (w, h) = (1280.0, 800.0);
		let before = m.dialog_rect(w, h);
		m.status = vec!["islands: seed 42".into(), "100 water / 200 land cells".into(), "5 shore tiles".into()];
		let after = m.dialog_rect(w, h);
		assert!(after.h > before.h, "the report gets its own rows instead of cropping");
	}
}
