//! Scenery authoring: the editor-state half of the cut-out pipeline - the
//! New/Clone/Edit paint runs, id allocation, PNG import/export, commit into
//! the per-pack user library, delete/rename, and the library reload that
//! follows every mutation. Split from `state.rs` (audit 2026-08-07); a child
//! module, so it reads the parent's private fields directly.

use super::*;

impl EditorState {
	/// Where the user's own cut-outs live: `resources/user/scenery/<PACK>/`.
	/// [`map_core::user_scenery_root`] derives it from the tile-pack root, so
	/// the editor and the loader can never disagree about the path.
	fn user_scenery_root(&self) -> PathBuf {
		map_core::user_scenery_root(&self.assets_root).unwrap_or_else(|| self.resources_root.join("user"))
	}

	/// Where the shipped cut-outs live: `resources/assets/scenery/<PACK>/`.
	fn shipped_scenery_root(&self) -> PathBuf {
		map_core::scenery_root(&self.assets_root).unwrap_or_else(|| self.resources_root.join("assets"))
	}

	/// The packs a cut-out may be filed under: the map's own tilesets first (so
	/// the prefilled choice is the one it will show up in), then every other
	/// installed tileset.
	///
	/// Wider than [`Self::authoring_pack_names`] on purpose - scenery is loose
	/// art, not a tile in a map's own tileset, and authoring a SNOW rock while a
	/// GREEN map happens to be open is a reasonable thing to want. A piece filed
	/// under a pack this map does not use simply does not appear in the panel
	/// until a map that uses it is opened, which [`Self::add_scenery_piece`]
	/// says out loud.
	pub fn scenery_pack_names(&self) -> Vec<String> {
		let mut out = self.authoring_pack_names();
		for entry in crate::packlist::scan(&self.assets_root) {
			if !entry.water && !out.contains(&entry.name) {
				out.push(entry.name);
			}
		}
		out
	}

	/// Open New Scenery with nothing loaded - the dialog's Import PNG key is
	/// the next step. Carries the destination packs and the ground tone each
	/// one paints with, so the preview can show a cut-out over the ground it
	/// will actually land on.
	pub(super) fn open_scenery_new(&mut self) -> Outcome {
		self.open_scenery_run(crate::scenerypaint::Mode::New, None)
	}

	/// Open the authoring dialog on the armed piece - a Clone (a new id, any
	/// pack) or an Edit (in place). A shipped piece is read-only: it clones, and
	/// only `--dev` edits it.
	pub(super) fn open_scenery_from_armed(&mut self, mode: crate::scenerypaint::Mode) -> Outcome {
		let verb = if mode.in_place() { "scenery-edit" } else { "scenery-clone" };
		let Some(i) = self.active_scenery else {
			return Outcome::Failed(format!("{verb}: arm a piece in the Scenery panel first"));
		};
		let Some((pack, piece)) = crate::scenery::piece_at(&self.project, i) else {
			return Outcome::Failed(format!("{verb}: the armed piece is gone"));
		};
		if mode.in_place() && !piece.user && !self.dev_mode {
			return Outcome::Failed("scenery-edit: shipped scenery is read-only (clone it instead)".into());
		}
		let (pack, piece) = (pack.to_string(), piece.clone());
		self.open_scenery_run(mode, Some((pack, piece)))
	}

