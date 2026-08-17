//! The Generate Random Terrain dialog: the worldgen form over
//! [`genform::GenMemory`], with the live run controls and the hover hint.

use super::*;

#[derive(Clone, Copy)]
pub(super) struct GenerateIds {
	pub(super) surprise: WidgetId,
	/// The three top selects + the accessibility-mode select inline on its row.
	pub(super) generator: WidgetId,
	pub(super) symmetry: WidgetId,
	pub(super) shore: WidgetId,
	pub(super) access: WidgetId,
	/// The wide seed field (the per-knob field ids live in `Overlay::gen_rows`,
	/// which varies with the generator).
	pub(super) seed: WidgetId,
	/// The hover hint line + inline validation error.
	pub(super) hint: WidgetId,
	pub(super) error: WidgetId,
	/// The report inset: progress stage + bar while running, else up to three
	/// status lines.
	pub(super) stage: WidgetId,
	pub(super) bar: WidgetId,
	pub(super) status: [WidgetId; 3],
	/// Close (disabled while running), Copy Seed (after a run), Generate/Abort.
	pub(super) close: WidgetId,
	pub(super) copy_seed: WidgetId,
	pub(super) generate: WidgetId,
}

/// Generate dialog state: the per-knob field ids for the current generator's
/// rows (`(knob, label, [count, min, max])`; absent columns are NONE), the
/// per-generator session memory (canonical values, updated by capture before
/// any rebuild), the current generator, the map size (Surprise scales body
/// sizes to it), the seed text (kept as typed across generator switches), and
/// the run/report display state.
///
/// One struct, replaced wholesale by [`Overlay::open_generate`] and reset by
/// [`Overlay::hide`].
pub(super) struct GenerateState {
	pub(super) rows: Vec<(crate::genform::Knob, &'static str, [WidgetId; 3])>,
	pub(super) mem: std::collections::HashMap<map_core::Generator, map_core::GenParams>,
	pub(super) current: map_core::Generator,
	pub(super) map: (usize, usize),
	pub(super) seed: String,
	pub(super) running: bool,
	pub(super) reported: Option<u64>,
}

impl Default for GenerateState {
	fn default() -> Self {
		Self {
			rows: Vec::new(),
			mem: std::collections::HashMap::new(),
			current: map_core::Generator::Islands,
			map: (64, 64),
			seed: String::new(),
			running: false,
			reported: None,
		}
	}
}

impl Overlay {
	/// Opens the Generate Random Terrain dialog — a non-blocking float over the
	/// live map (like Fix Shore): the per-generator knob form, hover hints, and
	/// a report inset that shows the run's progress bar while it steps and the
	/// seed/counts report after. The run lives on the editor; the shell re-syncs
	/// this window each frame via [`sync_generate`](Self::sync_generate). `mem`
	/// restores each generator's last-used settings; the map size scales
	/// Surprise Me body sizes.
	pub fn open_generate(&mut self, mem: &genform::GenMemory, map_w: usize, map_h: usize) {
		self.generate =
			GenerateState { mem: mem.params.clone(), current: mem.last, map: (map_w, map_h), ..Default::default() };
		self.build_generate();
		self.events.clear();
		self.visible = true;
	}

