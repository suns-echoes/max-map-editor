//! The New Map and Import WRL dialogs, together because they share the
//! tile-set / water-tiles picker machinery: the pack rows, the palette-
//! owner and water radio groups, and the live preview-strip cache.

use super::*;

#[derive(Clone, Copy)]
pub(super) struct NewMapIds {
	pub(super) create: WidgetId,
	pub(super) cancel: WidgetId,
	pub(super) preset: WidgetId,
	pub(super) width: WidgetId,
	pub(super) height: WidgetId,
	/// The palette selector ("from selected tileset" | tileset palettes |
	/// user palettes).
	pub(super) palette: WidgetId,
	/// The "Palette preview" checkbox: checked = the strips recolour with the
	/// effective palette (selector or owner radio); unchecked = original
	/// pack colours.
	pub(super) preview: WidgetId,
	/// The inline error slot: a refused Create says why here ([`status_slot`]).
	pub(super) status: WidgetId,
}

#[derive(Clone, Copy)]
pub(super) struct ImportWrlIds {
	/// Cancel/Abort + the Import button (Import → match at the picker stage,
	/// Import-tiles → finish at the review stage) + review-only extras.
	pub(super) cancel: WidgetId,
	pub(super) import: WidgetId,
	pub(super) ignore: WidgetId,
	pub(super) dest_project: WidgetId,
	pub(super) dest_user: WidgetId,
	pub(super) error: WidgetId,
}

impl Overlay {
	/// Opens the New Map form: a size preset, a Width/Height line, the palette
	/// selector, and the tile-set picker sections ([`pack_rows`](Self::pack_rows)).
	/// `palettes` feeds the selector (see [`crate::newmap::palette_choices`]);
	/// `shape` is the land/water PNG File → New Terrain from Image picked
	/// (`None` for a plain New Map) - Create then carves it in.
	pub fn open_newmap(
		&mut self,
		chrome: &mut MenuChrome,
		packs: Vec<PackEntry>,
		assets_root: &std::path::Path,
		palettes: Vec<PaletteChoice>,
		tileset_palettes: usize,
		preview: bool,
		shape: Option<PathBuf>,
	) {
		let preset = Select::new(["Classic 112x112", "Mega 224x224", "Giga 448x448", "Custom"]);
		let width = TextInput::with_text("112").charset(Charset::Digits).align(wgpu_ui::TextAlign::Right);
		let height = TextInput::with_text("112").charset(Charset::Digits).align(wgpu_ui::TextAlign::Right);
		let mut palette_sel = Select::new(palettes.iter().map(|c| c.label.clone()));
		if tileset_palettes > 0 && palettes.len() > tileset_palettes + 1 {
			// Tileset palettes above, user palettes below the rule.
			palette_sel = palette_sel.separator_after(tileset_palettes);
		}
		let create = Button::new("Create").primary();
		let cancel = Button::new("Cancel").secondary();
		// Palette preview: on = the strips recolour with the effective palette
		// (the selector's choice, or the owner radio's pack); off = original
		// pack colours. Defaults to the persisted `[Preferences]` value.
		let preview_cb = Checkbox::new("").with_checked(preview);
		let status = Label::new("").small().with_id();
		let ids = NewMapIds {
			create: create.id(),
			cancel: cancel.id(),
			preset: preset.id(),
			width: width.id(),
			height: height.id(),
			palette: palette_sel.id(),
			preview: preview_cb.id(),
			status: status.id(),
		};
		// A changed palette list (a user palette added/removed since the last
		// open) invalidates the cached per-choice atlases.
		if self.nm_palettes != palettes {
			self.preview_cache.clear();
			self.nm_palettes = palettes;
		}
		self.nm_palette_sel = palette_sel.id();
		self.nm_preview = preview_cb.id();
		self.packs = packs;
		self.owner_choice = None;
		// The initial atlas matches the open-time state: with the preview on,
		// the selector's "from selected tileset" means the default owner's
		// palette; off means original pack colours.
		let key = if preview { self.initial_preview_key() } else { (0, self.default_water()) };
		let tex = self.resolve_pack_tex(chrome, assets_root, key);
		// Width and Height share one line; the numbers align right like a
		// column. Fixed field widths keep the row's natural measure inside the
		// dialog strut (a text field's own natural width is generous).
		let size_row = Linear::row()
			.spacing(8.0)
			.cross_align(CrossAlign::Center)
			.child(Label::new("Width").small(), Length::Fixed(78.0))
			.child(width, Length::Fixed(100.0))
			.child(Label::new("Height").small(), Length::Fixed(52.0))
			.child(height, Length::Fixed(100.0));
		let preview_row = Linear::row()
			.spacing(8.0)
			.cross_align(CrossAlign::Center)
			.push(Label::new("Palette preview").small())
			.push(preview_cb);
		let col = column()
			.push(width_strut(NEWMAP_W))
			.push(field_row("Preset", preset))
			.push(size_row)
			.push(field_row("Palette", palette_sel))
			.push(preview_row);
		let mut col = self.pack_rows(col, tex);
		if let Some(path) = &shape {
			let name = path.file_name().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
			col = col.push(Label::new(format!("Terrain shape: {name}")).small().muted());
		}
		let content = status_slot(col, status, NEWMAP_W, 1).push(buttons(cancel, create));
		let win = dialog("New Map", content);
		self.win_id = Some(win.id());
		self.ui = Ui::new(modal(win));
		self.ui.focus_first();
		self.dialog = Dialog::NewMap(ids);
		self.newmap_shape = shape;
		self.events.clear();
		self.visible = true;
	}

