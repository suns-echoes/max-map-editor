//! The Tile Painter dialog (DEV): paint a tile's pixels over the palette
//! swatch grid, set passability, and commit through the shell.

use super::*;

#[derive(Clone, Copy)]
pub(super) struct TilePaintIds {
	/// The two custom widgets (the paintable canvas + the 16×16 palette grid).
	pub(super) canvas: WidgetId,
	pub(super) swatches: WidgetId,
	/// The current-color chip + its "color N" caption.
	pub(super) chip: WidgetId,
	pub(super) color_label: WidgetId,
	/// Zoom select + the three toggles (eyedrop/replace are mutually exclusive).
	pub(super) zoom: WidgetId,
	pub(super) eyedrop: WidgetId,
	pub(super) replace: WidgetId,
	pub(super) animate: WidgetId,
	/// Canvas commands (Paste greys out until something was copied).
	pub(super) copy: WidgetId,
	pub(super) paste: WidgetId,
	pub(super) export: WidgetId,
	pub(super) import: WidgetId,
	/// The passability radio row (one group of four).
	pub(super) pass: [WidgetId; 4],
	/// The target-pack select (New mode only; NONE otherwise).
	pub(super) pack: WidgetId,
	pub(super) id_field: WidgetId,
	/// Inline commit-failure line (the dialog stays open on a bad id).
	pub(super) error: WidgetId,
	pub(super) cancel: WidgetId,
	pub(super) save: WidgetId,
}

/// Tile Painter dialog state: the dialog-side canvas (the truth while open;
/// mirrored to the editor's run after every edited frame), the family mask,
/// the armed tool + picked color, the animate toggle, and the open mode.
///
/// One struct, replaced wholesale by [`Overlay::open_tile_paint`] and reset
/// to default by [`Overlay::hide`].
pub(super) struct TilePaintState {
	pub(super) canvas: Vec<u8>,
	pub(super) mask: Option<u8>,
	pub(super) state: crate::tilepaint::PaintState,
	pub(super) animate: bool,
	/// The last live palette table (256x4 sRGB) - textures recompose when it
	/// moves (palette cycling) or the canvas changes.
	pub(super) rgba: Vec<u8>,
	pub(super) dirty: bool,
	/// Canvas edited since the shell last took it (mirror re-sync flag).
	pub(super) edited: bool,
	/// The last seen [`crate::tilepaint::TilePaintRun::canvas_rev`] (an editor
	/// write - PNG import - bumps it; the dialog then reloads its copy).
	pub(super) rev: u64,
	/// The dialog's copy buffer (seeded from the shell's tile clipboard on
	/// open, so Paste works across painter sessions).
	pub(super) clip: Option<Vec<u8>>,
	/// New-mode target packs / the fixed source pack (Edit/Clone), and the mode.
	pub(super) packs: Vec<String>,
	pub(super) pack_name: String,
	pub(super) mode: crate::tilepaint::Mode,
}

impl Default for TilePaintState {
	fn default() -> Self {
		Self {
			canvas: Vec::new(),
			mask: None,
			state: crate::tilepaint::PaintState::default(),
			animate: false,
			rgba: Vec::new(),
			dirty: false,
			edited: false,
			rev: 0,
			clip: None,
			packs: Vec::new(),
			pack_name: String::new(),
			mode: crate::tilepaint::Mode::New,
		}
	}
}

impl Overlay {
	/// Opens the Rename Template dialog: a frozen thumbnail, its footprint, an
	/// editable name field + inline alert, and Cancel / Save. Save emits
	/// `template-rename "from" "to"` once the name validates.
	/// Opens the Tile Painter over `run` (the editor-owned context). The dialog
	/// takes a working copy of the canvas; `rgba` is the live palette table,
	/// `animate` seeds the animate-colors toggle (from the editor's map
	/// setting), and `clipboard` seeds Paste (the shell's tile clipboard).
	pub fn open_tile_paint(
		&mut self,
		chrome: &mut MenuChrome,
		run: &crate::tilepaint::TilePaintRun,
		rgba: &[u8],
		animate: bool,
		clipboard: Option<&[u8]>,
	) {
		use crate::tilepaint::{TILE, compose_canvas_rgba, compose_swatches_rgba};
		self.tp = TilePaintState {
			canvas: run.canvas.clone(),
			mask: run.mask,
			state: crate::tilepaint::PaintState { color: 1, ..Default::default() },
			animate,
			rgba: rgba.to_vec(),
			rev: run.canvas_rev,
			clip: clipboard.map(<[u8]>::to_vec),
			packs: run.packs.clone(),
			pack_name: run.pack_name.clone(),
			mode: run.mode,
			..Default::default()
		};
		// Compose both textures into their reusable fixed-size slots.
		let canvas_rgba = compose_canvas_rgba(&self.tp.canvas, &self.tp.rgba, self.tp.mask);
		let (tw, th) = (TILE as u32, TILE as u32);
		match self.tp_canvas_tex {
			Some(id) => chrome.replace_texture(id, &canvas_rgba, tw, th),
			None => self.tp_canvas_tex = Some(chrome.register_texture(&canvas_rgba, tw, th)),
		}
		let swatch_rgba = compose_swatches_rgba(&self.tp.rgba);
		match self.tp_swatch_tex {
			Some(id) => chrome.replace_texture(id, &swatch_rgba, 16, 16),
			None => self.tp_swatch_tex = Some(chrome.register_texture(&swatch_rgba, 16, 16)),
		}
		self.build_tile_paint(run.pass, &run.id_text);
		self.events.clear();
		self.visible = true;
	}

