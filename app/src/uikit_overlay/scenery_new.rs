//! The New Scenery dialog: cut, recolour, set passability on and commit a
//! scenery piece over a [`SceneryPaintRun`](crate::scenerypaint::SceneryPaintRun)
//! (New, Clone and Edit all open here).

use super::*;

#[derive(Clone, Copy)]
pub(super) struct SceneryNewIds {
	pub(super) preview: WidgetId,
	pub(super) swatches: WidgetId,
	pub(super) chip: WidgetId,
	pub(super) color_label: WidgetId,
	pub(super) used: WidgetId,
	pub(super) all: WidgetId,
	pub(super) none: WidgetId,
	pub(super) reset: WidgetId,
	pub(super) mode: [WidgetId; 2],
	/// Checker / Ground, the preview's backdrop.
	pub(super) backdrop: [WidgetId; 2],
	/// The alpha-rule note's [`Reveal`]: the rule only says anything while the
	/// art comes from an image, and a Clone or an Edit starts from a piece that
	/// was already cut. Revealed, never rebuilt - importing a PNG into an open
	/// Edit brings the note back without minting new ids.
	pub(super) alpha_row: WidgetId,
	pub(super) pass: [WidgetId; 4],
	/// Image / Heightmap - the two things a piece is, each with its own
	/// controls. One [`Tabs`](wgpu_ui::Tabs), built once: switching tabs may not
	/// rebuild anything, or the typed name and the recolour go with it.
	pub(super) tabs: WidgetId,
	/// How high the piece stands, for the `higher` blend mode - an index into
	/// [`RELIEFS`], `0` = leave it inferred.
	pub(super) relief: WidgetId,
	/// The Heightmap tab: the relief preview, the two file keys, and the note
	/// that says which of drawn / inferred is on screen.
	pub(super) height_view: WidgetId,
	pub(super) height_import: WidgetId,
	pub(super) height_export: WidgetId,
	pub(super) height_clear: WidgetId,
	pub(super) height_label: WidgetId,
	pub(super) pack: WidgetId,
	pub(super) name_field: WidgetId,
	pub(super) id_field: WidgetId,
	pub(super) import: WidgetId,
	pub(super) size_label: WidgetId,
	pub(super) error: WidgetId,
	pub(super) cancel: WidgetId,
	pub(super) save: WidgetId,
}

/// New Scenery dialog state. Everything from `sprite` down is *derived* -
/// re-rasterizing the source is how a threshold change takes effect, and the
/// recolour is a 256-entry map applied on top rather than a paint over the
/// pixels, so every control is undoable by moving it back.
///
/// One struct, replaced wholesale by [`Overlay::open_scenery_new`]: the
/// struct literal there is the complete reseed (a new field cannot be
/// forgotten), and `hide` leaves it alone.
pub(super) struct SceneryNewState {
	pub(super) packs: Vec<String>,
	pub(super) grounds: Vec<[u8; 3]>,
	/// The art comes from an imported image, so the alpha thresholds apply. A
	/// Clone or an Edit starts `false` and flips when a PNG is imported over it.
	pub(super) from_image: bool,
	/// What the preview is judged against - the checkerboard by default, because
	/// only a checkerboard shows *how* see-through a shadow is.
	pub(super) backdrop: crate::scenerypaint::Backdrop,
	/// The last seen [`crate::scenerypaint::SceneryPaintRun::rev`] - an editor
	/// write (a PNG chosen in the native dialog) moves it and the dialog
	/// re-derives, the Tile Painter's `canvas_rev` contract.
	pub(super) rev: u64,
	/// The rasterized piece, before the recolour.
	pub(super) sprite: map_core::Sprite,
	pub(super) pass: Vec<u8>,
	pub(super) cells: (u16, u16),
	/// The palette indices the object uses, darkest first, and which of them a
	/// palette pick would retarget.
	pub(super) base: Vec<u8>,
	pub(super) sel: Vec<bool>,
	pub(super) anchor: usize,
	pub(super) remap: [u8; 256],
	pub(super) mode: crate::scenerypaint::RemapMode,
	pub(super) opts: map_core::RasterOpts,
	pub(super) color: u8,
	pub(super) rgba: Vec<u8>,
	/// The **drawn relief**, in the sprite's frame - `None` while the piece
	/// infers its height from its art, which is what every shipped piece but
	/// CRATER's does. Set by importing a picture, by opening on a piece that
	/// already carries one, and cleared by the tab's Clear key.
	pub(super) height: Option<Vec<u8>>,
	/// The piece is a **scarp** (`map_core::scarp_face`) - carried, not edited.
	///
	/// A wall is marked in the pack's `tune.json` and baked in, because which
	/// pieces are one is a judgement about the art that the bake makes once. The
	/// dialog has no control for it and offers none; what it must do is not
	/// *lose* it, so opening on a cliff and saving a new name keeps the flag
	/// instead of quietly turning the piece back into a ridge.
	pub(super) scarp: Option<bool>,
	/// The last [`crate::scenerypaint::SceneryPaintRun::hgt_rev`] seen - a
	/// height map the editor loaded lands on the next sync, the same contract
	/// the source image arrives by.
	pub(super) hgt_rev: u64,
	/// Read the loaded picture again on the next sync: the `Stands:` scale
	/// moved, and that is the number a grey means. Distinct from a *new* picture
	/// (`hgt_rev`), which is also a reason to re-read but not the same one.
	pub(super) refit: bool,
	/// The preview texture needs recomposing (a recolour, a threshold, or a
	/// palette that moved under it).
	pub(super) dirty: bool,
	/// The **height** preview needs recomposing (a drawn relief came or went, or
	/// the art under the inference moved).
	pub(super) height_dirty: bool,
	/// A threshold moved: rasterize the source again on the next sync, which is
	/// where the shell hands the source over. Distinct from a *new* source
	/// (`rev`), which also resets the recolour and re-seeds the fields -
	/// re-thresholding must keep both.
	pub(super) redo: bool,
}

