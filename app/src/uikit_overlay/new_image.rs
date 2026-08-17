//! The New from Image dialog: pick a PNG, set the quantize thresholds and
//! rasterize a new map from it (the running/idle two-state form).

use super::*;

#[derive(Clone, Copy)]
pub(super) struct NewImageIds {
	// Settings stage.
	pub(super) width: WidgetId,
	pub(super) height: WidgetId,
	pub(super) coverage: WidgetId,
	pub(super) off_x: WidgetId,
	pub(super) off_y: WidgetId,
	pub(super) strict: WidgetId,
	pub(super) relaxed: WidgetId,
	pub(super) threshold: WidgetId,
	pub(super) error: WidgetId,
	// Running stage.
	pub(super) stage: WidgetId,
	pub(super) bar: WidgetId,
	pub(super) time: WidgetId,
	// Cancel + Convert/Abort.
	pub(super) cancel: WidgetId,
	pub(super) convert: WidgetId,
}

/// New-from-Image field values (canonical copies preserved across a rebuild,
/// which happens when the dedupe choice flips the threshold row), and whether
/// the dialog shows its running stage.
///
/// One struct, replaced wholesale by [`Overlay::open_new_image`]: the open is
/// the reset (`hide` leaves it alone).
#[derive(Default)]
pub(super) struct NewImageState {
	pub(super) relaxed: bool,
	pub(super) coverage: usize,
	pub(super) width: String,
	pub(super) height: String,
	pub(super) off_x: String,
	pub(super) off_y: String,
	pub(super) threshold: String,
	pub(super) running: bool,
}

impl Overlay {
	/// Opens the New from Image dialog at its settings stage (size, coverage,
	/// offsets, dedupe + threshold). The conversion lives on the editor; while
	/// it runs the dialog shows a stage/progress/time + Abort view, re-synced
	/// each frame. `width`/`height` seed the fit-to-source defaults.
	pub fn open_new_image(&mut self, width: u32, height: u32) {
		self.ni = NewImageState {
			width: width.to_string(),
			height: height.to_string(),
			off_x: "0".to_string(),
			off_y: "0".to_string(),
			threshold: "5".to_string(),
			..Default::default()
		};
		self.build_new_image();
		self.events.clear();
		self.visible = true;
	}