	/// Builds the Tile Painter tree: the canvas column (viewport + zoom/tools/
	/// commands) beside the palette column (swatches, current color, pass,
	/// pack, id), over a Cancel/Save row. Built once per open - runtime state
	/// changes go through setters, never a rebuild (the canvas must not reset).
	fn build_tile_paint(&mut self, pass: u8, id_text: &str) {
		use crate::tilepaint::{Chip, Mode, PASSES, PixelCanvas, SwatchGrid, ZOOMS};
		let canvas_tex = self.tp_canvas_tex.expect("registered in open_tile_paint");
		let swatch_tex = self.tp_swatch_tex.expect("registered in open_tile_paint");

		let canvas = PixelCanvas::new(canvas_tex);
		let swatches = SwatchGrid::new(swatch_tex, self.tp.state.color);
		let chip = Chip::new(crate::tilepaint::slot_color(&self.tp.rgba, self.tp.state.color), 24.0);
		let color_label = Label::new(format!("color {}", self.tp.state.color)).small().with_id();
		let zoom = Select::new(ZOOMS.iter().map(|z| z.1)).small().with_selected(ZOOMS.len() - 1);
		let eyedrop = Checkbox::new("Eyedropper");
		let replace = Checkbox::new("Replace color");
		let animate = Checkbox::new("Animate colors").with_checked(self.tp.animate);
		let copy = Button::new("Copy");
		let paste = Button::new("Paste").disabled(self.tp.clip.is_none());
		let export = Button::new("Export PNG");
		let import = Button::new("Import PNG");
		let id_field = TextInput::with_text(id_text).charset(Charset::Identifier).max_len(24).placeholder("(auto)");
		let error = Label::new("").small().with_id();
		let cancel = Button::new("Cancel").secondary();
		let save = Button::new("Save").primary();

		let mut ids = TilePaintIds {
			canvas: canvas.id(),
			swatches: swatches.id(),
			chip: chip.id(),
			color_label: color_label.id(),
			zoom: zoom.id(),
			eyedrop: eyedrop.id(),
			replace: replace.id(),
			animate: animate.id(),
			copy: copy.id(),
			paste: paste.id(),
			export: export.id(),
			import: import.id(),
			pass: [WidgetId::NONE; 4],
			pack: WidgetId::NONE,
			id_field: id_field.id(),
			error: error.id(),
			cancel: cancel.id(),
			save: save.id(),
		};

		let row = || Linear::row().spacing(8.0).cross_align(CrossAlign::Center);
		let left = column()
			.push(canvas)
			.push(row().child(zoom, Length::Fixed(84.0)).push(eyedrop).push(replace))
			.push(row().push(copy).push(paste).push(animate))
			.push(row().push(export).push(import));

		let mut pass_row = Linear::row().spacing(6.0);
		for (i, name) in PASSES.iter().enumerate() {
			let rb = Radio::new(*name).with_selected(pass.min(3) as usize == i);
			ids.pass[i] = rb.id();
			pass_row = pass_row.push(rb);
		}
		// The grid must not stretch past its 16×18px cells (the selection rings
		// are laid out on that grid), so it rides a start-aligned row.
		let grid_w = 16.0 * crate::tilepaint::SW;
		let mut right = column()
			.push(Linear::row().child(swatches, Length::Fixed(grid_w)))
			.push(row().push(chip).push(color_label))
			.push(Label::new("Passability").small().muted())
			.push(pass_row)
			.push(Label::new("Pack").small().muted());
		if self.tp.mode == Mode::New {
			let pack = Select::new(self.tp.packs.iter().map(String::as_str)).small();
			ids.pack = pack.id();
			right = right.push(row().child(pack, Length::Flex(1.0)));
		} else {
			right = right.push(Label::new(self.tp.pack_name.clone()).small());
		}
		right = right.push(Label::new("Tile id").small().muted()).push(row().child(id_field, Length::Flex(1.0)));
		right = status_slot(right, error, grid_w, 2);

		let content = column().push(Linear::row().spacing(16.0).push(left).push(right)).push(buttons(cancel, save));
		let win = dialog(self.tp.mode.title(), content);
		self.win_id = Some(win.id());
		self.ui = Ui::new(modal(win));
		self.dialog = Dialog::TilePaint(ids);
		self.blocking = true;
	}