impl Default for SceneryNewState {
	fn default() -> Self {
		Self {
			packs: Vec::new(),
			grounds: Vec::new(),
			from_image: false,
			backdrop: crate::scenerypaint::Backdrop::Checker,
			rev: 0,
			sprite: map_core::Sprite::default(),
			pass: Vec::new(),
			cells: (0, 0),
			base: Vec::new(),
			sel: Vec::new(),
			anchor: 0,
			remap: crate::scenerypaint::identity_remap(),
			mode: crate::scenerypaint::RemapMode::Ramp,
			opts: map_core::RasterOpts::default(),
			color: 1,
			rgba: Vec::new(),
			height: None,
			scarp: None,
			hgt_rev: 0,
			refit: false,
			dirty: false,
			height_dirty: false,
			redo: false,
		}
	}
}

/// What the `Stands:` dropdown offers, as `(label, peak and sunken)`.
///
/// `auto` is the first and the default: the relief a cut-out stands at is
/// *inferred* from its art (`map_core::Sprite::height_field`), and the guess is
/// right for the shipped libraries. The rest are for the piece the guess gets
/// wrong - a spire the sprite's short side reads as small, a hollow the family
/// name does not say is one. Named rather than numbered, because the number is
/// map pixels of elevation and nobody has that in their head.
pub(super) const RELIEFS: [(&str, Option<(u8, bool)>); 6] = [
	("auto", None),
	("low", Some((48, false))),
	("medium", Some((96, false))),
	("tall", Some((192, false))),
	("sunken", Some((96, true))),
	("sunken deep", Some((192, true))),
];

/// Which [`RELIEFS`] entry a piece's authored relief is, or `0` for inferred.
fn relief_index(peak: Option<u8>, sunken: Option<bool>) -> usize {
	match (peak, sunken) {
		(None, None) => 0,
		(peak, sunken) => RELIEFS
			.iter()
			.position(|(_, r)| *r == Some((peak.unwrap_or(96), sunken.unwrap_or(false))))
			// An authored pair no preset names still reads as authored: the
			// nearest one, rather than silently offering to throw it away.
			.unwrap_or(if sunken.unwrap_or(false) { 4 } else { 2 }),
	}
}

impl Overlay {
	/// Opens New Scenery over `run` (the editor-owned context). `palette` is the
	/// project's 256x3 table the image quantizes against, `rgba` the live
	/// (cycled) one the preview draws through.
	pub fn open_scenery_new(
		&mut self,
		chrome: &mut MenuChrome,
		run: &crate::scenerypaint::SceneryPaintRun,
		palette: &[u8],
		rgba: &[u8],
	) {
		// A carried piece brings its own passability along; the radios start where
		// it already is, so opening a Clone and pressing Save changes nothing.
		let mut opts = map_core::RasterOpts::default();
		if let Some(piece) = run.piece.as_ref() {
			if let Some(&pass) = piece.pass.iter().find(|&&p| p != map_core::PASS_EMPTY) {
				opts.pass = pass;
			}
		}
		self.sn = SceneryNewState {
			packs: run.packs.clone(),
			grounds: run.grounds.clone(),
			rev: run.rev,
			hgt_rev: run.hgt_rev,
			opts,
			rgba: rgba.to_vec(),
			..Default::default()
		};
		self.derive_scenery(run, palette);

		let swatches = crate::tilepaint::compose_swatches_rgba(&self.sn.rgba);
		match self.sn_swatch_tex {
			Some(id) => chrome.replace_texture(id, &swatches, 16, 16),
			None => self.sn_swatch_tex = Some(chrome.register_texture(&swatches, 16, 16)),
		}
		self.recompose_scenery_preview(chrome);
		self.recompose_scenery_height(chrome);
		self.build_scenery_new(run);
		// The tree is built empty and filled by the per-frame view sync; run it
		// once here so the first frame already shows the piece rather than
		// flashing "no image" at a dialog that was opened *with* one.
		if let Dialog::SceneryNew(ids) = self.dialog {
			self.sync_scenery_view(ids);
		}
		self.events.clear();
		self.visible = true;
	}