	/// The preview atlas for `key` (palette choice, water pack), built and
	/// cached - the open path with device access
	/// ([`provide_preview_tex`](Self::provide_preview_tex) fills combinations
	/// requested after open). Key `(0, _)` = original pack colours.
	fn resolve_pack_tex(
		&mut self,
		chrome: &mut MenuChrome,
		assets_root: &std::path::Path,
		key: (usize, usize),
	) -> TextureId {
		if let Some(&t) = self.preview_cache.get(&key) {
			return t;
		}
		let override_pal =
			self.nm_palettes.get(key.0).and_then(|c| c.path.as_ref()).and_then(|p| crate::palette_io::load(p).ok());
		let water = self.packs.get(key.1).map(|p| p.name.clone()).unwrap_or_default();
		let (rgba, _rows) = crate::newmap::build_rgba(&self.packs, assets_root, override_pal.as_deref(), &water);
		let t = chrome.register_texture(&rgba, (PREVIEW_TILES * 64) as u32, (self.packs.len().max(1) as u32) * 64);
		self.preview_cache.insert(key, t);
		t
	}

	/// The palette-selector index of `pack`'s palette (tileset choices sit at
	/// 1..=T in pack-list order, mirroring [`crate::newmap::palette_choices`]).
	fn choice_of_pack(&self, name: &str) -> Option<usize> {
		let mut idx = 0;
		for p in &self.packs {
			if p.has_palette {
				idx += 1;
				if p.name == name {
					return Some(idx);
				}
			}
		}
		None
	}

	/// The open-time preview key with the preview toggle on: the selector
	/// starts at "from selected tileset", so the default owner's palette
	/// colours everything (0 = original colours when no owner resolves).
	fn initial_preview_key(&self) -> (usize, usize) {
		let pal = packlist::effective_owner(&self.packs, &None).and_then(|n| self.choice_of_pack(&n)).unwrap_or(0);
		(pal, self.default_water())
	}

	/// The index of the chosen water pack per `packs` state (the radio's
	/// default before the Ui exists), falling back to the first water pack.
	pub(super) fn default_water(&self) -> usize {
		self.packs
			.iter()
			.position(|p| p.water && p.selected)
			.or_else(|| self.packs.iter().position(|p| p.water))
			.unwrap_or(0)
	}