	/// (Re)builds the New from Image tree for the current field values + stage.
	pub(super) fn build_new_image(&mut self) {
		let mut ids = NewImageIds {
			width: WidgetId::NONE,
			height: WidgetId::NONE,
			coverage: WidgetId::NONE,
			off_x: WidgetId::NONE,
			off_y: WidgetId::NONE,
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
		if self.ni.running {
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
			let digits = |s: &str| TextInput::with_text(s).charset(Charset::Digits).max_len(5);
			let signed = |s: &str| TextInput::with_text(s).charset(Charset::SignedInt).max_len(5);
			let width = digits(&self.ni.width);
			let height = digits(&self.ni.height);
			let coverage = Select::new(["Crop", "Stretch", "Fill"]).small();
			let off_x = signed(&self.ni.off_x);
			let off_y = signed(&self.ni.off_y);
			let strict = Radio::new("strict").with_selected(!self.ni.relaxed);
			let relaxed = Radio::new("relaxed").with_selected(self.ni.relaxed);
			let error = Label::new("").small().with_id();
			let convert = Button::new("Convert").primary();
			ids.width = width.id();
			ids.height = height.id();
			ids.coverage = coverage.id();
			ids.off_x = off_x.id();
			ids.off_y = off_y.id();
			ids.strict = strict.id();
			ids.relaxed = relaxed.id();
			ids.error = error.id();
			ids.convert = convert.id();
			let size_row = Linear::row()
				.spacing(6.0)
				.cross_align(CrossAlign::Center)
				.child(width, Length::Flex(1.0))
				.push(Label::new("x").small().muted())
				.child(height, Length::Flex(1.0));
			let off_row = Linear::row()
				.spacing(6.0)
				.cross_align(CrossAlign::Center)
				.child(off_x, Length::Flex(1.0))
				.child(off_y, Length::Flex(1.0));
			let mut dedupe_row = Linear::row().spacing(6.0).cross_align(CrossAlign::Center).push(strict).push(relaxed);
			if self.ni.relaxed {
				let threshold = TextInput::with_text(&self.ni.threshold).charset(Charset::Digits).max_len(5);
				ids.threshold = threshold.id();
				dedupe_row = dedupe_row.child(threshold, Length::Fixed(48.0)).push(Label::new("%").small().muted());
			}
			col = col
				.push(field_row("size (tiles)", size_row))
				.push(field_row("coverage", coverage))
				.push(field_row("offset x,y", off_row))
				.push(field_row("dedupe", dedupe_row));
			col = status_slot(col, error, 360.0, 2).push(buttons(cancel, convert));
		}
		let win = self.dialog_kept("New from Image", col, matches!(self.dialog, Dialog::NewImage(_)));
		self.win_id = Some(win.id());
		self.ui = Ui::new(modal(win));
		self.ui.focus_first();
		// Restore the coverage selection after the tree is built.
		if !self.ni.running {
			if let Dialog::NewImage(_) = Dialog::NewImage(ids) {
				if let Some(sel) = self.ui.get_mut::<Select>(ids.coverage) {
					sel.set_selected(self.ni.coverage);
				}
			}
		}
		self.dialog = Dialog::NewImage(ids);
	}

	/// Pushes the live conversion numbers into the running stage; flips the
	/// options ↔ running stages when the run starts or ends.
	pub fn sync_new_image(&mut self, running: bool, progress: f32, stage: &str, time: &str) {
		if !matches!(self.dialog, Dialog::NewImage(_)) {
			return;
		}
		if running != self.ni.running {
			self.ni.running = running;
			self.build_new_image();
		}
		let Dialog::NewImage(ids) = self.dialog else { return };
		if !running {
			return;
		}
		self.set_label(ids.stage, stage);
		self.set_label(ids.time, time);
		if let Some(b) = self.ui.get_mut::<ProgressBar>(ids.bar) {
			b.set_fraction(progress);
		}
	}

	/// Resolve the Convert press at the settings stage: validate the fields into
	/// `ConvertOpts` (inline alert on failure) and hand them to the shell's
	/// stepped run. Mirrors the legacy modal's `opts()` rules.
	pub(super) fn new_image_confirm(&mut self, ids: NewImageIds) -> Outcome {
		let width: u32 = match self.ni.width.trim().parse() {
			Ok(v) => v,
			Err(_) => return self.ni_err(ids, "width is not a number"),
		};
		let height: u32 = match self.ni.height.trim().parse() {
			Ok(v) => v,
			Err(_) => return self.ni_err(ids, "height is not a number"),
		};
		if !(1..=1024).contains(&width) || !(1..=1024).contains(&height) {
			return self.ni_err(ids, &format!("map size {width}x{height} (1..=1024 tiles)"));
		}
		let off_x: i32 = match self
			.ni
			.off_x
			.trim()
			.parse()
			.or_else(|_| if self.ni.off_x.trim().is_empty() { Ok(0) } else { Err(()) })
		{
			Ok(v) => v,
			Err(_) => return self.ni_err(ids, "offset x is not a number"),
		};
		let off_y: i32 = match self
			.ni
			.off_y
			.trim()
			.parse()
			.or_else(|_| if self.ni.off_y.trim().is_empty() { Ok(0) } else { Err(()) })
		{
			Ok(v) => v,
			Err(_) => return self.ni_err(ids, "offset y is not a number"),
		};
		let coverage = match self.ni.coverage {
			1 => map_core::Coverage::Stretch,
			2 => map_core::Coverage::Fill,
			_ => map_core::Coverage::Crop,
		};
		let (dedupe, threshold) = if self.ni.relaxed {
			match self.ni.threshold.trim().parse::<f32>() {
				Ok(pct) => (map_core::Dedupe::Relaxed, (pct / 100.0).clamp(0.0, 1.0)),
				Err(_) => return self.ni_err(ids, "threshold is not a number"),
			}
		} else {
			(map_core::Dedupe::Strict, 0.0)
		};
		Outcome::NewImageStart(map_core::ConvertOpts {
			width_tiles: width,
			height_tiles: height,
			coverage,
			off_x,
			off_y,
			dedupe,
			threshold,
		})
	}

	fn ni_err(&mut self, ids: NewImageIds, msg: &str) -> Outcome {
		self.set_label(ids.error, msg);
		Outcome::Idle
	}

	/// Snapshot the New-from-Image field widgets into the canonical copies
	/// before a rebuild (so a rebuild doesn't lose in-progress typing).
	pub(super) fn capture_new_image(&mut self, ids: NewImageIds) {
		if ids.width != WidgetId::NONE {
			self.ni.width = self.text(ids.width);
			self.ni.height = self.text(ids.height);
			self.ni.off_x = self.text(ids.off_x);
			self.ni.off_y = self.text(ids.off_y);
			if let Some(sel) = self.ui.get::<Select>(ids.coverage) {
				self.ni.coverage = sel.selected();
			}
		}
		if ids.threshold != WidgetId::NONE {
			self.ni.threshold = self.text(ids.threshold);
		}
	}

	/// One frame of the dialog's control dispatch (the render match arm).
	pub(super) fn dispatch_new_image(&mut self, ids: NewImageIds) -> Outcome {
		let mut outcome = Outcome::Idle;
		if self.ui.fired(ids.cancel) {
			outcome = Outcome::NewImageCancel;
			self.hide();
		} else if self.ui.fired(ids.convert) {
			if self.ni.running {
				outcome = Outcome::NewImageAbort;
			} else {
				self.capture_new_image(ids);
				outcome = self.new_image_confirm(ids);
			}
		} else if (self.ui.fired(ids.strict) || self.ui.fired(ids.relaxed)) && ids.strict != WidgetId::NONE {
			self.capture_new_image(ids);
			self.ni.relaxed = self.ui.fired(ids.relaxed);
			self.build_new_image();
		}
		outcome
	}
}