	/// Re-derive the piece and everything that hangs off it: rasterize the source
	/// image at the current thresholds, or take the art of the piece a Clone or
	/// an Edit opened on.
	///
	/// The recolour map is **kept** across a threshold change - the colours you
	/// retargeted are the same colours - but any entry the new rasterization no
	/// longer uses simply stops mattering.
	fn derive_scenery(&mut self, run: &crate::scenerypaint::SceneryPaintRun, palette: &[u8]) {
		self.sn.from_image = run.uses_image();
		let derived = match (self.sn.from_image, run.piece.as_ref()) {
			// A carried piece is already cut: the thresholds have nothing to say
			// about it, but the pass radios still re-stamp the cells it covers.
			(false, Some(p)) => {
				let pass =
					p.pass.iter().map(|&v| if v == map_core::PASS_EMPTY { v } else { self.sn.opts.pass }).collect();
				Some((p.sprite.clone(), pass, p.cells_w, p.cells_h))
			}
			(false, None) => None,
			(true, _) => map_core::rasterize(&run.src, run.src_w as usize, run.src_h as usize, palette, &self.sn.opts),
		};
		let (sprite, pass, cw, ch) = derived.unwrap_or_else(|| (map_core::Sprite::default(), Vec::new(), 0, 0));
		self.sn.base = crate::scenerypaint::sub_palette(&sprite, palette);
		self.sn.sel = vec![false; self.sn.base.len()];
		self.sn.anchor = 0;
		// A relief is drawn for one silhouette: it survives a re-cut only if the
		// frame it was drawn in still fits. The piece a Clone or an Edit opened
		// on brings its own along - that is what keeps an Edit from quietly
		// throwing away a height map somebody drew.
		let texels = sprite.width as usize * sprite.height as usize;
		let carried = match (self.sn.from_image, run.piece.as_ref()) {
			(false, Some(p)) => p.height.clone(),
			_ => self.sn.height.take(),
		};
		self.sn.height = carried.filter(|h| h.len() == texels && texels > 0);
		// Which shape the piece stands in comes along with it, so the Heightmap
		// tab draws a cliff as the wall it is. Unlike the relief it is not tied
		// to a frame - it says how to read whatever silhouette there is - so a
		// re-cut keeps it where a drawn height map would be dropped.
		self.sn.scarp = run.piece.as_ref().and_then(|p| p.scarp);
		self.sn.sprite = sprite;
		self.sn.pass = pass;
		self.sn.cells = (cw, ch);
		self.sn.dirty = true;
		self.sn.height_dirty = true;
	}

	/// The relief the piece would ship with: the drawn one where there is one,
	/// the inference where there is not - the very split
	/// `SceneryPiece::height_field` makes, so what the tab shows is what the map
	/// will stand the object at.
	fn scenery_height_field(&self) -> Vec<u8> {
		if let Some(height) = &self.sn.height {
			return height.clone();
		}
		let sprite = self.scenery_piece_sprite();
		if sprite.is_empty() {
			return Vec::new();
		}
		let (peak, sunken) = match self.scenery_relief() {
			Some((peak, sunken)) => (peak, sunken),
			None => (self.scenery_family_peak(&sprite), false),
		};
		// No rim: tracing one is a bake input, and this is the live inference.
		let scarp = self.sn.scarp.unwrap_or(false);
		sprite.height_field(
			&self.scenery_brightness(),
			&map_core::HeightOpts { peak, sunken, pyramid: false, scarp, rim: &[], foot: &[] },
		)
	}

	/// The peak the Heightmap tab reads a picture at: the `Stands:` choice, or
	/// what the art falls back to.
	fn scenery_peak(&self) -> u8 {
		self.scenery_relief()
			.map(|(peak, _)| peak)
			.unwrap_or_else(|| self.scenery_family_peak(&self.scenery_piece_sprite()))
	}

	/// The `Stands:` dropdown's value, or `None` for `auto`.
	/// The peak an unauthored piece falls back to: what its footprint implies,
	/// brought down for a family that lies on the ground rather than standing on
	/// it (`map_core::family_peak`).
	///
	/// The family comes from the **typed name**, which is where
	/// `file_scenery_piece` gets it too - so renaming a piece "Dune 7" in the
	/// dialog drops the Heightmap tab's scale to a dune's on the spot, and the
	/// preview cannot show a height the piece will not ship at.
	fn scenery_family_peak(&self, sprite: &map_core::Sprite) -> u8 {
		let Dialog::SceneryNew(ids) = self.dialog else { return sprite.default_peak() };
		let family = map_core::piece_family(self.text(ids.name_field).trim());
		map_core::family_peak(&family, sprite.default_peak())
	}

	fn scenery_relief(&self) -> Option<(u8, bool)> {
		let Dialog::SceneryNew(ids) = self.dialog else { return None };
		let i = self.ui.get::<Select>(ids.relief).map(Select::selected).unwrap_or(0);
		RELIEFS.get(i).and_then(|(_, r)| *r)
	}