	/// The tile-set picker sections shared by New Map and Import WRL:
	/// "Select tile set" (with "Select tileset palette" captioning the radio
	/// column) - a darkened inset well scrolling 3 land-pack items, each its
	/// own inset well with a checkbox (+ palette-owner radio) above the
	/// inset-framed preview tiles - then "Select water tiles" - the same well
	/// scrolling 2 water items, each a radio (exactly one water pack fills
	/// the bottom layer) above its tiles. Fills `pack_ids` / `palette_ids`
	/// (parallel to `packs`) and `preview_img_ids`, and returns `col` with
	/// the sections appended.
	fn pack_rows(&mut self, mut col: Linear, tex: TextureId) -> Linear {
		let n = self.packs.len().max(1) as f32;
		let owner = packlist::effective_owner(&self.packs, &self.owner_choice);
		let mut pack_ids = vec![WidgetId::NONE; self.packs.len()];
		let mut palette_ids = vec![None; self.packs.len()];
		let mut img_ids = Vec::new();
		// The tile strip: each pick in its own inset frame, spread across the
		// item with equal gaps.
		let strip = |i: usize, img_ids: &mut Vec<WidgetId>| {
			let (v0, v1) = (i as f32 / n, (i + 1) as f32 / n);
			let mut row = Linear::row().main_align(MainAlign::SpaceBetween);
			for t in 0..PREVIEW_TILES {
				let (u0, u1) = (t as f32 / PREVIEW_TILES as f32, (t + 1) as f32 / PREVIEW_TILES as f32);
				let img = Image::sized(tex, STRIP_TILE, STRIP_TILE).uv(TexRect::new(u0, v0, u1, v1)).with_id();
				img_ids.push(img.id());
				row = row.push(InsetFrame::new(img));
			}
			row
		};
		// Items sit proud (outset plates) of their recessed, darkened list.
		let item_well = |head: Linear, strip: Linear| {
			Well::new(Linear::column().spacing(2.0).cross_align(CrossAlign::Stretch).push(head).push(strip))
				.padding(PICK_PAD)
				.raised()
		};
		// Keep the items off the scrollbar: a transparent gap column rides
		// along inside the scroll content.
		let gapped = |list: Linear| {
			Linear::row()
				.cross_align(CrossAlign::Stretch)
				.child(list, Length::Flex(1.0))
				.child(width_strut(LIST_GAP), Length::Fixed(LIST_GAP))
		};

		let mut land = Linear::column().spacing(PICK_SPACING).cross_align(CrossAlign::Stretch);
		for (i, p) in self.packs.iter().enumerate().filter(|(_, p)| !p.water) {
			let cb = Checkbox::new(p.title.clone()).with_checked(p.selected);
			pack_ids[i] = cb.id();
			let mut head = Linear::row().spacing(6.0).cross_align(CrossAlign::Center).child(cb, Length::Flex(1.0));
			if p.has_palette {
				let rb = Radio::new("").with_selected(owner.as_deref() == Some(p.name.as_str()));
				palette_ids[i] = Some(rb.id());
				head = head.child(rb, Length::Fixed(18.0));
			} else {
				// Keep the radio column aligned on rows without an owner toggle.
				head = head.child(width_strut(18.0), Length::Fixed(18.0));
			}
			land = land.push(item_well(head, strip(i, &mut img_ids)));
		}
		col = col.push(
			Linear::row()
				.main_align(MainAlign::SpaceBetween)
				.push(Label::new("Select tile set").small())
				.push(Label::new("Select tileset palette").small()),
		);
		col = col
			.child(Well::new(ScrollArea::new(gapped(land))).padding(LIST_PAD).shaded(77), Length::Fixed(LAND_LIST_H));

		let mut water = Linear::column().spacing(PICK_SPACING).cross_align(CrossAlign::Stretch);
		for (i, p) in self.packs.iter().enumerate().filter(|(_, p)| p.water) {
			let rb = Radio::new(p.title.clone()).with_selected(p.selected);
			pack_ids[i] = rb.id();
			let head = Linear::row().spacing(6.0).cross_align(CrossAlign::Center).child(rb, Length::Flex(1.0));
			water = water.push(item_well(head, strip(i, &mut img_ids)));
		}
		col = col.push(Label::new("Select water tiles").small());
		col = col
			.child(Well::new(ScrollArea::new(gapped(water))).padding(LIST_PAD).shaded(77), Length::Fixed(WATER_LIST_H));

		self.pack_ids = pack_ids;
		self.palette_ids = palette_ids;
		self.preview_img_ids = img_ids;
		col
	}