	/// Pushes the live palette table (and an editor-side canvas write, keyed by
	/// its revision) into the open Tile Painter. The shell calls this every
	/// frame before `render`; the textures recompose on change (or every frame
	/// while the palette cycles).
	pub fn sync_tile_paint(&mut self, rgba: &[u8], canvas: Option<(&[u8], u64)>) {
		if !matches!(self.dialog, Dialog::TilePaint(_)) {
			return;
		}
		if self.tp.rgba != rgba {
			self.tp.rgba = rgba.to_vec();
			self.tp.dirty = true;
		}
		if let Some((pixels, rev)) = canvas {
			if rev != self.tp.rev && pixels.len() == self.tp.canvas.len() {
				self.tp.rev = rev;
				self.tp.canvas.copy_from_slice(pixels);
				self.tp.dirty = true;
			}
		}
	}

	/// True while the open Tile Painter wants live palette cycling - the shell
	/// keeps ticking the cycler + redrawing so the preview shimmers.
	pub fn tile_paint_animating(&self) -> bool {
		self.visible && matches!(self.dialog, Dialog::TilePaint(_)) && self.tp.animate
	}

	/// The painter's working canvas, when it was edited since the last take -
	/// the shell mirrors it into [`crate::state::EditorState::tilepaint`] so
	/// command paths (commit/export) read current pixels.
	pub fn tile_canvas_if_edited(&mut self) -> Option<&[u8]> {
		(matches!(self.dialog, Dialog::TilePaint(_)) && std::mem::take(&mut self.tp.edited))
			.then_some(&self.tp.canvas[..])
	}

	/// Shows a commit failure inline (the dialog stays open, edits kept).
	pub fn tile_paint_error(&mut self, message: &str) {
		if let Dialog::TilePaint(ids) = self.dialog {
			self.set_label(ids.error, message);
		}
	}