	/// The one open path: build the run, then ask the shell for the dialog.
	pub(super) fn open_scenery_run(
		&mut self,
		mode: crate::scenerypaint::Mode,
		source: Option<(String, map_core::SceneryPiece)>,
	) -> Outcome {
		let packs = self.scenery_pack_names();
		if packs.is_empty() {
			return Outcome::Failed("scenery: no editable pack loaded".into());
		}
		let grounds = packs.iter().map(|name| self.pack_ground_tone(name)).collect();
		// A Clone or an Edit opens on its source's pack; a New piece on the map's
		// first, which is what `scenery_pack_names` puts at the front.
		let pack_sel = source.as_ref().and_then(|(p, _)| packs.iter().position(|n| n == p)).unwrap_or(0);
		let (piece, from, name_text, id_text) = match source {
			// A clone needs an id of its own: two pieces cannot answer to one
			// name, and silently replacing the original is the one thing a clone
			// must never do.
			Some((pack, piece)) => {
				let id = if mode.in_place() { piece.id.clone() } else { self.fresh_scenery_id(&pack, &piece.id) };
				let name = if mode.in_place() { piece.name.clone() } else { format!("{} Copy", piece.name) };
				let from = (pack, piece.id.clone(), piece.user);
				(Some(piece), Some(from), name, id)
			}
			None => (None, None, String::new(), String::new()),
		};
		self.scenerypaint = Some(crate::scenerypaint::SceneryPaintRun {
			mode,
			packs,
			grounds,
			pack_sel,
			src: Vec::new(),
			src_w: 0,
			src_h: 0,
			piece,
			from,
			name_text,
			id_text,
			rev: 0,
			hgt_src: Vec::new(),
			hgt_w: 0,
			hgt_h: 0,
			hgt_rev: 0,
			hgt_out: Vec::new(),
			hgt_out_w: 0,
			hgt_out_h: 0,
		});
		Outcome::OpenDialog(DialogRequest::SceneryNew)
	}

	/// An id `base` does not already own in `pack`: `<base>-copy`, then
	/// `-copy-2` and up.
	///
	/// The source id is kept whole - `mountain-1-copy`, not `mountain-copy` -
	/// so a clone still says what it came from, and two clones out of one family
	/// cannot land on the same suggestion. Checked against the **merged**
	/// library, so it never shadows a shipped piece by accident either.
	fn fresh_scenery_id(&self, pack: &str, base: &str) -> String {
		let taken = |id: &str| {
			self.project
				.scenery_packs
				.iter()
				.filter(|lib| lib.pack == pack)
				.any(|lib| lib.pieces.iter().any(|p| p.id == id))
		};
		let first = format!("{base}-copy");
		if !taken(&first) {
			return first;
		}
		(2u32..).map(|n| format!("{base}-copy-{n}")).find(|id| !taken(id)).unwrap_or(first)
	}

	/// The mean tone a pack's plain ground is painted with - the preview
	/// backdrop. Mid-grey for a pack with no plain-ground family (or none
	/// loaded), which is still a fair neutral to judge a shadow against.
	fn pack_ground_tone(&self, pack: &str) -> [u8; 3] {
		let Some(tp) = self.project.packs.iter().find(|p| p.name == pack && !p.user) else { return [96, 96, 96] };
		let ground = map_core::GroundInk::of_pack(tp);
		if ground.is_empty() {
			return [96, 96, 96];
		}
		let mean = ground.mean(&self.project.palette);
		[mean[0] as u8, mean[1] as u8, mean[2] as u8]
	}

	/// Rasterize the open run's source image at `opts` (or the dialog's
	/// defaults). `None` when nothing has been imported, or nothing survived
	/// the thresholds.
	pub(super) fn rasterize_scenery_run(
		&self,
		opts: Option<map_core::RasterOpts>,
	) -> Option<(map_core::Sprite, Vec<u8>, u16, u16)> {
		let run = self.scenerypaint.as_ref()?;
		map_core::rasterize(
			&run.src,
			run.src_w as usize,
			run.src_h as usize,
			&self.project.palette,
			&opts.unwrap_or_default(),
		)
	}