	/// Mirror the picker's live control states into `packs.selected` (the
	/// owner fallback and pack ordering read the pack list).
	fn sync_pack_selection(&mut self) {
		for i in 0..self.packs.len() {
			let id = self.pack_ids[i];
			if id == WidgetId::NONE {
				continue;
			}
			self.packs[i].selected = if self.packs[i].water {
				self.ui.get::<Radio>(id).is_some_and(Radio::selected)
			} else {
				self.ui.get::<Checkbox>(id).is_some_and(Checkbox::checked)
			};
		}
	}

	/// Whether any land pack's checkbox fired this dispatch (its selection
	/// feeds the owner fallback, so previews follow).
	fn pack_checkbox_fired(&self) -> bool {
		self.pack_ids
			.iter()
			.enumerate()
			.any(|(i, id)| !self.packs[i].water && *id != WidgetId::NONE && self.ui.fired(*id))
	}

	/// The current (palette choice, water pack) combination per the live Ui.
	/// With the "Palette preview" toggle off - or in the Import WRL picker -
	/// the strips keep their original pack colours (choice 0); with it on,
	/// the selector's choice colours them, falling back to the owner radio's
	/// pack palette on "from selected tileset".
	fn preview_key(&self) -> (usize, usize) {
		let water = self
			.packs
			.iter()
			.enumerate()
			.position(|(i, p)| {
				p.water
					&& self.pack_ids[i] != WidgetId::NONE
					&& self.ui.get::<Radio>(self.pack_ids[i]).is_some_and(Radio::selected)
			})
			.unwrap_or_else(|| self.default_water());
		let preview_on = self.nm_preview != WidgetId::NONE
			&& self.ui.get::<Checkbox>(self.nm_preview).is_some_and(Checkbox::checked);
		if !preview_on {
			return (0, water);
		}
		let sel = self.ui.get::<Select>(self.nm_palette_sel).map(Select::selected).unwrap_or(0);
		let pal = if sel > 0 {
			sel
		} else {
			let owner = packlist::effective_owner(&self.packs, &self.palette_owner());
			owner.and_then(|n| self.choice_of_pack(&n)).unwrap_or(0)
		};
		(pal, water)
	}

	/// Repaint the preview strips for the current choices: apply a cached
	/// atlas directly, or ask the host to compose one (the overlay has no
	/// device access at event time).
	pub(super) fn request_previews(&mut self) -> Outcome {
		self.sync_pack_selection();
		let key = self.preview_key();
		if let Some(&tex) = self.preview_cache.get(&key) {
			self.apply_preview_tex(tex);
			return Outcome::Idle;
		}
		self.preview_want = Some(key);
		Outcome::NewMapPreview {
			palette: self.nm_palettes.get(key.0).and_then(|c| c.path.clone()),
			water: self.packs.get(key.1).map(|p| p.name.clone()).unwrap_or_default(),
			key,
		}
	}

	/// The scanned pack list of the open picker - the host composes preview
	/// atlases from it ([`Outcome::NewMapPreview`]).
	pub fn pack_entries(&self) -> &[PackEntry] {
		&self.packs
	}

	/// Takes a "Palette preview" toggle the user made, if any - the shell
	/// mirrors it into the persisted `[Preferences]`.
	pub fn take_palette_preview_change(&mut self) -> Option<bool> {
		self.nm_preview_changed.take()
	}

	/// Receives the atlas the host composed for `key` (see
	/// [`Outcome::NewMapPreview`]): caches it and, when it is still the wanted
	/// combination, swaps it into the strips.
	pub fn provide_preview_tex(&mut self, key: (usize, usize), tex: TextureId) {
		self.preview_cache.insert(key, tex);
		if self.preview_want == Some(key) {
			self.preview_want = None;
			self.apply_preview_tex(tex);
		}
	}

	fn apply_preview_tex(&mut self, tex: TextureId) {
		for id in self.preview_img_ids.clone() {
			if id != WidgetId::NONE {
				if let Some(img) = self.ui.get_mut::<Image>(id) {
					img.set_tex(tex);
				}
			}
		}
	}