	/// Per-index brightness off the dialog's live palette - what the inference
	/// reads a lit face off. The table is 256 RGBA quads here and 256 RGB
	/// triples in `map_core`, so it is repacked rather than reinterpreted.
	fn scenery_brightness(&self) -> [u8; 256] {
		let mut palette = vec![0u8; 768];
		for i in 0..256usize {
			let at = i * 4;
			if at + 3 <= self.sn.rgba.len() {
				palette[i * 3..i * 3 + 3].copy_from_slice(&self.sn.rgba[at..at + 3]);
			}
		}
		map_core::brightness_table(&palette)
	}

	/// The piece as it will be saved: the rasterization with the recolour
	/// applied. Kept in one place so the preview and the commit cannot drift.
	fn scenery_piece_sprite(&self) -> map_core::Sprite {
		crate::scenerypaint::recolor(&self.sn.sprite, &self.sn.remap)
	}

	/// Push the recoloured piece into the preview texture (resized if the
	/// thresholds changed the crop).
	fn recompose_scenery_preview(&mut self, chrome: &mut MenuChrome) {
		let sprite = self.scenery_piece_sprite();
		let (w, h) = (sprite.width.max(1) as u32, sprite.height.max(1) as u32);
		let rgba = if sprite.is_empty() {
			vec![0u8; 4]
		} else {
			crate::scenerypaint::compose_preview_rgba(&sprite, &self.sn.rgba)
		};
		match self.sn_preview_tex {
			Some(id) => chrome.replace_texture(id, &rgba, w, h),
			None => self.sn_preview_tex = Some(chrome.register_texture(&rgba, w, h)),
		}
		self.sn.dirty = false;
	}

	/// Push the relief into the Heightmap tab's picture: the same greyscale
	/// `map_core::height_to_grey` writes to a file, so what is on screen is what
	/// Save PNG hands over and what Import PNG reads back.
	fn recompose_scenery_height(&mut self, chrome: &mut MenuChrome) {
		let sprite = self.scenery_piece_sprite();
		let (w, h) = (sprite.width.max(1) as u32, sprite.height.max(1) as u32);
		let field = self.scenery_height_field();
		let rgba = if field.len() != w as usize * h as usize {
			vec![0u8; 4]
		} else {
			// Opaque grey: a height map has no transparency to show, and a
			// checkerboard behind low ground would read as relief that is not there.
			map_core::height_to_grey(&field, self.scenery_peak()).into_iter().flat_map(|v| [v, v, v, 255]).collect()
		};
		let (w, h) = if rgba.len() == 4 { (1, 1) } else { (w, h) };
		match self.sn_height_tex {
			Some(id) => chrome.replace_texture(id, &rgba, w, h),
			None => self.sn_height_tex = Some(chrome.register_texture(&rgba, w, h)),
		}
		self.sn.height_dirty = false;
	}