	/// Bring a cut-out in from `path`. A `.scn` is a finished piece and is
	/// filed straight into the destination library; anything else is treated as
	/// an image and opens (or reloads) New Scenery, because an image still has
	/// thresholds and colours to settle before it is a piece.
	pub(super) fn scenery_import(&mut self, path: &Path) -> Outcome {
		let is_scn = path.extension().is_some_and(|e| e.eq_ignore_ascii_case(map_core::SCN_EXT));
		if is_scn {
			let bytes = match std::fs::read(path) {
				Ok(b) => b,
				Err(e) => return Outcome::Failed(format!("scenery-import: {e}")),
			};
			let (piece, hint) = match map_core::read_scn(&bytes) {
				Ok(v) => v,
				Err(e) => return Outcome::Failed(format!("scenery-import: {e}")),
			};
			// The file's own pack is a hint, not an instruction: it may name a
			// tileset this map never loaded. Prefer it when the map has it.
			let packs = self.scenery_pack_names();
			let pack = packs.iter().find(|p| **p == hint).cloned().or_else(|| packs.first().cloned());
			let Some(pack) = pack else {
				return Outcome::Failed("scenery-import: no editable pack loaded".into());
			};
			return self.add_scenery_piece(&pack, piece);
		}
		let (rgba, w, h) = match decode_png_rgba(path) {
			Ok(v) => v,
			Err(e) => return Outcome::Failed(format!("scenery-import: {e}")),
		};
		if w == 0 || h == 0 {
			return Outcome::Failed("scenery-import: empty image".into());
		}
		let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
		// Reloading an open dialog rather than stacking a second one: the
		// dialog re-derives when `rev` moves, exactly as the Tile Painter does.
		if let Some(run) = self.scenerypaint.as_mut() {
			Self::load_scenery_source(run, rgba, w, h, &stem);
			let line = format!("loaded {} ({w}x{h}) into {}", path.display(), run.mode.title().to_lowercase());
			eprintln!("{line}");
			self.console.push_line(line);
			return Outcome::Redraw;
		}
		let open = self.open_scenery_new();
		if let (Outcome::OpenDialog(_), Some(run)) = (&open, self.scenerypaint.as_mut()) {
			Self::load_scenery_source(run, rgba, w, h, &stem);
		}
		open
	}

	/// Point an open run at a freshly decoded image, bumping the revision the
	/// dialog re-derives on.
	///
	/// The name and the id follow the file **except in an Edit**, where the art
	/// is being replaced under a piece that already exists: its id is what
	/// placements store, and its name is the one on the panel.
	fn load_scenery_source(run: &mut crate::scenerypaint::SceneryPaintRun, rgba: Vec<u8>, w: u32, h: u32, stem: &str) {
		run.src = rgba;
		run.src_w = w;
		run.src_h = h;
		run.rev += 1;
		if !run.mode.in_place() {
			run.name_text = crate::scenerypaint::name_from_stem(stem);
			run.id_text = crate::scenerypaint::id_from_stem(stem);
		}
	}

	/// Bring a **painted height map** in from `path` for the open dialog: the
	/// picture's grey channel, handed to the Heightmap tab to fit onto the art.
	///
	/// Read as greyscale whatever the file's colour type says, because a height
	/// map has one channel by definition and a paint program will happily save
	/// the same picture as RGB. Alpha is ignored: the silhouette is the body
	/// plane's business (`map_core::height_from_grey`), not the painter's.
	pub(super) fn scenery_height_import(&mut self, path: &Path) -> Outcome {
		if self.scenerypaint.is_none() {
			return Outcome::Failed("scenery-height-import: open New/Clone/Edit Scenery first".into());
		}
		let (rgba, w, h) = match decode_png_rgba(path) {
			Ok(v) => v,
			Err(e) => return Outcome::Failed(format!("scenery-height-import: {e}")),
		};
		if w == 0 || h == 0 {
			return Outcome::Failed("scenery-height-import: empty image".into());
		}
		let run = self.scenerypaint.as_mut().expect("checked above");
		run.hgt_src = rgba.chunks_exact(4).map(|px| px[0]).collect();
		run.hgt_w = w;
		run.hgt_h = h;
		run.hgt_rev += 1;
		let line = format!("loaded height map {} ({w}x{h})", path.display());
		eprintln!("{line}");
		self.console.push_line(line);
		Outcome::Redraw
	}