	/// Opens the Import WRL dialog at its pack-picker stage: the WRL's header
	/// line, a checkbox per installed pack with a palette-owner radio (the same
	/// rows as New Map), and Cancel / Import. The heavy match runs on Import
	/// (`Outcome::WrlMatch`); if tiles found no home the shell switches this
	/// dialog to the review stage via [`show_wrl_unmapped`](Self::show_wrl_unmapped).
	pub fn open_import_wrl(
		&mut self,
		chrome: &mut MenuChrome,
		packs: Vec<PackEntry>,
		assets_root: &std::path::Path,
		name: &str,
		info: (u16, u16, u16),
	) {
		self.packs = packs;
		self.owner_choice = None;
		// No palette selector / preview toggle in this picker: the strips
		// keep their original pack colours (choice 0).
		self.nm_palette_sel = WidgetId::NONE;
		self.nm_preview = WidgetId::NONE;
		self.wrl_name = name.to_string();
		self.wrl_info = info;
		self.wrl_owner = String::new();
		let key = (0, self.default_water());
		let tex = self.resolve_pack_tex(chrome, assets_root, key);
		self.build_wrl_picker(tex);
		self.events.clear();
		self.visible = true;
	}

	/// (Re)builds the picker stage — on open, and when Esc steps back from the
	/// review (`self.packs` keeps the selections).
	pub(super) fn build_wrl_picker(&mut self, tex: TextureId) {
		self.wrl_unmapped = false;
		let (mw, mh, tiles) = self.wrl_info;
		let head = format!("{}.WRL - {}x{}, {} tiles", self.wrl_name, mw, mh, tiles);
		let col = column().push(width_strut(360.0)).push(Label::new(head).small());
		let mut col = self.pack_rows(col, tex);
		let error = Label::new("").small().with_id();
		let cancel = Button::new("Cancel").secondary();
		let import = Button::new("Import").primary();
		let ids = ImportWrlIds {
			cancel: cancel.id(),
			import: import.id(),
			ignore: WidgetId::NONE,
			dest_project: WidgetId::NONE,
			dest_user: WidgetId::NONE,
			error: error.id(),
		};
		col = status_slot(col, error, 360.0, 2).push(buttons(cancel, import));
		let win = self.dialog_kept("Import WRL", col, matches!(self.dialog, Dialog::ImportWrl(_)));
		self.win_id = Some(win.id());
		self.ui = Ui::new(modal(win));
		self.dialog = Dialog::ImportWrl(ids);
	}

	/// Switches the Import WRL dialog to its unmapped-review stage: the match
	/// summary, a scrolling list of the tiles that matched nothing, the
	/// destination radios, and Abort / Ignore missing / Import tiles.
	pub fn show_wrl_unmapped(&mut self, matched: usize, used: usize, rows: &[String]) {
		if !matches!(self.dialog, Dialog::ImportWrl(_)) {
			return;
		}
		self.wrl_unmapped = true;
		let (mw, mh, _) = self.wrl_info;
		let head = format!("{mw}x{mh} - {matched}/{used} tiles matched - {} unmapped", rows.len());
		let mut list = Linear::column().spacing(2.0).cross_align(CrossAlign::Stretch);
		for row in rows {
			list = list.push(Label::new(row.clone()).small().muted());
		}
		let user_label =
			if self.wrl_owner.is_empty() { "User tileset".to_string() } else { format!("-> {}", self.wrl_owner) };
		let dest_project = Radio::new("This project").with_selected(true);
		let dest_user = Radio::new(user_label);
		let cancel = Button::new("Abort").secondary();
		let ignore = Button::new("Ignore missing");
		let import = Button::new("Import tiles").primary();
		let ids = ImportWrlIds {
			cancel: cancel.id(),
			import: import.id(),
			ignore: ignore.id(),
			dest_project: dest_project.id(),
			dest_user: dest_user.id(),
			error: WidgetId::NONE,
		};
		// Cap the list well at ~7 rows; more scroll into view.
		let cap = (rows.len().min(7) as f32) * 17.0;
		let col = column()
			.push(width_strut(380.0))
			.push(Label::new(head).small())
			.child(Well::new(ScrollArea::new(list)), Length::Fixed(cap))
			.push(
				Linear::row()
					.spacing(8.0)
					.cross_align(CrossAlign::Center)
					.child(Label::new("Missing tiles").small().muted(), Length::Fixed(90.0))
					.push(dest_project)
					.push(dest_user),
			)
			.push(Linear::row().spacing(8.0).main_align(MainAlign::End).push(cancel).push(ignore).push(import));
		let win = self.dialog_kept("Import WRL", col, matches!(self.dialog, Dialog::ImportWrl(_)));
		self.win_id = Some(win.id());
		self.ui = Ui::new(modal(win));
		self.dialog = Dialog::ImportWrl(ids);
	}

