//! The Convert to Compatible Palette dialog: best-match or rasterize an
//! incompatible project onto a chosen palette (the running/idle form).

use super::*;

#[derive(Clone, Copy)]
pub(super) struct ConvertPaletteIds {
	/// Options stage (zeroed while running).
	pub(super) mmatch: WidgetId,
	pub(super) mrast: WidgetId,
	pub(super) note: WidgetId,
	pub(super) water: WidgetId,
	pub(super) strict: WidgetId,
	pub(super) relaxed: WidgetId,
	pub(super) threshold: WidgetId,
	pub(super) error: WidgetId,
	/// Running stage (zeroed while idle).
	pub(super) stage: WidgetId,
	pub(super) bar: WidgetId,
	pub(super) time: WidgetId,
	/// Cancel (always) + Convert/Abort.
	pub(super) cancel: WidgetId,
	pub(super) convert: WidgetId,
}

/// Convert Palette option values (the widgets are rebuilt when the method or
/// dedupe choice flips, so these are the canonical copies), and whether the
/// dialog shows its running stage.
///
/// One struct, replaced wholesale by [`Overlay::open_convert_palette`]: the
/// open is the reset (`hide` leaves it alone).
pub(super) struct ConvertPaletteState {
	pub(super) rasterize: bool,
	pub(super) water: bool,
	pub(super) relaxed: bool,
	pub(super) threshold: String,
	pub(super) running: bool,
}

impl Default for ConvertPaletteState {
	fn default() -> Self {
		Self { rasterize: false, water: true, relaxed: false, threshold: String::new(), running: false }
	}
}

impl Overlay {
	/// Opens the Convert to Compatible Palette dialog at its options stage
	/// (method, water, dedupe + threshold). The rasterize run itself lives on
	/// the editor; while it runs the dialog shows its running stage
	/// (stage/progress/time + Abort), re-synced by the shell each frame.
	pub fn open_convert_palette(&mut self) {
		self.cp = ConvertPaletteState { threshold: "5".to_string(), ..Default::default() };
		self.build_convert_palette();
		self.events.clear();
		self.visible = true;
	}

	/// (Re)builds the Convert Palette tree for the current option values +
	/// stage. Called on open and whenever a choice flips the visible rows
	/// (method/dedupe) or the run starts/stops.
	pub(super) fn build_convert_palette(&mut self) {
		let mut ids = ConvertPaletteIds {
			mmatch: WidgetId::NONE,
			mrast: WidgetId::NONE,
			note: WidgetId::NONE,
			water: WidgetId::NONE,
			strict: WidgetId::NONE,
			relaxed: WidgetId::NONE,
			threshold: WidgetId::NONE,
			error: WidgetId::NONE,
			stage: WidgetId::NONE,
			bar: WidgetId::NONE,
			time: WidgetId::NONE,
			cancel: WidgetId::NONE,
			convert: WidgetId::NONE,
		};
		let cancel = Button::new("Cancel").secondary();
		ids.cancel = cancel.id();
		let mut col = column().push(width_strut(360.0));
		if self.cp.running {
			// Running stage: stage line, progress bar, %/elapsed/ETA line, Abort.
			let stage = Label::new("").small().with_id();
			let bar = ProgressBar::new(0.0).with_id();
			let time = Label::new("").small().muted().with_id();
			let abort = Button::new("Abort").secondary();
			ids.stage = stage.id();
			ids.bar = bar.id();
			ids.time = time.id();
			ids.convert = abort.id();
			col = status_slot(col, stage, 360.0, 2).push(bar).push(time).push(buttons(cancel, abort));
		} else {
			// Options stage.
			let mmatch = Radio::new("best match").with_selected(!self.cp.rasterize);
			let mrast = Radio::new("rasterize").with_selected(self.cp.rasterize);
			let note_text = if self.cp.rasterize {
				"render with the internal palette, re-import like New from Image"
			} else {
				"remap each used color to its nearest game-legal slot"
			};
			let note = Label::new(note_text).small().muted().with_id();
			let water = Checkbox::new("keep animated water colors").with_checked(self.cp.water);
			let error = Label::new("").small().with_id();
			let convert = Button::new("Convert").primary();
			ids.mmatch = mmatch.id();
			ids.mrast = mrast.id();
			ids.note = note.id();
			ids.water = water.id();
			ids.error = error.id();
			ids.convert = convert.id();
			col = col.push(field_row("method", Linear::row().spacing(6.0).push(mmatch).push(mrast)));
			col = status_slot(col, note, 360.0, 2).push(field_row("water", water));
			if self.cp.rasterize {
				let strict = Radio::new("strict").with_selected(!self.cp.relaxed);
				let relaxed = Radio::new("relaxed").with_selected(self.cp.relaxed);
				ids.strict = strict.id();
				ids.relaxed = relaxed.id();
				let mut row = Linear::row().spacing(6.0).cross_align(CrossAlign::Center).push(strict).push(relaxed);
				if self.cp.relaxed {
					let threshold = TextInput::with_text(&self.cp.threshold).charset(Charset::Decimal).max_len(5);
					ids.threshold = threshold.id();
					row = row.child(threshold, Length::Fixed(52.0)).push(Label::new("%").small().muted());
				}
				col = col.push(field_row("dedupe", row));
			}
			col = status_slot(col, error, 360.0, 2).push(buttons(cancel, convert));
		}
		let win =
			self.dialog_kept("Convert to Compatible Palette", col, matches!(self.dialog, Dialog::ConvertPalette(_)));
		self.win_id = Some(win.id());
		self.ui = Ui::new(modal(win));
		self.ui.focus_first();
		self.dialog = Dialog::ConvertPalette(ids);
	}