	/// Fit the imported height map (if there is one) onto `sprite`, at the peak
	/// `relief` names or the one the sprite falls back to.
	///
	/// The one place the picture becomes elevations, so the dialog's Heightmap
	/// tab and a script's `scenery-height-import` cannot read the same PNG two
	/// different ways. `None` when nothing was imported, or when what was does
	/// not line up with the art (`map_core::height_from_grey`).
	pub(super) fn fit_scenery_height(
		&self,
		sprite: &map_core::Sprite,
		(cells_w, cells_h): (u16, u16),
		relief: Option<(u8, bool)>,
	) -> Option<Vec<u8>> {
		let run = self.scenerypaint.as_ref()?;
		if run.hgt_src.is_empty() {
			return None;
		}
		let peak = relief.map(|(peak, _)| peak).unwrap_or_else(|| sprite.default_peak());
		map_core::height_from_grey(&run.hgt_src, run.hgt_w as usize, run.hgt_h as usize, sprite, cells_w, cells_h, peak)
	}

	/// Write the picture the dialog handed over ([`SceneryPaintRun::hgt_out`])
	/// to `path`, as the 8-bit greyscale PNG
	/// [`scenery_height_import`](Self::scenery_height_import) reads back.
	pub(super) fn scenery_height_export(&mut self, path: &Path) -> Outcome {
		let Some(run) = self.scenerypaint.as_ref() else {
			return Outcome::Failed("scenery-height-export: open New/Clone/Edit Scenery first".into());
		};
		let (w, h) = (run.hgt_out_w, run.hgt_out_h);
		if run.hgt_out.len() != w as usize * h as usize || w == 0 || h == 0 {
			return Outcome::Failed("scenery-height-export: no height map to write - import art first".into());
		}
		if let Err(e) = write_grey_png(path, &run.hgt_out, w, h) {
			return Outcome::Failed(format!("scenery-height-export: {e}"));
		}
		let line = format!("wrote height map {} ({w}x{h})", path.display());
		eprintln!("{line}");
		self.console.push_line(line);
		Outcome::Redraw
	}

	/// Write the armed piece out as a shareable `.scn`.
	pub(super) fn scenery_export(&mut self, path: &Path) -> Outcome {
		let Some(i) = self.active_scenery else {
			return Outcome::Failed("scenery-export: arm a piece in the Scenery panel first".into());
		};
		let Some((pack, piece)) = crate::scenery::piece_at(&self.project, i) else {
			return Outcome::Failed("scenery-export: the armed piece is gone".into());
		};
		let bytes = map_core::write_scn(piece, pack);
		match std::fs::write(path, &bytes) {
			Ok(()) => {
				let line = format!("exported scenery {} to {}", piece.id, path.display());
				eprintln!("{line}");
				self.console.push_line(line);
				Outcome::Redraw
			}
			Err(e) => Outcome::Failed(format!("scenery-export: {e}")),
		}
	}

	/// Build a piece out of the authoring dialog's values and file it.
	///
	/// An **Edit** writes back over its source - same pack, same id, same root
	/// (so `--dev` really does rewrite a shipped cut-out). A New or a Clone
	/// files a user piece, and may not take a **shipped** id: shadowing one is
	/// how you would edit shipped art without `--dev`, and the whole point of
	/// the clone key is that you make your own instead.
	pub fn scenery_commit(
		&mut self,
		pack: String,
		id: String,
		name: String,
		sprite: map_core::Sprite,
		pass: Vec<u8>,
		(cells_w, cells_h): (u16, u16),
		// The `Stands:` choice as `(peak, sunken)`, or `None` to leave the relief
		// inferred from the art - which is what every shipped piece does.
		relief: Option<(u8, bool)>,
		// The Heightmap tab's drawn relief, in the sprite's frame, or `None` to
		// infer the whole field. A plane that does not fit the art is dropped
		// rather than filed: the art is what a placement draws, so it wins.
		height: Option<Vec<u8>>,
	) -> Outcome {
		let (id, name) = (id.trim().to_string(), name.trim().to_string());
		if id.is_empty() {
			return Outcome::Failed("scenery: the id is empty".into());
		}
		if !id.chars().all(crate::scenerypaint::is_id_char) {
			return Outcome::Failed("scenery: id: only letters, digits, - and _".into());
		}
		if sprite.is_empty() {
			return Outcome::Failed("scenery: nothing survived the thresholds".into());
		}
		let source = self.scenerypaint.as_ref().and_then(|r| r.from.clone().map(|f| (r.mode, f)));
		let (pack, id, shipped) = match source {
			Some((mode, (from_pack, from_id, user))) if mode.in_place() => (from_pack, from_id, !user),
			_ => (pack, id, false),
		};
		if !shipped && !self.dev_mode && self.shipped_scenery_has(&pack, &id) {
			return Outcome::Failed(format!("scenery: '{id}' is a shipped piece in {pack} - give the copy its own id"));
		}
		let name = if name.is_empty() { id.clone() } else { name };
		let texels = sprite.width as usize * sprite.height as usize;
		let piece = map_core::SceneryPiece {
			family: map_core::piece_family(&name),
			transformable: Default::default(),
			peak: relief.map(|(peak, _)| peak),
			sunken: relief.map(|(_, sunken)| sunken),
			// Carried off the piece this run opened on, not chosen in the dialog:
			// a wall is marked in the pack's `tune.json` and baked in, and there
			// is no control for it. What matters here is that a Clone or an Edit
			// does not quietly file a cliff back as a ridge.
			scarp: self.scenerypaint.as_ref().and_then(|r| r.piece.as_ref()).and_then(|p| p.scarp),
			height: height.filter(|h| h.len() == texels),
			name,
			id,
			cells_w,
			cells_h,
			pass,
			sprite,
			user: !shipped,
		};
		self.add_scenery_piece(&pack, piece)
	}