	/// Builds the authoring tree: a **two-tab left column** beside the palette
	/// column (swatches, the used-colour strip and its remap mode, pack, name,
	/// id), over Cancel/Save. Built once per open - runtime changes go through
	/// setters, never a rebuild (a rebuild would lose the typed name and the
	/// selection, and switching tabs is a runtime change).
	///
	/// The tabs are the two pictures a piece is made of, and each carries the
	/// controls that belong to it: **Image** the cut-out itself (well, backdrop,
	/// import, the alpha rule, passability), **Heightmap** how high it stands
	/// (the relief picture, its own import and export, and the `Stands:` scale
	/// they are read at). They share the palette column, because a recolour is
	/// about the piece and not about either picture.
	///
	/// An **Edit** is the one shape difference: its id field and its pack
	/// dropdown are built disabled, because a placement stores the id and the
	/// library is keyed by the pack - moving either would orphan every object
	/// already on a map. Clone exists for exactly that.
	fn build_scenery_new(&mut self, run: &crate::scenerypaint::SceneryPaintRun) {
		use crate::scenerypaint::{SpriteView, SubPalette};
		use crate::tilepaint::{Chip, PASSES, SwatchGrid};
		let preview_tex = self.sn_preview_tex.expect("registered in open_scenery_new");
		let swatch_tex = self.sn_swatch_tex.expect("registered in open_scenery_new");
		let in_place = run.mode.in_place();

		let preview = SpriteView::new(preview_tex);
		let swatches = SwatchGrid::new(swatch_tex, self.sn.color);
		let chip = Chip::new(crate::tilepaint::slot_color(&self.sn.rgba, self.sn.color), 24.0);
		let color_label = Label::new(format!("color {}", self.sn.color)).small().with_id();
		let used = SubPalette::new();
		let (all, none, reset) =
			(Button::new("All").small(), Button::new("None").small(), Button::new("Reset").small());
		// Authoring from nothing *imports*; a Clone or an Edit already has art, so
		// the same key replaces it (and brings the alpha note back with it).
		let import = Button::new(if run.piece.is_some() { "Replace art..." } else { "Import PNG..." });
		let size_label = Label::new("no image").small().muted().with_id();
		let name_field = TextInput::with_text(run.name_text.clone()).max_len(48).placeholder("(from the file name)");
		let id_field = TextInput::with_text(run.id_text.clone())
			.charset(Charset::Slug)
			.max_len(32)
			.placeholder("(from the file name)")
			.disabled(in_place);
		let error = Label::new("").small().with_id();
		let cancel = Button::new("Cancel").secondary();
		let save = Button::new("Save").primary();

		let height_view = SpriteView::new(self.sn_height_tex.expect("registered in open_scenery_new"));
		let height_import = Button::new("Import PNG...");
		let height_export = Button::new("Save PNG...");
		let height_clear = Button::new("Clear").small();
		let height_label = Label::new("").small().muted().with_id();

		let mut ids = SceneryNewIds {
			preview: preview.id(),
			tabs: WidgetId::NONE,
			height_view: height_view.id(),
			height_import: height_import.id(),
			height_export: height_export.id(),
			height_clear: height_clear.id(),
			height_label: height_label.id(),
			swatches: swatches.id(),
			chip: chip.id(),
			color_label: color_label.id(),
			used: used.id(),
			all: all.id(),
			none: none.id(),
			reset: reset.id(),
			mode: [WidgetId::NONE; 2],
			backdrop: [WidgetId::NONE; 2],
			alpha_row: WidgetId::NONE,
			pass: [WidgetId::NONE; 4],
			relief: WidgetId::NONE,
			pack: WidgetId::NONE,
			name_field: name_field.id(),
			id_field: id_field.id(),
			import: import.id(),
			size_label: size_label.id(),
			error: error.id(),
			cancel: cancel.id(),
			save: save.id(),
		};

		let row = || Linear::row().spacing(8.0).cross_align(CrossAlign::Center);
		let label = |t: &str| Label::new(t.to_string()).small().muted();
		// The backdrop is a control, not a preference: the checkerboard shows how
		// see-through the shadow is, the pack's ground shows whether it reads on
		// the terrain it will land on, and both are worth a look.
		let mut backdrop_row = row().push(label("Behind:"));
		for (i, name) in ["Checker", "Ground"].into_iter().enumerate() {
			let rb = Radio::new(name).with_selected(i == 0);
			ids.backdrop[i] = rb.id();
			backdrop_row = backdrop_row.push(rb);
		}
		// The alpha rule, stated rather than offered: it is fixed (map_core's
		// `ImageBand`), because a piece has to cut the same way here as it does
		// in the offline bake. One `Reveal`, so an Edit that imports a PNG gets
		// the note back without a rebuild (which would lose the typed name and
		// the recolour).
		let alpha_row = wgpu_ui::Reveal::new(
			column()
				.push(label("Alpha: 0% = nothing, 50% = shadow, anything else = ink"))
				.push(label("Ink takes the nearest palette color; shadow is 50% black")),
		)
		.with_shown(run.uses_image());
		ids.alpha_row = alpha_row.id();
		let mut left = column()
			.push(preview)
			.push(backdrop_row)
			.push(row().push(import).push(size_label))
			.push(alpha_row)
			.push(label("Blocks movement as"));
		let mut pass_row = Linear::row().spacing(6.0);
		for (i, name) in PASSES.iter().enumerate() {
			let rb = Radio::new(*name).with_selected(self.sn.opts.pass.min(3) as usize == i);
			ids.pass[i] = rb.id();
			pass_row = pass_row.push(rb);
		}
		left = left.push(pass_row);

		// The Heightmap tab. A relief is a picture like the art is, so it is
		// looked at the same way: the same well, its own import, and an export
		// because a height map nobody can get out of the editor is one nobody
		// can paint on.
		//
		// `Stands:` lives here rather than beside the art, because it is what a
		// grey *means*: white in the picture is the peak, and with nothing drawn
		// it is the scale the inference is stretched to. One number, one place.
		let relief = Select::new(RELIEFS.iter().map(|(name, _)| *name)).small().with_selected(relief_index(
			run.piece.as_ref().and_then(|p| p.peak),
			run.piece.as_ref().and_then(|p| p.sunken),
		));
		ids.relief = relief.id();
		let height = column()
			.push(height_view)
			.push(row().push(height_import).push(height_export).push(height_clear))
			.push(height_label)
			.push(label("White is the top of the object, black is ground level."))
			.push(label("Nothing drawn = the height is inferred from the art."))
			// Sized to its own widest option rather than flexed: a control that
			// claims the width it is offered inside an auto-sizing window drags
			// the dialog off screen.
			.push(row().push(label("Stands:")).push(relief));

		let tabs = wgpu_ui::Tabs::new().tab("Image", left).tab("Heightmap", height);
		ids.tabs = tabs.id();

		let grid_w = 16.0 * crate::tilepaint::SW;
		let mut mode_row = Linear::row().spacing(6.0);
		for (i, name) in ["Ramp", "Flat"].into_iter().enumerate() {
			let rb = Radio::new(name).with_selected(i == 0);
			ids.mode[i] = rb.id();
			mode_row = mode_row.push(rb);
		}
		let mut right = column()
			.push(Linear::row().child(swatches, Length::Fixed(grid_w)))
			.push(row().push(chip).push(color_label))
			.push(label("Colors used - pick some, then a new color above"))
			.push(used)
			.push(row().push(all).push(none).push(reset).push(mode_row));
		// Always a dropdown, even for a one-entry list: which pack a cut-out
		// belongs to is the user's call, and a map that uses one tileset is no
		// reason to hide the rest of them. Disabled in an Edit - moving a piece
		// between libraries is a Clone plus a delete, not a silent rewrite.
		right = right.push(label("Pack"));
		let pack = Select::new(self.sn.packs.iter().map(String::as_str))
			.small()
			.with_selected(run.pack_sel)
			.disabled(in_place)
			.max_visible(10);
		ids.pack = pack.id();
		right = right
			.push(row().child(pack, Length::Flex(1.0)))
			.push(label("Name"))
			.push(row().child(name_field, Length::Flex(1.0)))
			.push(label(if in_place { "Id (fixed - placements point at it)" } else { "Id" }))
			.push(row().child(id_field, Length::Flex(1.0)));
		right = status_slot(right, error, grid_w, 2);

		let content = column().push(Linear::row().spacing(16.0).push(tabs).push(right)).push(buttons(cancel, save));
		let win = dialog(run.mode.title(), content);
		self.win_id = Some(win.id());
		self.ui = Ui::new(modal(win));
		self.dialog = Dialog::SceneryNew(ids);
		self.blocking = true;
	}