	/// Read the picker's controls (land checkboxes, water radios) into the pack
	/// list and resolve the match order + explicit owner choice (shared
	/// New Map / Import WRL semantics).
	fn collect_packs(&mut self) -> (Vec<String>, Option<String>) {
		self.sync_pack_selection();
		let owner = self.palette_owner();
		self.owner_choice = owner.clone();
		(packlist::selected(&self.packs, &owner), owner)
	}

	/// The palette-owner radio group as plain ids (a pack without a palette
	/// has no radio and holds the group's slot as `WidgetId::NONE`).
	fn palette_group(&self) -> Vec<WidgetId> {
		self.palette_ids.iter().map(|id| id.unwrap_or(WidgetId::NONE)).collect()
	}

	/// Palette-owner radios are one group: when one fires, clear the others and
	/// imply that pack is checked (owning ⇒ selected). Returns whether one fired.
	fn owner_radio_fired(&mut self) -> bool {
		let group = self.palette_group();
		let Some(sel) = self.radio_group(&group) else {
			return false;
		};
		if let Some(cb) = self.ui.get_mut::<Checkbox>(self.pack_ids[sel]) {
			cb.set_checked(true);
		}
		true
	}

	/// Water radios are one group across the water list: when one fires, clear
	/// the others and mirror the choice into `packs` (exactly one water pack
	/// is selected). Returns whether one fired.
	fn water_radio_fired(&mut self) -> bool {
		let fired = (0..self.packs.len())
			.find(|&i| self.packs[i].water && self.pack_ids[i] != WidgetId::NONE && self.ui.fired(self.pack_ids[i]));
		let Some(sel) = fired else {
			return false;
		};
		for i in 0..self.packs.len() {
			if self.packs[i].water {
				let on = i == sel;
				self.packs[i].selected = on;
				if let Some(rb) = self.ui.get_mut::<Radio>(self.pack_ids[i]) {
					rb.set_selected(on);
				}
			}
		}
		true
	}

	/// The New Map fields collected, or the inline error to show - a bad W/H
	/// refuses with the reason rather than silently coercing.
	pub(super) fn collect_newmap(&mut self, ids: &NewMapIds) -> Result<NewMapValues, String> {
		let width = parse_dim(&self.text(ids.width)).map_err(|e| format!("width {e}"))?;
		let height = parse_dim(&self.text(ids.height)).map_err(|e| format!("height {e}"))?;
		let (packs, _owner) = self.collect_packs();
		// The selector's non-default choice loads over the created map's palette.
		let pal = self.ui.get::<Select>(ids.palette).map(Select::selected).unwrap_or(0);
		let palette = (pal > 0).then(|| self.nm_palettes.get(pal).and_then(|c| c.path.clone())).flatten();
		Ok(NewMapValues { width, height, packs, palette })
	}

	/// The pack the user chose to own the palette (its radio selected), or `None`
	/// to let [`packlist::selected`] fall back to the first palette-capable pack.
	fn palette_owner(&self) -> Option<String> {
		for (i, id) in self.palette_ids.iter().enumerate() {
			if let Some(w) = id {
				if self.ui.get::<Radio>(*w).is_some_and(Radio::selected) {
					return Some(self.packs[i].name.clone());
				}
			}
		}
		None
	}