	/// Whether the shipped bake already holds `id` in `pack`.
	fn shipped_scenery_has(&self, pack: &str, id: &str) -> bool {
		map_core::SceneryPack::load(&self.shipped_scenery_root(), pack)
			.is_ok_and(|lib| lib.pieces.iter().any(|p| p.id == id))
	}

	/// File `piece` into `pack`'s library, persist it, and re-list the panel. A
	/// repeated id **replaces** - the only other option is to refuse, and
	/// re-importing a piece you just tweaked is the common case.
	///
	/// The user root, unless the piece itself says it is shipped art - which
	/// only an `--dev` Edit can produce.
	fn add_scenery_piece(&mut self, pack: &str, piece: map_core::SceneryPiece) -> Outcome {
		let root = if piece.user { self.user_scenery_root() } else { self.shipped_scenery_root() };
		let mut lib = map_core::SceneryPack::load(&root, pack)
			.unwrap_or_else(|_| map_core::SceneryPack { pack: pack.to_string(), pieces: Vec::new() });
		let (id, name) = (piece.id.clone(), piece.name.clone());
		let replaced = match lib.pieces.iter().position(|p| p.id == id) {
			Some(i) => {
				lib.pieces[i] = piece;
				true
			}
			None => {
				lib.pieces.push(piece);
				false
			}
		};
		if let Err(e) = lib.save(&root) {
			return Outcome::Failed(format!("scenery: {e}"));
		}
		self.reload_scenery_libraries();
		// Arm what was just made: the next click should place it.
		self.active_scenery = crate::scenery::index_of(&self.project, pack, &id);
		let verb = if replaced { "replaced" } else { "added" };
		// Filing under a pack this map does not use is allowed, but the panel
		// lists only the open map's libraries - so say where it went.
		let unused = if self.project.uses.iter().any(|u| u.name == pack) {
			String::new()
		} else {
			format!(" (this map does not use {pack} - open one that does to place it)")
		};
		let line = format!("{verb} scenery {id} (\"{name}\") in {pack}{unused}");
		eprintln!("{line}");
		self.console.push_line(line);
		self.scenerypaint = None;
		Outcome::DocReplaced
	}

	/// Re-read every library the open project uses, so the panel, the atlas and
	/// the placements all see the same set after a write.
	fn reload_scenery_libraries(&mut self) {
		let (shipped, user) = (self.shipped_scenery_root(), self.user_scenery_root());
		let armed = self.armed_scenery();
		self.project.scenery_packs = self
			.project
			.uses
			.iter()
			.filter_map(|u| map_core::SceneryPack::load_merged(&shipped, &user, &u.name))
			.collect();
		// The flat index is a position, so it moves when the set does; re-find
		// the armed piece by name rather than trusting the old number.
		self.active_scenery = armed.and_then(|(pack, id)| crate::scenery::index_of(&self.project, &pack, &id));
	}