	/// Pushes the live palette table and any editor-side source write (a PNG
	/// picked in the native dialog, keyed by its revision) into the open dialog.
	/// The shell calls this every frame before `render`.
	pub fn sync_scenery_new(
		&mut self,
		chrome: &mut MenuChrome,
		run: &crate::scenerypaint::SceneryPaintRun,
		palette: &[u8],
		rgba: &[u8],
	) {
		if !matches!(self.dialog, Dialog::SceneryNew(_)) {
			return;
		}
		if self.sn.rgba != rgba {
			self.sn.rgba = rgba.to_vec();
			let swatches = crate::tilepaint::compose_swatches_rgba(&self.sn.rgba);
			if let Some(id) = self.sn_swatch_tex {
				chrome.update_texture(id, &swatches);
			}
			self.sn.dirty = true;
		}
		if run.rev != self.sn.rev {
			// A *different* image: the recolour was about the old one's colours,
			// and the fields were named after the old one's file. (In an Edit the
			// editor leaves both texts alone - the piece keeps its identity while
			// its art is replaced.)
			self.sn.rev = run.rev;
			self.sn.redo = false;
			self.sn.remap = crate::scenerypaint::identity_remap();
			self.derive_scenery(run, palette);
			let from_image = self.sn.from_image;
			if let Dialog::SceneryNew(ids) = self.dialog {
				self.set_text(ids.name_field, &run.name_text);
				self.set_text(ids.id_field, &run.id_text);
				// The bands are back in play - an Edit that replaced its art is
				// authoring from an image again.
				if let Some(r) = self.ui.get_mut::<wgpu_ui::Reveal>(ids.alpha_row) {
					r.set_shown(from_image);
				}
			}
		} else if std::mem::take(&mut self.sn.redo) {
			// The same image at new thresholds: keep the recolour and the
			// typed name, and only re-cut.
			let remap = self.sn.remap;
			self.derive_scenery(run, palette);
			self.sn.remap = remap;
		}
		// A height map the editor loaded, on the source image's terms. It is fitted
		// to the art here and not stored as a picture, because the art is what says
		// which pixels are even the object - and re-fitted when the scale it is
		// read at moves.
		let refit = std::mem::take(&mut self.sn.refit);
		if (run.hgt_rev != self.sn.hgt_rev || refit) && !run.hgt_src.is_empty() {
			self.sn.hgt_rev = run.hgt_rev;
			let sprite = self.scenery_piece_sprite();
			let peak = self.scenery_peak();
			let fitted = map_core::height_from_grey(
				&run.hgt_src,
				run.hgt_w as usize,
				run.hgt_h as usize,
				&sprite,
				self.sn.cells.0,
				self.sn.cells.1,
				peak,
			);
			match fitted {
				Some(plane) => self.sn.height = Some(plane),
				// Said inline rather than swallowed: the one thing that can go
				// wrong here is a picture drawn at another size, and a user who
				// sees nothing happen has no way to guess that.
				None => self.scenery_new_error(&format!(
					"the height map must be {}x{} or {}x{} (the whole footprint)",
					sprite.width,
					sprite.height,
					self.sn.cells.0 as u32 * 64,
					self.sn.cells.1 as u32 * 64,
				)),
			}
			self.sn.height_dirty = true;
		}
		if self.sn.dirty {
			self.recompose_scenery_preview(chrome);
			self.sn.height_dirty = true; // the art moved, so the inference did too
		}
		if self.sn.height_dirty {
			self.recompose_scenery_height(chrome);
		}
		if let Dialog::SceneryNew(ids) = self.dialog {
			self.sync_scenery_view(ids);
		}
	}