	/// (Re)builds the Generate tree for the current generator + run state.
	/// Called on open, on a generator switch, after Surprise Me, and when the
	/// run starts/ends — always from the canonical values in `gen_mem` (capture
	/// first if widgets hold newer text).
	fn build_generate(&mut self) {
		use map_core::{AccessibilityMode, GenParams, Generator as Gen, ShoreMethod, Symmetry};
		let p = self
			.generate
			.mem
			.get(&self.generate.current)
			.copied()
			.unwrap_or_else(|| GenParams::defaults(self.generate.current));
		let run = self.generate.running;
		const LABEL_W: f32 = 108.0;
		const COL_W: f32 = 46.0;
		const GAP: f32 = 6.0;
		/// The form's content width (the strut): label column + three numeric
		/// columns — also the wrap width for the hint/error status slots.
		const FORM_W: f32 = LABEL_W + 3.0 * COL_W + 2.0 * GAP;

		let surprise = Button::new("Surprise Me!").disabled(run);
		let pos = |i: Option<usize>| i.unwrap_or(0);
		let gsel = Select::new(Gen::ALL.iter().map(|g| g.label()))
			.small()
			.with_selected(pos(Gen::ALL.iter().position(|&g| g == self.generate.current)))
			.disabled(run);
		let ssel = Select::new(Symmetry::ALL.iter().map(|s| s.label()))
			.small()
			.with_selected(pos(Symmetry::ALL.iter().position(|&s| s == p.symmetry)))
			.disabled(run);
		let shsel = Select::new(ShoreMethod::ALL.iter().map(|s| s.label()))
			.small()
			.with_selected(pos(ShoreMethod::ALL.iter().position(|&s| s == p.shore)))
			.disabled(run);
		let seed = TextInput::with_text(&self.generate.seed)
			.charset(Charset::Digits)
			.max_len(20)
			.placeholder("random")
			.disabled(run);
		let hint = Label::new("").small().muted().with_id();
		let error = Label::new("").small().with_id();
		let close = Button::new("Close").secondary().disabled(run);
		let generate = if run { Button::new("Abort").secondary() } else { Button::new("Generate").primary() };

		let mut ids = GenerateIds {
			surprise: surprise.id(),
			generator: gsel.id(),
			symmetry: ssel.id(),
			shore: shsel.id(),
			access: WidgetId::NONE,
			seed: seed.id(),
			hint: hint.id(),
			error: error.id(),
			stage: WidgetId::NONE,
			bar: WidgetId::NONE,
			status: [WidgetId::NONE; 3],
			close: close.id(),
			copy_seed: WidgetId::NONE,
			generate: generate.id(),
		};

		let gen_row = |label: &str| {
			Linear::row()
				.spacing(GAP)
				.cross_align(CrossAlign::Center)
				.child(Label::new(label).small().muted(), Length::Fixed(LABEL_W))
		};
		let mut col = column().push(width_strut(FORM_W)).push(surprise);
		col = col
			.push(gen_row("generator").child(gsel, Length::Flex(1.0)))
			.push(gen_row("symmetry").child(ssel, Length::Flex(1.0)))
			.push(gen_row("shore").child(shsel, Length::Flex(1.0)));

		// Column headers over the numeric columns.
		let mut header = gen_row("");
		for c in ["count", "min", "max"] {
			header = header.child(Label::new(c).small().muted(), Length::Fixed(COL_W));
		}
		col = col.push(header);

		// The per-generator knob rows (accessibility hosts its mode select
		// across the min/max columns).
		self.generate.rows.clear();
		for (k, label, cols) in genform::rows(self.generate.current) {
			let (c, mn, mx) = genform::get(&p, k);
			let vals = [c, mn, mx];
			let mut row = gen_row(label);
			let mut ids3 = [WidgetId::NONE; 3];
			if k == genform::Knob::Accessibility {
				let f = TextInput::with_text(c.to_string()).charset(Charset::Digits).max_len(3).disabled(run);
				ids3[0] = f.id();
				row = row.child(f, Length::Fixed(COL_W));
				let asel = Select::new(AccessibilityMode::ALL.iter().map(|m| m.label()))
					.small()
					.with_selected(pos(AccessibilityMode::ALL.iter().position(|&m| m == p.accessibility_mode)))
					.disabled(run);
				ids.access = asel.id();
				row = row.child(asel, Length::Fixed(2.0 * COL_W + GAP));
			} else {
				for (ci, val) in vals.into_iter().enumerate() {
					if cols.contains(&ci) {
						let f = TextInput::with_text(val.to_string()).charset(Charset::Digits).max_len(3).disabled(run);
						ids3[ci] = f.id();
						row = row.child(f, Length::Fixed(COL_W));
					} else {
						row = row.child(Label::new("").small(), Length::Fixed(COL_W));
					}
				}
			}
			self.generate.rows.push((k, label, ids3));
			col = col.push(row);
		}
		col = col.push(gen_row("seed").child(seed, Length::Flex(1.0)));

		// Hover hint (a reserved three-line slot: the longest hints wrap to
		// three lines at this width and the window must never resize), then the
		// report inset: progress while running, else up to three status lines.
		col = status_slot(col, hint, FORM_W, 3);
		if run {
			let stage = Label::new("").small().with_id();
			let bar = ProgressBar::new(0.0).with_id();
			ids.stage = stage.id();
			ids.bar = bar.id();
			col = status_slot(col, stage, FORM_W, 2).push(bar);
		} else {
			for slot in &mut ids.status {
				let line = Label::new("").small().muted().with_id();
				*slot = line.id();
				col = col.push(line);
			}
		}
		col = status_slot(col, error, FORM_W, 2);

		let mut brow = Linear::row().spacing(8.0).main_align(MainAlign::End).push(close);
		if !run && self.generate.reported.is_some() {
			let cs = Button::new("Copy Seed");
			ids.copy_seed = cs.id();
			brow = brow.push(cs);
		}
		brow = brow.push(generate);
		col = col.push(brow);

		let win = self.dialog_kept("Generate Random Terrain", col, matches!(self.dialog, Dialog::Generate(_)));
		self.win_id = Some(win.id());
		// Non-blocking float: the window is the whole tree (no scrim), over the
		// live map — pan/zoom/paint keep working around a run.
		self.ui = Ui::new(win);
		self.dialog = Dialog::Generate(ids);
		self.blocking = false;
	}