	/// The armed piece, with whether it is the user's to change.
	fn armed_scenery_piece(&self) -> Option<(String, String, bool)> {
		let i = self.active_scenery?;
		let (pack, piece) = crate::scenery::piece_at(&self.project, i)?;
		Some((pack.to_string(), piece.id.clone(), piece.user))
	}

	/// Delete the armed piece. Without `force` this only asks; the confirmation
	/// fires `scenery-delete!`.
	pub(super) fn scenery_delete(&mut self, force: bool) -> Outcome {
		let Some((pack, id, user)) = self.armed_scenery_piece() else {
			return Outcome::Failed("scenery-delete: arm a piece in the Scenery panel first".into());
		};
		if !user && !self.dev_mode {
			return Outcome::Failed("scenery-delete: shipped scenery is read-only (needs --dev)".into());
		}
		if !force {
			let placed = self.project.scenery.iter().filter(|s| s.pack == pack && s.piece == id).count();
			let name = crate::scenery::piece_at(&self.project, self.active_scenery.unwrap_or(0))
				.map_or_else(|| id.clone(), |(_, p)| p.name.clone());
			return Outcome::OpenDialog(DialogRequest::DeleteScenery { pack, id, name, placed });
		}
		// A shipped piece can only be removed from the shipped bake (--dev);
		// the user's own from the user root.
		let root = if user { self.user_scenery_root() } else { self.shipped_scenery_root() };
		let Ok(mut lib) = map_core::SceneryPack::load(&root, &pack) else {
			return Outcome::Failed(format!("scenery-delete: no library to edit for '{pack}'"));
		};
		let before = lib.pieces.len();
		lib.pieces.retain(|p| p.id != id);
		if lib.pieces.len() == before {
			return Outcome::Failed(format!("scenery-delete: '{id}' is not in {pack}'s editable library"));
		}
		if let Err(e) = lib.save(&root) {
			return Outcome::Failed(format!("scenery-delete: {e}"));
		}
		self.active_scenery = None;
		self.reload_scenery_libraries();
		let line = format!("deleted scenery {id} from {pack}");
		eprintln!("{line}");
		self.console.push_line(line);
		Outcome::DocReplaced
	}

	/// Rename the armed piece's **display name**. Its id never moves: that is
	/// what a placement stores, and renaming it would orphan every object
	/// already on a map.
	pub(super) fn scenery_rename(&mut self, name: Option<String>) -> Outcome {
		let Some((pack, id, user)) = self.armed_scenery_piece() else {
			return Outcome::Failed("scenery-rename: arm a piece in the Scenery panel first".into());
		};
		if !user && !self.dev_mode {
			return Outcome::Failed("scenery-rename: shipped scenery is read-only (needs --dev)".into());
		}
		let Some(name) = name else {
			let from = crate::scenery::piece_at(&self.project, self.active_scenery.unwrap_or(0))
				.map_or_else(String::new, |(_, p)| p.name.clone());
			return Outcome::OpenDialog(DialogRequest::RenameScenery { pack, id, from });
		};
		let name = name.trim().to_string();
		if name.is_empty() {
			return Outcome::Failed("scenery-rename: the name is empty".into());
		}
		let root = if user { self.user_scenery_root() } else { self.shipped_scenery_root() };
		let Ok(mut lib) = map_core::SceneryPack::load(&root, &pack) else {
			return Outcome::Failed(format!("scenery-rename: no library to edit for '{pack}'"));
		};
		let Some(piece) = lib.pieces.iter_mut().find(|p| p.id == id) else {
			return Outcome::Failed(format!("scenery-rename: '{id}' is not in {pack}'s editable library"));
		};
		piece.name = name.clone();
		if let Err(e) = lib.save(&root) {
			return Outcome::Failed(format!("scenery-rename: {e}"));
		}
		self.reload_scenery_libraries();
		let line = format!("renamed scenery {id} to \"{name}\"");
		eprintln!("{line}");
		self.console.push_line(line);
		Outcome::DocReplaced
	}
}