	/// Per-frame view sync: the preview's size and backdrop, the used-colour
	/// strip, the current-colour chip and the footprint readout.
	fn sync_scenery_view(&mut self, ids: SceneryNewIds) {
		let sprite = self.scenery_piece_sprite();
		let cells = self.sn.cells;
		let empty = sprite.is_empty();
		let ground = self.scenery_ground();
		let backdrop = self.sn.backdrop;
		if let Some(v) = self.ui.get_mut::<crate::scenerypaint::SpriteView>(ids.preview) {
			v.set_size(sprite.width as u32, sprite.height as u32);
			v.set_ground(ground);
			v.set_backdrop(backdrop);
		}
		// The relief well shows the same box the art does, so the two tabs line up
		// pixel for pixel when you flick between them.
		let drawn = self.sn.height.is_some();
		let peak = self.scenery_peak();
		if let Some(v) = self.ui.get_mut::<crate::scenerypaint::SpriteView>(ids.height_view) {
			v.set_size(sprite.width as u32, sprite.height as u32);
			v.set_backdrop(crate::scenerypaint::Backdrop::Ground);
			v.set_ground(Rgba::rgb(0, 0, 0));
		}
		let height_note = match (empty, drawn) {
			(true, _) => "no image".to_string(),
			// Named for what it is, because the two are not the same promise: one
			// is a measurement somebody made, the other a guess off the art.
			(_, true) => format!("drawn - white stands {peak} px tall"),
			(_, false) => format!("inferred from the art - white stands {peak} px tall"),
		};
		self.set_label(ids.height_label, &height_note);
		let cells_note = if empty {
			"no image".to_string()
		} else {
			let blocked = self.sn.pass.iter().filter(|&&p| p != map_core::PASS_EMPTY).count();
			format!("{}x{} px   {}x{} cells   {blocked} blocked", sprite.width, sprite.height, cells.0, cells.1)
		};
		self.set_label(ids.size_label, &cells_note);
		let swatch = |i: u8| crate::tilepaint::slot_color(&self.sn.rgba, i);
		let cells: Vec<Rgba> = self.sn.base.iter().map(|&i| swatch(self.sn.remap[i as usize])).collect();
		let sel = self.sn.sel.clone();
		let hovered = self.ui.get::<crate::scenerypaint::SubPalette>(ids.used).and_then(|s| s.hover());
		if let Some(strip) = self.ui.get_mut::<crate::scenerypaint::SubPalette>(ids.used) {
			strip.set_cells(cells, sel);
		}
		let color = self.sn.color;
		if let Some(c) = self.ui.get_mut::<crate::tilepaint::Chip>(ids.chip) {
			c.set_color(swatch(color));
		}
		self.set_label(ids.color_label, &format!("color {color}"));
		// A hovered used-colour rings its slot in the big grid, so "where does
		// this one live" needs no hunting.
		let hint = hovered.and_then(|i| self.sn.base.get(i)).map(|&i| self.sn.remap[i as usize]);
		if let Some(g) = self.ui.get_mut::<crate::tilepaint::SwatchGrid>(ids.swatches) {
			g.set_sel(color);
			g.set_hint(hint);
		}
	}

	/// The ground tone the preview is judged against: the selected pack's.
	fn scenery_ground(&self) -> Rgba {
		let i = match self.dialog {
			Dialog::SceneryNew(ids) if ids.pack != WidgetId::NONE => {
				self.ui.get::<Select>(ids.pack).map(Select::selected).unwrap_or(0)
			}
			_ => 0,
		};
		let g = self.sn.grounds.get(i).copied().unwrap_or([96, 96, 96]);
		Rgba::rgb(g[0], g[1], g[2])
	}

	/// Visual-suite hook: select every used colour and retarget it at `target`
	/// under the current mode - exactly what a Select-All followed by a palette
	/// click does, minus the two pointer events.
	#[cfg(test)]
	pub(crate) fn scenery_recolor_for_test(
		&mut self,
		chrome: &mut MenuChrome,
		target: u8,
		mode: crate::scenerypaint::RemapMode,
	) {
		self.sn.sel = vec![true; self.sn.base.len()];
		self.sn.mode = mode;
		self.sn.color = target;
		let (base, sel) = (self.sn.base.clone(), self.sn.sel.clone());
		crate::scenerypaint::apply_remap(&base, &sel, target, mode, &mut self.sn.remap);
		self.recompose_scenery_preview(chrome);
		if let Dialog::SceneryNew(ids) = self.dialog {
			self.sync_scenery_view(ids);
		}
	}

	/// Visual-suite hook: show the Heightmap tab, exactly as clicking its header
	/// does.
	#[cfg(test)]
	pub(crate) fn scenery_show_heightmap_for_test(&mut self) {
		if let Dialog::SceneryNew(ids) = self.dialog {
			if let Some(tabs) = self.ui.get_mut::<wgpu_ui::Tabs>(ids.tabs) {
				tabs.set_active(1);
			}
			self.sync_scenery_view(ids);
		}
	}

	/// Shows a commit failure inline (the dialog stays open, the work kept).
	pub fn scenery_new_error(&mut self, message: &str) {
		if let Dialog::SceneryNew(ids) = self.dialog {
			self.set_label(ids.error, message);
		}
	}