	/// Snapshot the Generate widgets into the canonical per-generator memory
	/// (best effort — a non-numeric field keeps its previous value, like the
	/// legacy modal's close snapshot). The seed text is kept as typed.
	fn capture_generate(&mut self) {
		use map_core::{AccessibilityMode, GenParams, ShoreMethod, Symmetry};
		let Dialog::Generate(ids) = self.dialog else { return };
		let mut p = self
			.generate
			.mem
			.get(&self.generate.current)
			.copied()
			.unwrap_or_else(|| GenParams::defaults(self.generate.current));
		p.generator = self.generate.current;
		let rows = self.generate.rows.clone();
		for (k, _, ids3) in rows {
			let (c, mn, mx) = genform::get(&p, k);
			let mut vals = [c, mn, mx];
			for (ci, slot) in vals.iter_mut().enumerate() {
				if ids3[ci] != WidgetId::NONE {
					if let Ok(v) = self.text(ids3[ci]).trim().parse::<u8>() {
						*slot = v;
					}
				}
			}
			genform::set(&mut p, k, vals[0], vals[1], vals[2]);
		}
		if let Some(s) = self.ui.get::<Select>(ids.symmetry) {
			p.symmetry = Symmetry::ALL[s.selected().min(Symmetry::ALL.len() - 1)];
		}
		if let Some(s) = self.ui.get::<Select>(ids.shore) {
			p.shore = ShoreMethod::ALL[s.selected().min(ShoreMethod::ALL.len() - 1)];
		}
		if let Some(s) = self.ui.get::<Select>(ids.access) {
			p.accessibility_mode = AccessibilityMode::ALL[s.selected().min(AccessibilityMode::ALL.len() - 1)];
		}
		self.generate.seed = self.text(ids.seed);
		self.generate.mem.insert(self.generate.current, p);
	}

	/// Resolve the Generate press: strict-validate every visible field (inline
	/// alert on failure — the legacy modal's `params()` rules), then hand the
	/// settings to the shell's stepped run.
	pub(super) fn generate_confirm(&mut self, ids: GenerateIds) -> Outcome {
		let rows = self.generate.rows.clone();
		for (_, label, ids3) in &rows {
			for &id in ids3 {
				if id != WidgetId::NONE && self.text(id).trim().parse::<u8>().is_err() {
					self.set_label(ids.error, &format!("{label} is not a number"));
					return Outcome::Idle;
				}
			}
		}
		let seed_text = self.text(ids.seed);
		let seed = if seed_text.trim().is_empty() {
			None
		} else {
			match seed_text.trim().parse::<u64>() {
				Ok(s) => Some(s),
				Err(_) => {
					self.set_label(ids.error, "seed is not a number (u64)");
					return Outcome::Idle;
				}
			}
		};
		self.set_label(ids.error, "");
		self.capture_generate();
		let params = self
			.generate
			.mem
			.get(&self.generate.current)
			.copied()
			.unwrap_or_else(|| map_core::GenParams::defaults(self.generate.current));
		Outcome::GenerateStart { params, seed }
	}

	/// Close the Generate dialog, handing the per-generator session memory back
	/// to the shell (so reopening restores it).
	pub(super) fn generate_close_outcome(&mut self) -> Outcome {
		self.capture_generate();
		let mem = genform::GenMemory { last: self.generate.current, params: self.generate.mem.clone() };
		self.hide();
		Outcome::GenerateClose(mem)
	}