	/// One frame of the dialog's control dispatch (the render match arm).
	pub(super) fn dispatch_new_map(&mut self, ids: NewMapIds) -> Outcome {
		let mut outcome = Outcome::Idle;
		// A preset selection fills the W/H fields (Custom leaves them).
		if self.ui.fired(ids.preset) {
			let i = self.ui.get::<Select>(ids.preset).map(Select::selected).unwrap_or(0);
			if let Some((_, w, h)) = SIZE_PRESETS.get(i) {
				self.set_text(ids.width, &w.to_string());
				self.set_text(ids.height, &h.to_string());
			}
		}
		if self.ui.fired(ids.palette) {
			if self.ui.get::<Select>(ids.palette).map(Select::selected).unwrap_or(0) != 0 {
				// A custom palette overrides the owner's: the radio choice
				// no longer applies, so clear it (a radio click snaps back).
				let group = self.palette_group();
				self.radio_select(&group, None);
				self.owner_choice = None;
			} else {
				// Back to "from selected tileset": show the palette that
				// will apply - the first selected tileset's radio.
				self.sync_pack_selection();
				let owner = packlist::effective_owner(&self.packs, &self.owner_choice);
				let at = owner.and_then(|n| self.packs.iter().position(|p| p.name == n));
				let group = self.palette_group();
				self.radio_select(&group, at);
			}
			outcome = self.request_previews();
		}
		if self.owner_radio_fired() {
			// An owner pick means "palette from the selected tileset".
			if let Some(sel) = self.ui.get_mut::<Select>(ids.palette) {
				sel.set_selected(0);
			}
			outcome = self.request_previews();
		}
		if self.water_radio_fired() {
			// The water underlay in the previews follows the choice.
			outcome = self.request_previews();
		}
		if self.ui.fired(ids.preview) {
			// The preview toggle flipped: recolour, and hand the new
			// value to the shell (persisted as a preference).
			self.nm_preview_changed = Some(self.ui.get::<Checkbox>(ids.preview).is_some_and(Checkbox::checked));
			outcome = self.request_previews();
		}
		if self.pack_checkbox_fired() {
			// A selection change can move the owner fallback: recolour
			// (cached keys apply directly).
			outcome = self.request_previews();
		}
		if self.ui.fired(ids.cancel) {
			self.hide();
		} else if self.ui.fired(ids.create) {
			// A bad W/H refuses inline rather than silently coercing -
			// the house pattern (convert-palette, worldgen).
			match self.collect_newmap(&ids) {
				Ok(v) => {
					outcome = match self.newmap_shape.take() {
						Some(image) => Outcome::CreateShapedMap {
							width: v.width,
							height: v.height,
							packs: v.packs,
							palette: v.palette,
							image,
						},
						None => Outcome::CreateMap(v),
					};
					self.hide();
				}
				Err(msg) => self.set_label(ids.status, &msg),
			}
		}
		outcome
	}

	/// One frame of the dialog's control dispatch (the render match arm).
	pub(super) fn dispatch_import_wrl(&mut self, ids: ImportWrlIds) -> Outcome {
		let mut outcome = Outcome::Idle;
		if self.wrl_unmapped {
			// Destination radios are one group of two.
			self.radio_group(&[ids.dest_project, ids.dest_user]);
			if self.ui.fired(ids.cancel) {
				outcome = Outcome::WrlCancel;
				self.hide();
			} else if self.ui.fired(ids.ignore) {
				outcome = Outcome::WrlFinish { dest: map_core::ExtrasDest::Ignore };
				self.hide();
			} else if self.ui.fired(ids.import) {
				let user = self.ui.get::<Radio>(ids.dest_user).is_some_and(Radio::selected);
				let dest = if user { map_core::ExtrasDest::UserTileset } else { map_core::ExtrasDest::ProjectPack };
				outcome = Outcome::WrlFinish { dest };
				self.hide();
			}
		} else {
			self.owner_radio_fired();
			if self.water_radio_fired() {
				outcome = self.request_previews();
			}
			if self.ui.fired(ids.cancel) {
				outcome = Outcome::WrlCancel;
				self.hide();
			} else if self.ui.fired(ids.import) {
				let (packs, owner_choice) = self.collect_packs();
				if packlist::has_palette_owner(&self.packs) {
					let owner = packlist::effective_owner(&self.packs, &owner_choice).unwrap_or_default();
					self.wrl_owner = owner.clone();
					outcome = Outcome::WrlMatch { packs, owner };
				} else {
					self.set_label(ids.error, "select at least one palette-owning tileset (e.g. GREEN)");
				}
			}
		}
		outcome
	}
}