	/// One frame of the dialog's control dispatch (the render match arm).
	pub(super) fn dispatch_tile_paint(&mut self, ids: TilePaintIds, chrome: &mut MenuChrome) -> Outcome {
		let mut outcome = Outcome::Idle;
		use crate::tilepaint::{self, PixelCanvas, SwatchGrid};
		// Eyedropper / replace are mutually exclusive tool modes.
		if self.ui.fired(ids.eyedrop) {
			self.tp.state.eyedrop = self.ui.get::<Checkbox>(ids.eyedrop).is_some_and(Checkbox::checked);
			if self.tp.state.eyedrop {
				self.tp.state.replace = false;
				if let Some(c) = self.ui.get_mut::<Checkbox>(ids.replace) {
					c.set_checked(false);
				}
			}
		}
		if self.ui.fired(ids.replace) {
			self.tp.state.replace = self.ui.get::<Checkbox>(ids.replace).is_some_and(Checkbox::checked);
			if self.tp.state.replace {
				self.tp.state.eyedrop = false;
				if let Some(c) = self.ui.get_mut::<Checkbox>(ids.eyedrop) {
					c.set_checked(false);
				}
			}
		}
		if self.ui.fired(ids.animate) {
			self.tp.animate = self.ui.get::<Checkbox>(ids.animate).is_some_and(Checkbox::checked);
		}
		// The passability radios are one group: clear the others.
		self.radio_group(&ids.pass);
		// A swatch pick sets the paint color.
		if self.ui.fired(ids.swatches) {
			if let Some(g) = self.ui.get::<SwatchGrid>(ids.swatches) {
				self.tp.state.color = g.sel();
			}
		}
		// Zoom: the select, or wheel notches over the canvas.
		if self.ui.fired(ids.zoom) {
			let i = self.ui.get::<Select>(ids.zoom).map(Select::selected).unwrap_or(0);
			let px = tilepaint::ZOOMS[i.min(tilepaint::ZOOMS.len() - 1)].0;
			if let Some(c) = self.ui.get_mut::<PixelCanvas>(ids.canvas) {
				c.set_zoom(px);
			}
		}
		// Canvas events: apply the armed tool to the working canvas.
		let (evs, wheel) = match self.ui.get_mut::<PixelCanvas>(ids.canvas) {
			Some(c) => (c.take_events(), c.take_wheel()),
			None => (Vec::new(), 0.0),
		};
		for ev in evs {
			if tilepaint::apply_canvas_event(&mut self.tp.canvas, &mut self.tp.state, ev) {
				self.tp.dirty = true;
				self.tp.edited = true;
			}
		}
		if wheel != 0.0 {
			let cur = self.ui.get::<Select>(ids.zoom).map(Select::selected).unwrap_or(0);
			let next = (cur as i32 + wheel.signum() as i32).clamp(0, tilepaint::ZOOMS.len() as i32 - 1);
			if next != cur as i32 {
				if let Some(s) = self.ui.get_mut::<Select>(ids.zoom) {
					s.set_selected(next as usize);
				}
				if let Some(c) = self.ui.get_mut::<PixelCanvas>(ids.canvas) {
					c.set_zoom(tilepaint::ZOOMS[next as usize].0);
				}
			}
		}
		// Commands.
		if self.ui.fired(ids.cancel) {
			self.hide();
			outcome = Outcome::TilePaintClose;
		} else if self.ui.fired(ids.save) {
			// The dialog stays open: the shell hides it on success or
			// pushes the failure back in (edits survive a bad id).
			let id = self.text(ids.id_field).trim().to_string();
			let pass =
				ids.pass.iter().position(|id| self.ui.get::<Radio>(*id).is_some_and(Radio::selected)).unwrap_or(0)
					as u8;
			let pack = if ids.pack != WidgetId::NONE {
				let i = self.ui.get::<Select>(ids.pack).map(Select::selected).unwrap_or(0);
				self.tp.packs.get(i).cloned().unwrap_or_default()
			} else {
				self.tp.pack_name.clone()
			};
			outcome = Outcome::TileCommit { id, pass, pack, pixels: self.tp.canvas.clone() };
		} else if self.ui.fired(ids.copy) {
			self.tp.clip = Some(self.tp.canvas.clone());
			if let Some(b) = self.ui.get_mut::<Button>(ids.paste) {
				b.set_disabled(false);
			}
			outcome = Outcome::TileCopy(self.tp.canvas.clone());
		} else if self.ui.fired(ids.paste) {
			if let Some(clip) = self.tp.clip.clone() {
				self.tp.canvas.copy_from_slice(&clip);
				self.tp.dirty = true;
				self.tp.edited = true;
			}
		} else if self.ui.fired(ids.export) {
			outcome = Outcome::TileExportPng { id: self.text(ids.id_field).trim().to_string() };
		} else if self.ui.fired(ids.import) {
			outcome = Outcome::TileImportPng;
		}
		// Per-frame view sync: the selected swatch (the eyedropper may
		// have moved it), the hovered pixel's palette-slot ring, the
		// one-shot eyedropper checkbox, and the current-color chip.
		if matches!(self.dialog, Dialog::TilePaint(_)) {
			let hover = self.ui.get::<PixelCanvas>(ids.canvas).and_then(PixelCanvas::hover);
			let hint = hover.map(|(px, py)| self.tp.canvas[py as usize * tilepaint::TILE + px as usize]);
			if let Some(g) = self.ui.get_mut::<SwatchGrid>(ids.swatches) {
				g.set_sel(self.tp.state.color);
				g.set_hint(hint);
			}
			if let Some(c) = self.ui.get_mut::<Checkbox>(ids.eyedrop) {
				c.set_checked(self.tp.state.eyedrop);
			}
			if let Some(ch) = self.ui.get_mut::<tilepaint::Chip>(ids.chip) {
				ch.set_color(tilepaint::slot_color(&self.tp.rgba, self.tp.state.color));
			}
			let color_line = format!("color {}", self.tp.state.color);
			self.set_label(ids.color_label, &color_line);
			// Re-upload the composed art on change - and every frame
			// while the palette cycles (the shell re-syncs the table).
			if self.tp.dirty || self.tp.animate {
				if let Some(tex) = self.tp_canvas_tex {
					let rgba = tilepaint::compose_canvas_rgba(&self.tp.canvas, &self.tp.rgba, self.tp.mask);
					chrome.update_texture(tex, &rgba);
				}
				if let Some(tex) = self.tp_swatch_tex {
					chrome.update_texture(tex, &tilepaint::compose_swatches_rgba(&self.tp.rgba));
				}
				self.tp.dirty = false;
			}
		}
		outcome
	}
}