	/// Resolve the Convert press at the options stage: best match runs the
	/// command line straight away; rasterize validates the threshold and hands
	/// the options to the shell's stepped run (the dialog flips to its running
	/// stage on the next sync). Mirrors the legacy modal's `command()` rules.
	pub(super) fn convert_palette_confirm(&mut self, ids: ConvertPaletteIds) -> Outcome {
		let water = if self.cp.water { "water=keep" } else { "water=drop" };
		if !self.cp.rasterize {
			self.hide();
			return Outcome::RunCommand(format!("convert-palette match {water}"));
		}
		let threshold = if self.cp.relaxed {
			match self.cp.threshold.trim().parse::<f32>() {
				Ok(pct) if (0.0..=100.0).contains(&pct) => pct / 100.0,
				Ok(pct) => {
					self.set_label(ids.error, &format!("threshold {pct}% (0..=100)"));
					return Outcome::Idle;
				}
				Err(_) => {
					self.set_label(ids.error, "threshold is not a number");
					return Outcome::Idle;
				}
			}
		} else {
			0.0
		};
		Outcome::PaletteConvertStart { water: self.cp.water, relaxed: self.cp.relaxed, threshold }
	}

	/// Pushes the live rasterize numbers into the running stage (the shell
	/// calls this every frame while the dialog is open); flips between the
	/// options ↔ running stages when the run starts or ends.
	pub fn sync_convert_palette(&mut self, running: bool, progress: f32, stage: &str, time: &str) {
		if !matches!(self.dialog, Dialog::ConvertPalette(_)) {
			return;
		}
		if running != self.cp.running {
			// Run started or ended (finished/aborted): flip stages.
			self.cp.running = running;
			self.build_convert_palette();
		}
		let Dialog::ConvertPalette(ids) = self.dialog else { return };
		if !running {
			return;
		}
		self.set_label(ids.stage, stage);
		self.set_label(ids.time, time);
		if let Some(b) = self.ui.get_mut::<ProgressBar>(ids.bar) {
			b.set_fraction(progress);
		}
	}

	/// One frame of the dialog's control dispatch (the render match arm).
	pub(super) fn dispatch_convert_palette(&mut self, ids: ConvertPaletteIds) -> Outcome {
		let mut outcome = Outcome::Idle;
		if self.ui.fired(ids.cancel) {
			outcome = Outcome::PaletteConvertCancel;
			self.hide();
		} else if self.ui.fired(ids.convert) {
			if self.cp.running {
				outcome = Outcome::PaletteConvertAbort;
			} else {
				// Read the current option widgets (the canonical copies
				// live on self; the threshold text is read on demand).
				if ids.threshold != WidgetId::NONE {
					self.cp.threshold = self.text(ids.threshold);
				}
				outcome = self.convert_palette_confirm(ids);
			}
		} else if self.ui.fired(ids.mmatch) || self.ui.fired(ids.mrast) {
			if ids.threshold != WidgetId::NONE {
				self.cp.threshold = self.text(ids.threshold);
			}
			self.cp.rasterize = self.ui.fired(ids.mrast);
			self.build_convert_palette();
		} else if (self.ui.fired(ids.strict) || self.ui.fired(ids.relaxed)) && ids.strict != WidgetId::NONE {
			if ids.threshold != WidgetId::NONE {
				self.cp.threshold = self.text(ids.threshold);
			}
			self.cp.relaxed = self.ui.fired(ids.relaxed);
			self.build_convert_palette();
		} else if self.ui.fired(ids.water) {
			self.cp.water = !self.cp.water;
		}
		outcome
	}
}