	/// Pushes the live run state into the Generate window (the shell calls this
	/// every frame while it's open): the progress stage + bar while running,
	/// the report lines when idle, and a rebuild when the run starts/ends
	/// (fields freeze while running; Copy Seed appears once a run reported).
	pub fn sync_generate(
		&mut self,
		running: bool,
		progress: Option<(&'static str, f32)>,
		status: &[String],
		reported: Option<u64>,
	) {
		if !matches!(self.dialog, Dialog::Generate(_)) {
			return;
		}
		if running != self.generate.running || reported != self.generate.reported {
			self.generate.running = running;
			self.generate.reported = reported;
			self.build_generate();
		}
		let Dialog::Generate(ids) = self.dialog else { return };
		if running {
			if let Some((stage, frac)) = progress {
				self.set_label(ids.stage, stage);
				if let Some(b) = self.ui.get_mut::<ProgressBar>(ids.bar) {
					b.set_fraction(frac);
				}
			}
		} else {
			for (i, id) in ids.status.into_iter().enumerate() {
				self.set_label(id, status.get(i).map(String::as_str).unwrap_or(""));
			}
		}
	}

	/// Sets the hover-hint line from whatever control the cursor is over (row
	/// bands span the window width, like the legacy dialog's hint box). Runs in
	/// [`render`](Self::render), right after the dispatch, so the `Ui`'s own
	/// pointer — the coordinate hover was just resolved against — is current;
	/// the row *bands* are host geometry (window-wide at each row's height),
	/// which is why this reads the pointer rather than only `Ui::hovered`.
	pub(super) fn sync_generate_hint(&mut self) {
		let Dialog::Generate(ids) = self.dialog else { return };
		let Some(win) = self.win_id.and_then(|id| self.ui.rect_of(id)) else { return };
		let cur = self.ui.pointer();
		let mut hint = "";
		if win.contains(cur) {
			let over = |id: WidgetId| id != WidgetId::NONE && self.ui.rect_of(id).is_some_and(|r| r.contains(cur));
			let named = [
				(ids.surprise, genform::SURPRISE_HINT),
				(ids.generator, genform::GENERATOR_HINT),
				(ids.symmetry, genform::SYMMETRY_HINT),
				(ids.shore, genform::SHORE_HINT),
				(ids.access, genform::ACCESS_HINT),
				(ids.seed, genform::SEED_HINT),
			];
			hint = named.into_iter().find(|&(id, _)| over(id)).map(|(_, h)| h).unwrap_or("");
			if hint.is_empty() {
				// A knob row's band spans the window width at its fields' height.
				for (k, _, ids3) in &self.generate.rows {
					let Some(fid) = ids3.iter().copied().find(|&i| i != WidgetId::NONE) else { continue };
					if self
						.ui
						.rect_of(fid)
						.is_some_and(|r| wgpu_ui::Rect::new(win.x, r.y - 2.0, win.w, r.h + 4.0).contains(cur))
					{
						hint = genform::knob_hint(*k);
						break;
					}
				}
			}
		}
		self.set_label(ids.hint, hint);
	}

	/// One frame of the dialog's control dispatch (the render match arm).
	pub(super) fn dispatch_generate(&mut self, ids: GenerateIds) -> Outcome {
		let mut outcome = Outcome::Idle;
		if self.ui.fired(ids.generator) {
			// Generator switched: stash the old form, load the new one.
			let sel = self.ui.get::<Select>(ids.generator).map_or(0, |s| s.selected());
			self.capture_generate();
			self.generate.current = map_core::Generator::ALL[sel.min(map_core::Generator::ALL.len() - 1)];
			self.build_generate();
		} else if self.ui.fired(ids.surprise) {
			// Surprise Me: roll the visible knobs (map-aware) + a fresh
			// reproducible seed, then repaint the form from the values.
			self.capture_generate();
			let mut p = self
				.generate
				.mem
				.get(&self.generate.current)
				.copied()
				.unwrap_or_else(|| map_core::GenParams::defaults(self.generate.current));
			let seed = genform::surprise(&mut p, self.generate.map.0, self.generate.map.1);
			self.generate.mem.insert(self.generate.current, p);
			self.generate.seed = seed.to_string();
			self.build_generate();
		} else if self.ui.fired(ids.generate) {
			if self.generate.running {
				outcome = Outcome::GenerateAbort;
			} else {
				outcome = self.generate_confirm(ids);
			}
		} else if ids.copy_seed != WidgetId::NONE && self.ui.fired(ids.copy_seed) {
			if let Some(seed) = self.generate.reported {
				crate::clipboard::set(&seed.to_string());
			}
		} else if self.ui.fired(ids.close) && !self.generate.running {
			outcome = self.generate_close_outcome();
		}
		outcome
	}
}