	/// One frame of the dialog's control dispatch (the render match arm).
	pub(super) fn dispatch_scenery_new(&mut self, ids: SceneryNewIds) -> Outcome {
		let mut outcome = Outcome::Idle;
		use crate::scenerypaint::{Backdrop, RemapMode, SubPalette, apply_remap, click_selection, identity_remap};
		// The pass radios mean "re-derive": the piece is a function of the source
		// image and its footprint verdict. (The alpha rule is fixed, so there is
		// nothing else here that re-cuts.)
		if self.radio_group(&ids.pass).is_some_and(|sel| {
			self.sn.opts.pass = sel as u8;
			true
		}) {
			self.sn.redo = true;
			outcome = Outcome::SceneryRederive;
		}
		// Ramp / Flat is one radio group.
		if let Some(sel) = self.radio_group(&ids.mode) {
			self.sn.mode = if sel == 0 { RemapMode::Ramp } else { RemapMode::Flat };
		}
		// Checker / Ground is another.
		if let Some(sel) = self.radio_group(&ids.backdrop) {
			self.sn.backdrop = if sel == 0 { Backdrop::Checker } else { Backdrop::Ground };
		}
		// The used-colour strip's own gesture: plain picks one, Ctrl
		// toggles, Shift takes the run from the anchor.
		let picks = self.ui.get_mut::<SubPalette>(ids.used).map(SubPalette::take_picks).unwrap_or_default();
		for (i, mods) in picks {
			click_selection(&mut self.sn.sel, &mut self.sn.anchor, i, mods);
		}
		if self.ui.fired(ids.all) {
			self.sn.sel = vec![true; self.sn.base.len()];
		}
		if self.ui.fired(ids.none) {
			self.sn.sel = vec![false; self.sn.base.len()];
		}
		if self.ui.fired(ids.reset) {
			self.sn.remap = identity_remap();
			self.sn.dirty = true;
		}
		// A palette pick retargets whatever is selected; with nothing
		// selected it only arms the colour, so a stray click cannot
		// silently repaint the object.
		if self.ui.fired(ids.swatches) {
			if let Some(g) = self.ui.get::<crate::tilepaint::SwatchGrid>(ids.swatches) {
				self.sn.color = g.sel();
			}
			if self.sn.sel.iter().any(|&s| s) {
				let (base, sel, color, mode) = (self.sn.base.clone(), self.sn.sel.clone(), self.sn.color, self.sn.mode);
				apply_remap(&base, &sel, color, mode, &mut self.sn.remap);
				self.sn.dirty = true;
			}
		}
		if self.ui.fired(ids.pack) {
			self.sn.dirty = true; // a different pack, a different ground
		}
		// `Stands:` is the scale a grey is read at, so moving it re-reads the
		// picture that is already loaded rather than only labelling it
		// differently - what you see in the well is what the piece will stand at.
		if self.ui.fired(ids.relief) {
			// ...but only a picture that is actually loaded is re-read; a relief
			// that was cleared stays cleared, and one carried in from a piece is
			// already in the units it was saved in.
			self.sn.refit = self.sn.height.is_some();
			self.sn.height_dirty = true;
			outcome = Outcome::SceneryRederive;
		}
		// The relief's own two file keys, and the way back to the inference.
		if self.ui.fired(ids.height_clear) {
			self.sn.height = None;
			self.sn.height_dirty = true;
		}
		if self.ui.fired(ids.height_import) {
			outcome = Outcome::SceneryImportHeightPng;
		} else if self.ui.fired(ids.height_export) {
			let sprite = self.scenery_piece_sprite();
			let field = self.scenery_height_field();
			outcome = Outcome::SceneryExportHeightPng {
				grey: map_core::height_to_grey(&field, self.scenery_peak()),
				w: sprite.width as u32,
				h: sprite.height as u32,
			};
		} else if self.ui.fired(ids.import) {
			outcome = Outcome::SceneryImportPng;
		} else if self.ui.fired(ids.cancel) {
			self.hide();
			outcome = Outcome::SceneryNewClose;
		} else if self.ui.fired(ids.save) {
			// The dialog stays open: the shell hides it on success or
			// pushes the failure back in, so a bad id keeps the work.
			let pack = if ids.pack != WidgetId::NONE {
				let i = self.ui.get::<Select>(ids.pack).map(Select::selected).unwrap_or(0);
				self.sn.packs.get(i).cloned().unwrap_or_default()
			} else {
				self.sn.packs.first().cloned().unwrap_or_default()
			};
			let relief = self.ui.get::<Select>(ids.relief).map(Select::selected).unwrap_or(0);
			outcome = Outcome::SceneryCommit {
				pack,
				id: self.text(ids.id_field).trim().to_string(),
				name: self.text(ids.name_field).trim().to_string(),
				sprite: self.scenery_piece_sprite(),
				pass: self.sn.pass.clone(),
				cells: self.sn.cells,
				relief: RELIEFS.get(relief).and_then(|(_, r)| *r),
				// The drawn relief only - an inferred one is written as nothing, so
				// a piece nobody drew a height map for carries no `.hgt` and reads
				// its height off its art exactly as it always did.
				height: self.sn.height.clone(),
			};
		}
		outcome
	}
}
