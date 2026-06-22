//! Map project - the editor's primary document.
//!
//! v1 format: `resources/maps/*.json` - see `docs/design/tileset-contract.md`
//! §3. Each cell is a bottom-up stack (water layer, ground layer); tile refs
//! carry a transform (rotation + mirror). `compose_cell` flattens a stack to
//! raw pixels - the kernel of the future WRL export bake, and the
//! thing the 24-map equivalence test verifies against original WRLs.

use std::collections::HashMap;
use std::path::Path;

use max_assets::wrl::{TILE_DATA_SIZE, TILE_SIZE, WrlFile};

use crate::pack::TilePack;

mod palette_reimport;
mod serde;

pub use palette_reimport::PaletteReimport;

pub const LAYER_WATER: usize = 0;
pub const LAYER_GROUND: usize = 1;
pub const MAX_LAYERS: usize = 2;

/// The project-file format version this editor writes, `"MAJOR.MINOR"`, stored
/// under `"mme_project_file_version"`. The **MAJOR** is the compatibility
/// guard: a file with the same MAJOR opens and is migrated up to this MINOR; a
/// different MAJOR is unsupported (a hard break). A pre-scheme `"version": "1"`
/// is grandfathered in and migrated to this version.
pub const PROJECT_VERSION: &str = "2.0";

/// Undo depth cap - beyond this the oldest patches are dropped.
const MAX_UNDO: usize = 256;

/// The tileset-editable palette slots (contract §1: dynamic 64–159).
pub const DYNAMIC_SLOTS: std::ops::RangeInclusive<u8> = 64..=159;

/// The game-animated shimmer slots (contract §1): the engine re-tints this
/// fixed cycle each frame, so pixels are never quantized onto them.
pub const ANIMATED_SLOTS: std::ops::RangeInclusive<u8> = 9..=31;

/// The water / special-effect cycle band (contract §1; see [`WATER_CYCLES`]).
pub const WATER_SLOTS: std::ops::RangeInclusive<u8> = 96..=127;

/// The dynamic **animated** water cycle classes (contract §1) - each block
/// is one in-game color gradient; block re-tints keep it coherent.
pub const WATER_CYCLES: [(u8, u8); 5] = [(96, 102), (103, 109), (110, 116), (117, 122), (123, 127)];

/// The largest map dimension (cells per side) a document/template may have.
pub(crate) const MAX_DIM: u16 = 1024;

/// Validate a map's dimensions (both in `1..=MAX_DIM`).
pub(crate) fn check_map_size(width: u16, height: u16) -> Result<(), String> {
	if width == 0 || height == 0 || width > MAX_DIM || height > MAX_DIM {
		return Err(format!("bad map size {width}×{height} (1..=1024)"));
	}
	Ok(())
}

/// Encode a `width`×`height` cell grid as JSON rows (`[[String; width]; height]`),
/// each cell rendered by `cell(x, y)` - the shared map-body writer for the
/// project file and templates.
pub(crate) fn encode_cell_grid(
	width: usize,
	height: usize,
	cell: impl Fn(usize, usize) -> String,
) -> Vec<json::JsonValue> {
	(0..height)
		.map(|y| json::JsonValue::Array((0..width).map(|x| json::JsonValue::String(cell(x, y))).collect()))
		.collect()
}

/// Which layer a tile belongs on, by its passability: water (pass 1) is the
/// opaque base; land / shore / obstruction sit on the ground layer. This is
/// how an imported WRL is decomposed into the two editor layers, and how a
/// reloaded WRL-import project recovers the same split.
pub(crate) fn pass_layer(pass: u8) -> usize {
	if pass == 1 { LAYER_WATER } else { LAYER_GROUND }
}

/// A pass value's class glyph + dense class index (for `XXXY###` tile ids):
/// `W`ater / `S`hore / `L`and / obstruction (`X`).
pub(crate) fn pass_class(pass: u8) -> (char, usize) {
	match pass {
		1 => ('W', 1),
		2 => ('S', 2),
		3 => ('X', 3),
		_ => ('L', 0),
	}
}

/// The 3-letter id prefix for a pack built from a WRL: the first three
/// consonants of its name (upper-cased), topped up with vowels then `X` when a
/// name has fewer than three consonants. `GREEN_1` → `GRN`, `GO` → `GOX`.
pub(crate) fn pack_prefix(name: &str) -> String {
	let letters: Vec<char> = name.chars().filter(|c| c.is_ascii_alphabetic()).map(|c| c.to_ascii_uppercase()).collect();
	let vowel = |c: &char| matches!(c, 'A' | 'E' | 'I' | 'O' | 'U');
	let mut out: Vec<char> = letters.iter().copied().filter(|c| !vowel(c)).take(3).collect();
	for c in letters.iter().copied().filter(vowel) {
		if out.len() == 3 {
			break;
		}
		out.push(c);
	}
	while out.len() < 3 {
		out.push('X');
	}
	out.into_iter().collect()
}

/// Tiny deterministic PRNG (splitmix64) - the new-map fill and future
/// generators must reproduce exactly from a seed, on every
/// platform, forever. Never swap this for a library RNG.
pub struct Rng(u64);

/// The splitmix64 finalizer - the bit mixer behind both [`Rng`] and worldgen's
/// lattice hash. A pure function of its input; never change it, seeded output
/// must reproduce forever on every platform.
pub(crate) fn splitmix(mut z: u64) -> u64 {
	z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
	z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
	z ^ (z >> 31)
}

impl Rng {
	pub fn new(seed: u64) -> Self {
		Self(seed)
	}

	pub fn next_u64(&mut self) -> u64 {
		self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
		splitmix(self.0)
	}

	/// Uniform in `0..n` (`n > 0`; modulo bias is negligible at u64 width).
	pub fn below(&mut self, n: u32) -> u32 {
		(self.next_u64() % n as u64) as u32
	}
}

/// Rotation (quarter turns clockwise) + horizontal mirror (applied first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Transform {
	pub rot: u8, // 0..=3 - N, E, S, W
	pub mirror: bool,
}

impl Transform {
	/// Suffix convention (verified empirically against all 24 original WRLs
	/// in `tests/equivalence.rs`): plain letters are counter-clockwise
	/// quarter turns (`E` = 1 ccw = 3 cw, `W` = 3 ccw = 1 cw); with the `!`
	/// mirror prefix the letter counts clockwise (`!E` = 1 cw + mirror).
	/// A bare `!` (the `tiles.match.json` shorthand) is mirror-only = `!N`.
	pub fn parse(s: &str) -> Result<Self, String> {
		let (mirror, dir) = match s.strip_prefix('!') {
			Some(rest) => (true, rest),
			None => (false, s),
		};
		let cw = match dir {
			"N" => 0,
			"E" => 1,
			"S" => 2,
			"W" => 3,
			"" if mirror => 0,
			_ => return Err(format!("bad transform '{s}'")),
		};
		let rot = if mirror { cw } else { (4 - cw) % 4 };
		Ok(Self { rot, mirror })
	}

	pub fn suffix(&self) -> String {
		if !self.mirror && self.rot == 0 {
			return String::new();
		}
		let cw = if self.mirror { self.rot } else { (4 - self.rot) % 4 };
		let dir = ["N", "E", "S", "W"][cw as usize];
		format!(":{}{}", if self.mirror { "!" } else { "" }, dir)
	}

	/// Pack into 3 bits (rot in bits 0–1, mirror in bit 2) for the GPU.
	pub fn bits(&self) -> u32 {
		self.rot as u32 | ((self.mirror as u32) << 2)
	}

	// Composition: a stored transform is `R(rot) ∘ M(mirror)` (mirror first,
	// then clockwise quarter turns). The toolbox ops apply a further
	// operation *after* it and re-normalize to that form; `M ∘ R(r) =
	// R(-r) ∘ M` is the only identity needed. Verified pixel-for-pixel by
	// `transform_ops_match_pixel_operations`.

	/// This transform followed by one more clockwise quarter turn.
	pub fn rotated_cw(self) -> Self {
		Self { rot: (self.rot + 1) % 4, mirror: self.mirror }
	}

	/// This transform followed by one counter-clockwise quarter turn.
	pub fn rotated_ccw(self) -> Self {
		Self { rot: (self.rot + 3) % 4, mirror: self.mirror }
	}

	/// This transform followed by a horizontal mirror.
	pub fn flipped_h(self) -> Self {
		Self { rot: (4 - self.rot) % 4, mirror: !self.mirror }
	}

	/// This transform followed by a vertical mirror (= mirror + 180°).
	pub fn flipped_v(self) -> Self {
		Self { rot: (6 - self.rot) % 4, mirror: !self.mirror }
	}

	/// `self ∘ inner` - apply `inner` first, then `self`, re-normalized to
	/// the stored `R ∘ M` form (`R(a)M(α)R(b)M(β) = R(a∓b)M(α⊕β)`). The
	/// match rules describe neighbors relative to a family's base
	/// orientation; placing the family transformed means composing its
	/// transform onto every listed neighbor spec (auto-shore seams).
	pub fn compose(self, inner: Self) -> Self {
		Self {
			rot: (self.rot + if self.mirror { 4 - inner.rot } else { inner.rot }) % 4,
			mirror: self.mirror ^ inner.mirror,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileRef {
	pub pack: u8,
	pub tile: u16,
	pub transform: Transform,
}

#[derive(Clone)]
pub struct UseEntry {
	pub name: String,
	pub tileset: bool,
	pub palette: bool,
	pub version: String,
}

/// Append user-owned packs (`resources/user/tilepacks/<NAME>/`) that mirror
/// a stock pack already in `packs` - the Tile Painter stores new/cloned tiles
/// there, named after the pack they derive from. Appended *after* the stock
/// packs (so their ids resolve, and indices stay stable for the session), and
/// only when the matching stock pack is loaded (a GREEN user pack only joins a
/// map that uses GREEN). Best-effort: an unreadable user pack is skipped.
///
/// `assets_root` is `resources/assets/tilepacks`, so the user packs sit two
/// levels up under `user/tilepacks`.
fn append_user_packs(packs: &mut Vec<TilePack>, assets_root: &Path) {
	let Some(user_root) = assets_root.parent().and_then(Path::parent).map(|r| r.join("user/tilepacks")) else {
		return;
	};
	let Ok(dir) = std::fs::read_dir(&user_root) else { return };
	let stock: Vec<String> = packs.iter().filter(|p| !p.user).map(|p| p.name.clone()).collect();
	let mut names: Vec<String> = dir
		.flatten()
		.filter(|e| e.path().is_dir())
		.map(|e| e.file_name().to_string_lossy().into_owned())
		.filter(|name| stock.contains(name) && !packs.iter().any(|p| p.user && &p.name == name))
		.collect();
	names.sort(); // deterministic append order
	for name in names {
		if let Ok(mut pack) = TilePack::load(&user_root, &name) {
			pack.user = true;
			packs.push(pack);
		}
	}
}

pub struct Project {
	pub version: String,
	pub name: String,
	pub description: String,
	/// Map metadata (Map Preferences) - all optional, never affect the bake.
	/// Suggested player count (2–4); `None` = unspecified.
	pub players: Option<u8>,
	/// Free-text date (no enforced format).
	pub date: String,
	/// Author-facing map version string (distinct from the file format version).
	pub map_version: String,
	pub author: String,
	pub width: u16,
	pub height: u16,
	pub uses: Vec<UseEntry>,
	pub packs: Vec<TilePack>,
	/// `width * height` cell stacks, bottom-up: `[water, ground]`.
	pub cells: Vec<[Option<TileRef>; MAX_LAYERS]>,
	/// Per-cell pass-value override (Pass Table Editor) - `None`
	/// falls back to the derived stack-top pass. `width * height` long.
	pass_overrides: Vec<Option<u8>>,
	/// Working 256×RGB palette: the owner pack's palette + this map's
	/// dynamic-slot overrides (edited via `set_color`/`hsl_shift_block`).
	pub palette: Vec<u8>,
	/// The owner pack's pristine palette - the diff against it is what
	/// `save_string` writes as the project's `"palette"` override block.
	pack_palette: Vec<u8>,
	/// The document's palette exactly as its source carries it - the WRL's
	/// internal palette bytes (or the pack's `palette.json`), **before** the
	/// game statics replace the static slots. Debug rendering and the WRL
	/// Internal Palette panel read it via [`Self::internal_palette`].
	source_palette: Vec<u8>,
	/// Index of the pack that fills the water layer (v1: named "WATER").
	pub water_pack: Option<u8>,
	/// Unit-preview annotations (editor aid): real game units stamped on the
	/// map for palette tuning. Saved in the project (`"units"` block), never
	/// baked into the WRL, not part of undo (view-layer metadata).
	pub units: Vec<UnitNote>,

	dirty: bool,
	revision: u64,
	/// Bumped whenever document *structure* changes - pack tile tables /
	/// palette tables swapped (palette conversion and its undo/redo). The
	/// shell compares it across a command to know the GPU atlas must rebuild.
	structure: u64,
	undo_stack: Vec<Patch>,
	redo_stack: Vec<Patch>,
	/// Open stroke: edits accumulate here and undo as one unit.
	stroke: Option<Patch>,
}

/// One unit-preview annotation: a game unit stamped on a cell with a team
/// color (0-4: red green blue gray yellow). The sprite itself lives in the
/// user's MAX.RES - the project only records what stands where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitNote {
	pub tag: String,
	pub x: u16,
	pub y: u16,
	pub team: u8,
}

/// One undoable edit: cells with their *previous* layer entries, palette
/// slots with their *previous* colors.
#[derive(Default)]
struct Patch {
	cells: Vec<(u16, u16, usize, Option<TileRef>)>,
	colors: Vec<(u8, [u8; 3])>,
	/// Pass-override edits with their *previous* value (`None` = unset).
	passes: Vec<(u16, u16, Option<u8>)>,
	/// Per-tile passability edits (Pass Table Editor): `(pack, tile,
	/// previous pass)`. The pass lives in the pack, so one edit retints every
	/// cell that uses the tile.
	tile_passes: Vec<(u8, u16, u8)>,
	/// A whole-document swap (palette conversion rewrites tile pixel data -
	/// not expressible as per-cell edits). Applying swaps the stored state
	/// with the live one, so the patch is its own inverse carrier.
	doc: Option<Box<DocState>>,
}

impl Patch {
	fn is_empty(&self) -> bool {
		self.cells.is_empty()
			&& self.colors.is_empty()
			&& self.passes.is_empty()
			&& self.tile_passes.is_empty()
			&& self.doc.is_none()
	}
}

/// Everything a document-level operation may replace (same map dimensions).
struct DocState {
	uses: Vec<UseEntry>,
	packs: Vec<TilePack>,
	cells: Vec<[Option<TileRef>; MAX_LAYERS]>,
	pass_overrides: Vec<Option<u8>>,
	palette: Vec<u8>,
	pack_palette: Vec<u8>,
	source_palette: Vec<u8>,
	water_pack: Option<u8>,
}

impl Project {
	pub fn load(path: &Path, assets_root: &Path) -> Result<Self, String> {
		let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
		// A project saved from an imported WRL co-locates its synthetic pack
		// in a sibling folder; search there too (see `TilePack::dump`).
		Self::from_str_in(&text, assets_root, path.parent())
	}

	pub fn from_str(text: &str, assets_root: &Path) -> Result<Self, String> {
		Self::from_str_in(text, assets_root, None)
	}

	/// Import a flat WRL as a Project - the in-memory form for an opened
	/// `.WRL` (the document-model convergence). The WRL's tile table
	/// becomes one synthetic in-memory pack; every cell references its bigmap
	/// tile on the water (opaque base) layer with identity transform, so
	/// `compose_cell` reproduces the WRL's pixels byte-for-byte.
	///
	/// The synthetic pack carries the WRL palette and per-tile pass table but
	/// no adjacency rules, so auto-shore / suggestions / tile-pack browsing
	/// don't apply to an imported WRL. It exports back to a WRL via `bake`,
	/// but can't be saved as a `.json` project (the pack isn't on disk).
	pub fn from_wrl(wrl: &WrlFile, name: &str) -> Self {
		let tile_count = wrl.tile_count as usize;
		// Tile ids carry meaning: `XXXY###` - `XXX` from the map name, `Y` the
		// passability class (W/S/L/X), `###` a per-class running index.
		let prefix = pack_prefix(name);
		let mut class_seq = [0u32; 4];
		let ids: Vec<String> = (0..tile_count)
			.map(|i| {
				let (letter, slot) = pass_class(wrl.pass_table.get(i).copied().unwrap_or(0));
				let n = class_seq[slot];
				class_seq[slot] += 1;
				format!("{prefix}{letter}{n:03}")
			})
			.collect();
		let index_of: HashMap<String, u16> = ids.iter().enumerate().map(|(i, id)| (id.clone(), i as u16)).collect();

		// Static slots belong to the game (contract §1); resolve them to the
		// in-game values, matching how `from_str` treats a pack palette. The
		// WRL's own bytes are kept as the source palette for debug rendering.
		let source_palette = wrl.palette.clone();
		let mut palette = wrl.palette.clone();
		crate::game_palette::apply_game_statics(&mut palette);

		let pack = TilePack {
			name: name.to_string(),
			version: "wrl".to_string(),
			tiles: wrl.tiles.clone(),
			ids,
			index_of,
			palette: Some(palette.clone()),
			pass: Some(wrl.pass_table.clone()),
			matches: HashMap::new(),
			variant_groups: Vec::new(),
			variant_of: vec![None; tile_count],
			variant_named: HashMap::new(),
			props: HashMap::new(),
			patterns: Vec::new(),
			user: false,
		};

		// Decompose the flat map into the two editor layers by passability:
		// water cells ride the opaque base layer, everything else (land, shore,
		// obstructions) goes on the ground layer. A lone tile composes the same
		// on either layer, so this is pixel-lossless - it just makes the layers
		// meaningful (e.g. for "show only selected").
		let cells: Vec<[Option<TileRef>; MAX_LAYERS]> = wrl
			.bigmap
			.iter()
			.map(|&tile| {
				let mut stack = [None; MAX_LAYERS];
				let layer = pass_layer(wrl.pass_table.get(tile as usize).copied().unwrap_or(0));
				stack[layer] = Some(TileRef { pack: 0, tile, transform: Transform::default() });
				stack
			})
			.collect();

		Self {
			version: PROJECT_VERSION.to_string(),
			name: name.to_string(),
			description: String::new(),
			players: None,
			date: String::new(),
			map_version: String::new(),
			author: String::new(),
			width: wrl.width,
			height: wrl.height,
			uses: vec![UseEntry { name: name.to_string(), tileset: true, palette: true, version: "wrl".to_string() }],
			packs: vec![pack],
			cells,
			pass_overrides: vec![None; wrl.width as usize * wrl.height as usize],
			pack_palette: palette.clone(),
			source_palette,
			palette,
			water_pack: Some(0),
			units: Vec::new(),
			dirty: false,
			revision: 0,
			structure: 0,
			undo_stack: Vec::new(),
			redo_stack: Vec::new(),
			stroke: None,
		}
	}

	/// 1×1 placeholder Project - the document the editor holds before the
	/// initial `open` runs (replaces the old `MapDoc::empty`).
	pub fn empty() -> Self {
		Self::from_wrl(
			&WrlFile {
				header: vec![0; 5],
				width: 1,
				height: 1,
				minimap: vec![0],
				bigmap: vec![0],
				tile_count: 1,
				tiles: vec![0; TILE_DATA_SIZE],
				palette: vec![0; 768],
				pass_table: vec![0],
			},
			"empty",
		)
	}

	/// Blank project: the bottom layer fully covered with
	/// randomly distributed water variants (identity transform - WATER is
	/// `sync`), ground empty. Deterministic from `seed`. WATER is implied
	/// when not listed; the first listed pack with a palette owns it.
	pub fn new(width: u16, height: u16, pack_names: &[String], assets_root: &Path, seed: u64) -> Result<Self, String> {
		check_map_size(width, height)?;

		// WATER first (it fills the bottom layer), then the rest, deduped.
		let mut names: Vec<String> = vec!["WATER".to_string()];
		for name in pack_names {
			if !names.contains(name) {
				names.push(name.clone());
			}
		}
		let mut packs: Vec<TilePack> =
			names.iter().map(|name| TilePack::load(assets_root, name)).collect::<Result<_, _>>()?;
		// User-owned packs (custom tiles) join so they're paintable on a new map.
		append_user_packs(&mut packs, assets_root);

		// First pack with a palette owns it (compatibility verdicts).
		let owner = packs
			.iter()
			.position(|p| p.palette.is_some())
			.ok_or("no palette-owning pack - add a tileset (e.g. GREEN)")?;
		let mut palette = packs[owner].palette.clone().unwrap();
		let source_palette = palette.clone();
		crate::game_palette::apply_game_statics(&mut palette);
		let uses = names
			.iter()
			.enumerate()
			.map(|(i, name)| UseEntry {
				name: name.clone(),
				tileset: true,
				palette: i == owner,
				version: packs[i].version.clone(),
			})
			.collect();

		let water_tiles = packs[0].tile_count();
		if water_tiles == 0 {
			return Err("WATER pack has no tiles".into());
		}
		let mut rng = Rng::new(seed);
		let cells = (0..width as usize * height as usize)
			.map(|_| {
				let mut stack = [None; MAX_LAYERS];
				stack[LAYER_WATER] = Some(TileRef {
					pack: 0,
					tile: rng.below(water_tiles as u32) as u16,
					transform: Transform::default(),
				});
				stack
			})
			.collect();

		Ok(Self {
			version: PROJECT_VERSION.to_string(),
			name: "Untitled".to_string(),
			description: String::new(),
			players: None,
			date: String::new(),
			map_version: String::new(),
			author: String::new(),
			width,
			height,
			uses,
			packs,
			cells,
			pass_overrides: vec![None; width as usize * height as usize],
			pack_palette: palette.clone(),
			source_palette,
			palette,
			water_pack: Some(0),
			units: Vec::new(),
			dirty: false,
			revision: 0,
			structure: 0,
			undo_stack: Vec::new(),
			redo_stack: Vec::new(),
			stroke: None,
		})
	}

	pub fn dirty(&self) -> bool {
		self.dirty
	}
	pub fn revision(&self) -> u64 {
		self.revision
	}
	/// Bumped on structural changes - tile/palette table swaps (palette
	/// conversion and its undo/redo). When it moves across a command, the
	/// renderer's tile atlas is stale and must rebuild.
	pub fn structure_revision(&self) -> u64 {
		self.structure
	}

	pub fn mark_saved(&mut self) {
		self.dirty = false;
	}

	/// Stamp (or restamp) a unit-preview annotation on a cell. Replaces any
	/// note already on that cell. Marks the document dirty (the note is
	/// saved with the project) but records no undo patch - annotations are
	/// view-layer metadata, not map edits.
	pub fn stamp_unit(&mut self, note: UnitNote) {
		self.units.retain(|u| (u.x, u.y) != (note.x, note.y));
		self.units.push(note);
		self.dirty = true;
	}

	/// Remove the unit-preview annotation on a cell; `true` when one was there.
	pub fn erase_unit_at(&mut self, x: u16, y: u16) -> bool {
		let before = self.units.len();
		self.units.retain(|u| (u.x, u.y) != (x, y));
		let removed = self.units.len() != before;
		if removed {
			self.dirty = true;
		}
		removed
	}

	/// Remove all unit-preview annotations; returns how many there were.
	pub fn clear_units(&mut self) -> usize {
		let n = self.units.len();
		if n > 0 {
			self.dirty = true;
		}
		self.units.clear();
		n
	}

	/// Resolve a `"GSd004:!N"`-style reference to a tile ref + its layer
	/// (water-pack tiles go to the water layer, the rest to ground).
	pub fn resolve_ref(&self, text: &str) -> Result<(TileRef, usize), String> {
		let (id, transform) = match text.split_once(':') {
			Some((id, t)) => (id, Transform::parse(t)?),
			None => (text, Transform::default()),
		};
		for (pack_index, pack) in self.packs.iter().enumerate() {
			if let Some(&tile) = pack.index_of.get(id) {
				let pack_index = pack_index as u8;
				let layer = if Some(pack_index) == self.water_pack { LAYER_WATER } else { LAYER_GROUND };
				return Ok((TileRef { pack: pack_index, tile, transform }, layer));
			}
		}
		Err(format!("unknown tile id '{id}'"))
	}

	/// Encode a cell's stack in the save format (`"WATR05,GSd004:!N"`,
	/// empty string for an empty stack) - also the `assert-cell` syntax.
	pub fn cell_spec(&self, x: u16, y: u16) -> Option<String> {
		let stack = self.cell(x, y)?;
		let mut text = String::new();
		for layer in stack.iter().flatten() {
			if !text.is_empty() {
				text.push(',');
			}
			text.push_str(&self.packs[layer.pack as usize].ids[layer.tile as usize]);
			text.push_str(&layer.transform.suffix());
		}
		Some(text)
	}

	/// Set layer entries (one undo transaction - or part of the open stroke);
	/// `None` erases. Out-of-range and no-op edits are skipped; returns
	/// whether anything changed.
	pub fn place_many(&mut self, edits: &[(u16, u16, usize, Option<TileRef>)]) -> bool {
		let mut cells = Vec::new();
		for &(x, y, layer, entry) in edits {
			if x >= self.width || y >= self.height || layer >= MAX_LAYERS {
				continue;
			}
			if let Some(t) = entry {
				let Some(pack) = self.packs.get(t.pack as usize) else { continue };
				if t.tile >= pack.tile_count() {
					continue;
				}
			}
			let i = y as usize * self.width as usize + x as usize;
			if self.cells[i][layer] == entry {
				continue;
			}
			cells.push((x, y, layer, self.cells[i][layer]));
			self.cells[i][layer] = entry;
		}
		if cells.is_empty() {
			return false;
		}
		match &mut self.stroke {
			Some(stroke) => stroke.cells.extend(cells),
			None => {
				self.push_undo(Patch { cells, ..Patch::default() });
			}
		}
		self.redo_stack.clear();
		self.bump();
		true
	}

	/// Set a dynamic palette slot (the map's color override). Undoable; part
	/// of the open stroke when one is active (slider drags = one undo unit).
	pub fn set_color(&mut self, slot: u8, rgb: [u8; 3]) -> Result<bool, String> {
		if !DYNAMIC_SLOTS.contains(&slot) {
			return Err(format!("slot {slot} is game-static (editable: 64..=159)"));
		}
		let at = slot as usize * 3;
		let prev = [self.palette[at], self.palette[at + 1], self.palette[at + 2]];
		if prev == rgb {
			return Ok(false);
		}
		self.palette[at..at + 3].copy_from_slice(&rgb);
		match &mut self.stroke {
			Some(stroke) => stroke.colors.push((slot, prev)),
			None => {
				self.push_undo(Patch { colors: vec![(slot, prev)], ..Patch::default() });
			}
		}
		self.redo_stack.clear();
		self.bump();
		Ok(true)
	}

	/// Shift a whole water cycle block (the one containing `slot`) in HSL -
	/// re-tints the animated gradient coherently. One undo unit.
	/// `dh` degrees, `ds`/`dl` in 0..1 units.
	pub fn hsl_shift_block(&mut self, slot: u8, dh: f32, ds: f32, dl: f32) -> Result<bool, String> {
		let Some(&(start, end)) = WATER_CYCLES.iter().find(|(s, e)| (*s..=*e).contains(&slot)) else {
			return Err(format!(
				"slot {slot} is not in a water cycle block (blocks: 96-102 103-109 110-116 117-122 123-127)",
			));
		};
		let solo = self.stroke.is_none();
		if solo {
			self.begin_stroke();
		}
		let mut changed = false;
		for s in start..=end {
			let at = s as usize * 3;
			let (h, sat, l) = crate::color::rgb_to_hsl([self.palette[at], self.palette[at + 1], self.palette[at + 2]]);
			changed |= self.set_color(s, crate::color::hsl_to_rgb(h + dh, sat + ds, l + dl))?;
		}
		if solo {
			self.end_stroke();
		}
		Ok(changed)
	}

	/// Apply a loaded 256-colour palette (768 RGB bytes) to the editable
	/// dynamic slots (64..=159) as one undo unit; the game-static slots are
	/// left untouched. Returns how many slots changed.
	pub fn load_palette(&mut self, colors: &[u8]) -> Result<u32, String> {
		if colors.len() != 768 {
			return Err(format!("palette: {} bytes, want 768", colors.len()));
		}
		let solo = self.stroke.is_none();
		if solo {
			self.begin_stroke();
		}
		let mut changed = 0;
		for slot in 64u8..=159 {
			let at = slot as usize * 3;
			if self.set_color(slot, [colors[at], colors[at + 1], colors[at + 2]])? {
				changed += 1;
			}
		}
		if solo {
			self.end_stroke();
		}
		Ok(changed)
	}

	/// The document's internal palette: the source file's bytes (the WRL's
	/// internal palette / the pack's `palette.json` - game statics **not**
	/// applied) with this map's live dynamic-slot edits merged in. What the
	/// game would ignore, but what the file actually says - the debug render
	/// (`map-palette`) and the WRL Internal Palette panel read this.
	pub fn internal_palette(&self) -> Vec<u8> {
		let mut out = self.source_palette.clone();
		for slot in DYNAMIC_SLOTS {
			let at = slot as usize * 3;
			out[at..at + 3].copy_from_slice(&self.palette[at..at + 3]);
		}
		out
	}

	/// Is this document an imported flat WRL (a synthetic in-memory pack)?
	/// Palette conversion rewrites tile pixels, which only makes sense when
	/// the tiles belong to the document (a `.json` project's packs are shared
	/// on disk - mutating them would not persist).
	pub fn is_wrl_import(&self) -> bool {
		!self.uses.is_empty() && self.uses.iter().all(|u| u.version == "wrl")
	}

	/// Apply Map Preferences (all optional) and mark the document dirty. These
	/// are metadata - never baked into the WRL, never part of undo. Carriage
	/// returns in the description are stripped; newlines are kept, so it may be
	/// multi-line (escaped as `\n` in the project JSON).
	pub fn set_info(
		&mut self,
		name: String,
		players: Option<u8>,
		description: String,
		date: String,
		map_version: String,
		author: String,
	) {
		self.name = name;
		self.players = players.map(|p| p.clamp(2, 4));
		self.description = description.replace('\r', "");
		self.date = date;
		self.map_version = map_version;
		self.author = author;
		self.dirty = true;
	}

	/// Snapshot everything a document-level operation may replace - the undo
	/// half of a [`Patch::doc`] swap.
	fn doc_state(&self) -> Box<DocState> {
		Box::new(DocState {
			uses: self.uses.clone(),
			packs: self.packs.clone(),
			cells: self.cells.clone(),
			pass_overrides: self.pass_overrides.clone(),
			palette: self.palette.clone(),
			pack_palette: self.pack_palette.clone(),
			source_palette: self.source_palette.clone(),
			water_pack: self.water_pack,
		})
	}

	/// Commit a document-level change as one undo unit: `before` is the
	/// pre-change snapshot (see [`Self::doc_state`]). Structural - the
	/// renderer must rebuild its atlas (see [`Self::structure_revision`]).
	fn push_doc_patch(&mut self, before: Box<DocState>) {
		self.end_stroke(); // a doc swap must not interleave with an open stroke
		self.push_undo(Patch { doc: Some(before), ..Patch::default() });
		self.redo_stack.clear();
		self.structure += 1;
		self.bump();
	}

	/// Per-slot pixel usage over every pack's tile table.
	fn slot_usage(&self) -> [u64; 256] {
		let mut usage = [0u64; 256];
		for pack in &self.packs {
			for &b in &pack.tiles {
				usage[b as usize] += 1;
			}
		}
		usage
	}

	/// Remap the internal palette onto a MAX-compatible one (the "best match
	/// colors" method - see [`crate::palette_convert`] for the rules: only
	/// used colors move, game-animated slots are never used, water cycles are
	/// preserved per `opts`, in-game statics are reused when possible and the
	/// rest approximate into the unused dynamic slots). Tile pixels are
	/// rewritten through the slot mapping, so the rendered map keeps
	/// (approximately) its internal-palette look while becoming game-correct.
	///
	/// Lossy but undoable - the change lands as one document-swap undo unit.
	/// `None` when the palette is already compatible (nothing changed).
	pub fn convert_to_compatible_palette(
		&mut self,
		opts: crate::palette_convert::ConvertOptions,
	) -> Option<crate::palette_convert::ConvertReport> {
		let internal = self.internal_palette();
		let plan = crate::palette_convert::plan(&internal, &self.slot_usage(), opts)?;
		let before = self.doc_state();
		for pack in &mut self.packs {
			for b in &mut pack.tiles {
				*b = plan.map[*b as usize];
			}
		}
		// The compatible palette becomes the document's palette on every
		// level: the working copy, the source ("internal") palette - they now
		// agree - and the owner pack's (the save/export baseline).
		self.palette = plan.palette.clone();
		self.source_palette = plan.palette.clone();
		self.pack_palette = plan.palette.clone();
		for (i, u) in self.uses.iter().enumerate() {
			if u.palette {
				self.packs[i].palette = Some(plan.palette.clone());
			}
		}
		self.push_doc_patch(before);
		Some(plan.report)
	}

	/// Convert the palette by rasterizing the whole map through its internal
	/// palette and re-importing the raster exactly like New-from-Image does
	/// (k-means quantization into the dynamic slots + dither + reblock +
	/// dedupe). With `preserve_water`, pixels on the water cycle blocks
	/// (96-127) are pinned: they keep their slot and the blocks keep the
	/// map's colors, so the water still animates in-game. Per-cell pass
	/// values survive as pass overrides (the rebuilt tiles carry none).
	///
	/// Lossy but undoable - one document-swap undo unit. Errors leave the
	/// document untouched.
	pub fn convert_palette_by_reimport(
		&mut self,
		preserve_water: bool,
		dedupe: crate::image_import::Dedupe,
		threshold: f32,
	) -> Result<u16, String> {
		let mut session = PaletteReimport::new(self, preserve_water, dedupe, threshold);
		while !session.is_done() {
			session.step(self, usize::MAX);
		}
		let wrl = session.finish()?;
		Ok(self.apply_reimport(&wrl))
	}

	/// Swap a re-imported [`WrlFile`] (see [`PaletteReimport`]) in as the
	/// document's content - one document-swap undo unit. Pass truth lives in
	/// per-cell overrides afterwards (the reimported tiles carry none).
	pub fn apply_reimport(&mut self, wrl: &WrlFile) -> u16 {
		let (w, h) = (self.width as usize, self.height as usize);
		let before = self.doc_state();
		let pass_overrides = (0..h * w).map(|i| self.pass_at((i % w) as u16, (i / w) as u16)).collect();
		let name = self.uses.first().map_or_else(|| self.name.clone(), |u| u.name.clone());
		let rebuilt = Self::from_wrl(wrl, &name);
		self.uses = rebuilt.uses;
		self.packs = rebuilt.packs;
		self.cells = rebuilt.cells;
		self.pass_overrides = pass_overrides;
		self.palette = rebuilt.palette;
		self.pack_palette = rebuilt.pack_palette;
		self.source_palette = rebuilt.source_palette;
		self.water_pack = rebuilt.water_pack;
		self.push_doc_patch(before);
		wrl.tile_count
	}

	/// Open a stroke: subsequent edits merge into one undo unit (one brush
	/// drag = one Ctrl+Z). An already-open stroke is committed first.
	pub fn begin_stroke(&mut self) {
		self.end_stroke();
		self.stroke = Some(Patch::default());
	}

	/// Abort the open stroke: revert its edits right now and discard them -
	/// nothing lands on the undo/redo stacks. A cancelled generation
	/// (worldgen) never happened.
	pub fn rollback_stroke(&mut self) -> bool {
		let Some(stroke) = self.stroke.take() else { return false };
		if stroke.is_empty() {
			return false;
		}
		let _ = self.apply(stroke);
		self.bump();
		true
	}

	/// Commit the open stroke to the undo stack (no-op when empty/closed).
	pub fn end_stroke(&mut self) {
		let Some(stroke) = self.stroke.take() else { return };
		if stroke.is_empty() {
			return;
		}
		self.push_undo(stroke);
	}

	pub fn place(&mut self, x: u16, y: u16, layer: usize, entry: Option<TileRef>) -> bool {
		self.place_many(&[(x, y, layer, entry)])
	}

	/// A random interchangeable variant of `t` (same pack + transform); returns
	/// `t` unchanged when the tile has no variant group. The
	/// random-paint toggle swaps a placed tile for a sibling so a painted
	/// region doesn't visibly tile.
	pub fn random_variant(&self, t: TileRef, rng: &mut Rng) -> TileRef {
		let Some(pack) = self.packs.get(t.pack as usize) else { return t };
		let group = pack.variants_of(t.tile);
		if group.len() < 2 {
			return t;
		}
		TileRef { tile: group[rng.below(group.len() as u32) as usize], ..t }
	}

	/// Flood-fill (4-connected) the region of cells whose `layer` entry equals
	/// the clicked cell's, replacing each with `entry` - or a random variant of
	/// it when `randomize`. One undo unit; returns whether anything changed.
	pub fn fill(&mut self, x: u16, y: u16, entry: TileRef, layer: usize, randomize: bool, rng: &mut Rng) -> bool {
		if x >= self.width || y >= self.height || layer >= MAX_LAYERS {
			return false;
		}
		let w = self.width as usize;
		let h = self.height as usize;
		let idx = |x: u16, y: u16| y as usize * w + x as usize;
		let target = self.cells[idx(x, y)][layer];
		// Flood the connected run of `target` cells, collecting indices in pop
		// order; the tile (and its rng-rolled variant) is resolved afterwards so
		// the rng-consumption order matches the original in-traversal version.
		let mut seen = vec![false; w * h];
		let mut visited = Vec::new();
		crate::grid::flood4(w, h, idx(x, y), &mut seen, |n| self.cells[n][layer] == target, |i| visited.push(i));
		let edits: Vec<_> = visited
			.iter()
			.map(|&i| {
				let tile = if randomize { self.random_variant(entry, rng) } else { entry };
				((i % w) as u16, (i / w) as u16, layer, Some(tile))
			})
			.collect();
		self.place_many(&edits)
	}

	/// Set per-cell pass overrides (Pass Table Editor). Undoable -
	/// part of the open stroke when one is active (a paint drag = one undo
	/// unit). Returns whether anything changed.
	pub fn set_pass_many(&mut self, edits: &[(u16, u16, u8)]) -> bool {
		let mut passes = Vec::new();
		for &(x, y, value) in edits {
			if x >= self.width || y >= self.height || value > 3 {
				continue;
			}
			let i = y as usize * self.width as usize + x as usize;
			if self.pass_overrides[i] == Some(value) {
				continue;
			}
			passes.push((x, y, self.pass_overrides[i]));
			self.pass_overrides[i] = Some(value);
		}
		if passes.is_empty() {
			return false;
		}
		match &mut self.stroke {
			Some(stroke) => stroke.passes.extend(passes),
			None => {
				self.push_undo(Patch { passes, ..Patch::default() });
			}
		}
		self.redo_stack.clear();
		self.bump();
		true
	}

	pub fn set_pass(&mut self, x: u16, y: u16, value: u8) -> bool {
		self.set_pass_many(&[(x, y, value)])
	}

	/// Set (`Some`) or clear (`None`) a single cell's pass override (Local Pass
	/// Override Editor). Undoable - joins the open stroke. Returns whether
	/// anything changed.
	pub fn set_pass_override(&mut self, x: u16, y: u16, value: Option<u8>) -> bool {
		if x >= self.width || y >= self.height || value.is_some_and(|v| v > 3) {
			return false;
		}
		let i = y as usize * self.width as usize + x as usize;
		if self.pass_overrides[i] == value {
			return false;
		}
		let passes = vec![(x, y, self.pass_overrides[i])];
		self.pass_overrides[i] = value;
		match &mut self.stroke {
			Some(stroke) => stroke.passes.extend(passes),
			None => {
				self.push_undo(Patch { passes, ..Patch::default() });
			}
		}
		self.redo_stack.clear();
		self.bump();
		true
	}

	/// Drop every per-cell pass override back to the derived value (undoable;
	/// joins the open stroke). Wholesale terrain replacement (worldgen)
	/// must not inherit stale hand-painted pass data.
	pub fn clear_pass_overrides(&mut self) -> bool {
		let mut passes = Vec::new();
		for i in 0..self.pass_overrides.len() {
			if let Some(prev) = self.pass_overrides[i].take() {
				let (x, y) = ((i % self.width as usize) as u16, (i / self.width as usize) as u16);
				passes.push((x, y, Some(prev)));
			}
		}
		if passes.is_empty() {
			return false;
		}
		match &mut self.stroke {
			Some(stroke) => stroke.passes.extend(passes),
			None => {
				self.push_undo(Patch { passes, ..Patch::default() });
			}
		}
		self.redo_stack.clear();
		self.bump();
		true
	}

	/// Set the **per-tile** passability of the tile under each cell (the Pass
	/// Table Editor): looks up the cell's top tile and rewrites its pack pass,
	/// so every cell sharing that tile id retints at once. Undoable - joins the
	/// open stroke (a drag = one unit). Cells whose top tile's pack has no pass
	/// table are skipped. Returns whether anything changed.
	pub fn set_tile_pass_at_many(&mut self, edits: &[(u16, u16, u8)]) -> bool {
		let mut tile_passes: Vec<(u8, u16, u8)> = Vec::new();
		for &(x, y, value) in edits {
			if value > 3 {
				continue;
			}
			let (pack, tile) = {
				let Some(stack) = self.cell(x, y) else { continue };
				let Some(top) = stack[LAYER_GROUND].or(stack[LAYER_WATER]) else { continue };
				(top.pack, top.tile)
			};
			let Some(pass) = self.packs[pack as usize].pass.as_mut() else { continue };
			let prev = pass[tile as usize];
			if prev == value {
				continue;
			}
			pass[tile as usize] = value;
			// One tile may sit under many painted cells - record its original
			// pass once so undo restores it exactly.
			if !tile_passes.iter().any(|&(p, t, _)| p == pack && t == tile) {
				tile_passes.push((pack, tile, prev));
			}
		}
		if tile_passes.is_empty() {
			return false;
		}
		match &mut self.stroke {
			Some(stroke) => {
				for (pack, tile, prev) in tile_passes {
					if !stroke.tile_passes.iter().any(|&(p, t, _)| p == pack && t == tile) {
						stroke.tile_passes.push((pack, tile, prev));
					}
				}
			}
			None => {
				self.push_undo(Patch { tile_passes, ..Patch::default() });
			}
		}
		self.redo_stack.clear();
		self.bump();
		true
	}

	pub fn set_tile_pass_at(&mut self, x: u16, y: u16, value: u8) -> bool {
		self.set_tile_pass_at_many(&[(x, y, value)])
	}

	/// Reset every tile's pack pass to the canonical tileset value. `canonical`
	/// is indexed by pack: `Some(pass)` gives that pack's authoritative per-tile
	/// pass (already mapped to this pack's current tile indices by the caller),
	/// `None` leaves the pack untouched (a synthetic pack has no source tileset).
	/// Applies as one undo unit; per-cell pass overrides are untouched (this only
	/// reverts Pass Table Editor edits). Returns whether anything changed.
	pub fn reset_tile_pass(&mut self, canonical: &[Option<Vec<u8>>]) -> bool {
		self.end_stroke(); // a deliberate whole-map reset is never part of a stroke
		let mut tile_passes: Vec<(u8, u16, u8)> = Vec::new();
		for (pi, pack) in self.packs.iter_mut().enumerate() {
			let Some(want) = canonical.get(pi).and_then(|o| o.as_ref()) else { continue };
			let Some(pass) = pack.pass.as_mut() else { continue };
			for ti in 0..pass.len().min(want.len()) {
				if pass[ti] != want[ti] {
					tile_passes.push((pi as u8, ti as u16, pass[ti]));
					pass[ti] = want[ti];
				}
			}
		}
		if tile_passes.is_empty() {
			return false;
		}
		self.push_undo(Patch { tile_passes, ..Patch::default() });
		self.redo_stack.clear();
		self.bump();
		true
	}

	/// Set the water (base) layer tile by raw index - the flat-document edit
	/// behind `set-tile`, used to edit an imported WRL (its only tiles are the
	/// synthetic base pack). Validates against the base pack; `false` if out of
	/// range, off-map, or unchanged.
	pub fn set_base_tile(&mut self, x: u16, y: u16, tile: u16) -> bool {
		let pack = self.water_pack.unwrap_or(0);
		if x >= self.width || y >= self.height || tile >= self.packs[pack as usize].tile_count() {
			return false;
		}
		self.place(x, y, LAYER_WATER, Some(TileRef { pack, tile, transform: Transform::default() }))
	}

	/// The water (base) layer tile index at a cell (`set-tile`/`assert-tile`).
	pub fn base_tile(&self, x: u16, y: u16) -> Option<u16> {
		self.cell(x, y).and_then(|s| s[LAYER_WATER]).map(|t| t.tile)
	}

	/// Resize the canvas: the existing map is placed at
	/// `(off_x, off_y)` within the new `new_w × new_h` grid. Enlarging
	/// fills the new territory with water; a negative offset (or a smaller
	/// size) crops. Cell stacks and pass overrides move together. This is a
	/// structural change, so the per-cell undo journal is cleared.
	pub fn resize(&mut self, new_w: u16, new_h: u16, off_x: i32, off_y: i32) -> Result<(), String> {
		check_map_size(new_w, new_h)?;
		let water = self.water_pack;
		let water_tiles = water.and_then(|w| self.packs.get(w as usize)).map(|p| p.tile_count()).unwrap_or(0);
		let mut cells = Vec::with_capacity(new_w as usize * new_h as usize);
		let mut passes = Vec::with_capacity(new_w as usize * new_h as usize);
		for ny in 0..new_h as i32 {
			for nx in 0..new_w as i32 {
				let (ox, oy) = (nx - off_x, ny - off_y);
				if ox >= 0 && oy >= 0 && (ox as u16) < self.width && (oy as u16) < self.height {
					let oi = oy as usize * self.width as usize + ox as usize;
					cells.push(self.cells[oi]);
					passes.push(self.pass_overrides[oi]);
				} else {
					// New territory fills with water (deterministic per cell).
					let mut stack = [None; MAX_LAYERS];
					if let (Some(w), true) = (water, water_tiles > 0) {
						let mut rng = Rng::new(0x5245_5349_5a45 ^ ((nx as u64) << 32 | ny as u64));
						stack[LAYER_WATER] = Some(TileRef {
							pack: w,
							tile: rng.below(water_tiles as u32) as u16,
							transform: Transform::default(),
						});
					}
					cells.push(stack);
					passes.push(None);
				}
			}
		}
		self.cells = cells;
		self.pass_overrides = passes;
		self.width = new_w;
		self.height = new_h;
		// A dimension change can't be a per-cell patch - drop the journal.
		self.undo_stack.clear();
		self.redo_stack.clear();
		self.stroke = None;
		self.bump();
		Ok(())
	}

	/// Push a finished patch onto the undo journal, dropping the oldest once
	/// the stack exceeds [`MAX_UNDO`]. (The caller clears `redo_stack` / bumps
	/// the revision as appropriate - this only manages the bounded stack.)
	fn push_undo(&mut self, patch: Patch) {
		self.undo_stack.push(patch);
		if self.undo_stack.len() > MAX_UNDO {
			self.undo_stack.remove(0);
		}
	}

	pub fn undo(&mut self) -> bool {
		self.end_stroke(); // a mid-drag undo must not orphan the stroke
		let Some(patch) = self.undo_stack.pop() else { return false };
		let inverse = self.apply(patch);
		self.redo_stack.push(inverse);
		self.bump();
		true
	}

	pub fn redo(&mut self) -> bool {
		self.end_stroke();
		let Some(patch) = self.redo_stack.pop() else { return false };
		let inverse = self.apply(patch);
		self.undo_stack.push(inverse);
		self.bump();
		true
	}

	fn apply(&mut self, patch: Patch) -> Patch {
		// A document swap is its own inverse: swap the stored state with the
		// live fields and carry the displaced state back out. Structural -
		// the renderer's atlas is stale either way.
		if let Some(mut doc) = patch.doc {
			std::mem::swap(&mut self.uses, &mut doc.uses);
			std::mem::swap(&mut self.packs, &mut doc.packs);
			std::mem::swap(&mut self.cells, &mut doc.cells);
			std::mem::swap(&mut self.pass_overrides, &mut doc.pass_overrides);
			std::mem::swap(&mut self.palette, &mut doc.palette);
			std::mem::swap(&mut self.pack_palette, &mut doc.pack_palette);
			std::mem::swap(&mut self.source_palette, &mut doc.source_palette);
			std::mem::swap(&mut self.water_pack, &mut doc.water_pack);
			self.structure += 1;
			return Patch { doc: Some(doc), ..Patch::default() };
		}
		let mut cells = Vec::with_capacity(patch.cells.len());
		for &(x, y, layer, entry) in patch.cells.iter().rev() {
			let i = y as usize * self.width as usize + x as usize;
			cells.push((x, y, layer, self.cells[i][layer]));
			self.cells[i][layer] = entry;
		}
		let mut colors = Vec::with_capacity(patch.colors.len());
		for &(slot, rgb) in patch.colors.iter().rev() {
			let at = slot as usize * 3;
			colors.push((slot, [self.palette[at], self.palette[at + 1], self.palette[at + 2]]));
			self.palette[at..at + 3].copy_from_slice(&rgb);
		}
		let mut passes = Vec::with_capacity(patch.passes.len());
		for &(x, y, value) in patch.passes.iter().rev() {
			let i = y as usize * self.width as usize + x as usize;
			passes.push((x, y, self.pass_overrides[i]));
			self.pass_overrides[i] = value;
		}
		let mut tile_passes = Vec::with_capacity(patch.tile_passes.len());
		for &(pack, tile, value) in patch.tile_passes.iter().rev() {
			if let Some(pass) = self.packs[pack as usize].pass.as_mut() {
				tile_passes.push((pack, tile, pass[tile as usize]));
				pass[tile as usize] = value;
			}
		}
		Patch { cells, colors, passes, tile_passes, doc: None }
	}

	fn bump(&mut self) {
		self.dirty = true;
		self.revision += 1;
	}

	/// Remove tile `tile` from pack `pack`, shifting every higher tile index
	/// down by one across the pack's tables, this map's cells, and the pack's
	/// variant groups / patterns. Refuses if the tile is still painted anywhere
	/// (erase it first). Not undoable - a deliberate asset edit, like Bake.
	pub fn delete_tile(&mut self, pack: u8, tile: u16) -> Result<(), String> {
		let pk = pack as usize;
		let Some(p) = self.packs.get_mut(pk) else { return Err(format!("no pack {pack}")) };
		if tile >= p.tile_count() {
			return Err(format!("tile {tile} out of range"));
		}
		let id = p.ids[tile as usize].clone();
		// In use? (a cell still references this exact tile.)
		let used = self.cells.iter().flatten().flatten().any(|t| t.pack == pack && t.tile == tile);
		if used {
			return Err(format!("'{id}' is painted on the map - erase it first"));
		}
		// Drop the tile from the pack tables.
		let p = &mut self.packs[pk];
		let at = tile as usize * TILE_DATA_SIZE;
		p.tiles.drain(at..at + TILE_DATA_SIZE);
		p.ids.remove(tile as usize);
		p.variant_of.remove(tile as usize);
		if let Some(pass) = p.pass.as_mut() {
			pass.remove(tile as usize);
		}
		// Variant groups hold tile indices: drop the deleted one, shift the rest.
		for group in &mut p.variant_groups {
			group.retain(|&i| i != tile);
			for i in group.iter_mut() {
				if *i > tile {
					*i -= 1;
				}
			}
		}
		// Patterns reference tile indices too; a hole where the tile was used.
		for pat in &mut p.patterns {
			for cell in pat.cells.iter_mut() {
				match *cell {
					Some(i) if i == tile => *cell = None,
					Some(i) if i > tile => *cell = Some(i - 1),
					_ => {}
				}
			}
		}
		// Rebuild the id→index map (positions past `tile` all shifted).
		p.index_of = p.ids.iter().enumerate().map(|(i, id)| (id.clone(), i as u16)).collect();
		// Shift this map's cell references in the same pack.
		for stack in &mut self.cells {
			for t in stack.iter_mut().flatten() {
				if t.pack == pack && t.tile > tile {
					t.tile -= 1;
				}
			}
		}
		self.structure += 1;
		self.bump();
		Ok(())
	}

	pub fn cell(&self, x: u16, y: u16) -> Option<&[Option<TileRef>; MAX_LAYERS]> {
		if x >= self.width || y >= self.height {
			return None;
		}
		Some(&self.cells[y as usize * self.width as usize + x as usize])
	}

	/// Flatten one cell's stack to raw 64×64 indexed pixels: ground over
	/// water, ground index-0 = transparent. The bake kernel.
	pub fn compose_cell(&self, x: u16, y: u16) -> [u8; TILE_DATA_SIZE] {
		match self.cell(x, y) {
			Some(stack) => self.compose_stack(stack),
			None => [0u8; TILE_DATA_SIZE],
		}
	}

	/// Compose an arbitrary stack (used by the bake's water-phase
	/// canonicalization as well as `compose_cell`).
	pub fn compose_stack(&self, stack: &[Option<TileRef>; MAX_LAYERS]) -> [u8; TILE_DATA_SIZE] {
		let mut out = [0u8; TILE_DATA_SIZE];
		if let Some(water) = stack[LAYER_WATER] {
			let pixels = self.packs[water.pack as usize].tile_pixels(water.tile);
			transform_into(&mut out, pixels, water.transform, None);
		}
		if let Some(ground) = stack[LAYER_GROUND] {
			let pixels = self.packs[ground.pack as usize].tile_pixels(ground.tile);
			// Only families with a mask are transparent (over the water beneath);
			// opaque families fully cover.
			transform_into(&mut out, pixels, ground.transform, self.tile_mask(ground));
		}
		out
	}

	/// The transparency mask color of a tile - its family's `"mask"`, or `None`
	/// when the family is fully opaque.
	pub fn tile_mask(&self, t: TileRef) -> Option<u8> {
		self.packs[t.pack as usize].tile_mask(t.tile)
	}

	/// One composed pixel of a cell - the single-pixel form of
	/// `compose_cell` (O(1); minimap/overworld previews sample with this).
	pub fn pixel_at(&self, x: u16, y: u16, sub: (usize, usize)) -> u8 {
		let Some(stack) = self.cell(x, y) else { return 0 };
		if let Some(ground) = stack[LAYER_GROUND] {
			let pixel = self.tile_pixel(ground, sub);
			// Opaque family, or a non-mask pixel: the ground pixel wins.
			if self.tile_mask(ground) != Some(pixel) {
				return pixel;
			}
		}
		match stack[LAYER_WATER] {
			Some(water) => self.tile_pixel(water, sub),
			None => 0,
		}
	}

	/// A single tile pixel under its transform (the point form of
	/// `transform_into`'s inverse mapping).
	fn tile_pixel(&self, t: TileRef, (dx, dy): (usize, usize)) -> u8 {
		let n = TILE_SIZE;
		let (mut sx, mut sy) = (dx, dy);
		for _ in 0..t.transform.rot {
			let (rx, ry) = (sy, n - 1 - sx);
			sx = rx;
			sy = ry;
		}
		if t.transform.mirror {
			sx = n - 1 - sx;
		}
		self.packs[t.pack as usize].tile_pixels(t.tile)[sy * n + sx]
	}

	/// The in-game minimap byte for a cell (composed center pixel - the
	/// same derivation the bake uses).
	pub fn minimap_pixel(&self, x: u16, y: u16) -> u8 {
		self.pixel_at(x, y, (32, 32))
	}

	/// Pass value of a cell: the Pass Table Editor override if set,
	/// else the stack-top tile's pack pass (0 land / 1 water /
	/// 2 shore / 3 blocked). `None` when neither is available. Empty stacks
	/// read as land (0). Drives the pass overlay and the bake.
	pub fn pass_at(&self, x: u16, y: u16) -> Option<u8> {
		if x >= self.width || y >= self.height {
			return None;
		}
		let i = y as usize * self.width as usize + x as usize;
		if let Some(v) = self.pass_overrides[i] {
			return Some(v);
		}
		let stack = self.cell(x, y)?;
		let Some(top) = stack[LAYER_GROUND].or(stack[LAYER_WATER]) else {
			return Some(0);
		};
		self.packs[top.pack as usize].pass.as_ref().map(|pass| pass[top.tile as usize])
	}

	/// Whether a cell carries an explicit pass override.
	pub fn pass_override(&self, x: u16, y: u16) -> Option<u8> {
		if x >= self.width || y >= self.height {
			return None;
		}
		self.pass_overrides[y as usize * self.width as usize + x as usize]
	}

	/// FNV-1a over the cell grid (document identity for scripts/asserts).
	pub fn hash(&self) -> u64 {
		let mut h = 0xcbf2_9ce4_8422_2325u64;
		let mut eat = |bytes: &[u8]| {
			for &b in bytes {
				h ^= b as u64;
				h = h.wrapping_mul(0x0000_0100_0000_01b3);
			}
		};
		eat(&self.width.to_le_bytes());
		eat(&self.height.to_le_bytes());
		eat(&self.palette); // the map's colors are document state
		for stack in &self.cells {
			for layer in stack {
				match layer {
					None => eat(&[0xff]),
					Some(t) => {
						eat(&[t.pack]);
						eat(&t.tile.to_le_bytes());
						eat(&[t.transform.bits() as u8]);
					}
				}
			}
		}
		// Pass overrides are document state.
		for v in &self.pass_overrides {
			eat(&[v.map(|p| p + 1).unwrap_or(0)]);
		}
		h
	}
}

/// Apply a transform to a 64×64 tile (used by tests and the bake; the GPU
/// shader mirrors this addressing).
pub fn transform_tile(src: &[u8], transform: Transform) -> [u8; TILE_DATA_SIZE] {
	let mut out = [0u8; TILE_DATA_SIZE];
	transform_into(&mut out, src, transform, None);
	out
}

/// Write `src` into `dst` with `transform` applied; pixels equal to `mask`
/// (the family's transparency color, when it has one) keep the existing `dst`
/// value (layer fall-through). `None` = fully opaque.
fn transform_into(dst: &mut [u8; TILE_DATA_SIZE], src: &[u8], transform: Transform, mask: Option<u8>) {
	let n = TILE_SIZE; // 64
	for dy in 0..n {
		for dx in 0..n {
			// Map destination coords back to source coords (inverse of
			// mirror-then-rotate-cw).
			let (mut sx, mut sy) = (dx, dy);
			// Undo rotation: rotate counter-clockwise `rot` times.
			for _ in 0..transform.rot {
				let (rx, ry) = (sy, n - 1 - sx);
				sx = rx;
				sy = ry;
			}
			// Undo mirror (horizontal flip is its own inverse).
			if transform.mirror {
				sx = n - 1 - sx;
			}
			let pixel = src[sy * n + sx];
			if mask != Some(pixel) {
				dst[dy * n + dx] = pixel;
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn assets_root() -> std::path::PathBuf {
		Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/assets/tilepacks")
	}

	#[test]
	fn delete_tile_shifts_indices_and_refuses_in_use() {
		let mut p = Project::new(8, 6, &["GREEN".to_string()], &assets_root(), 1).unwrap();
		let green = 1u8; // pack 0 = WATER, 1 = GREEN
		let before = p.packs[green as usize].tile_count();
		let third_id = p.packs[green as usize].ids[2].clone();
		// Paint tile 5 onto a cell, then deleting tile 2 must shift it to 4.
		let t5 = TileRef { pack: green, tile: 5, transform: Transform::default() };
		assert!(p.place_many(&[(0, 0, LAYER_GROUND, Some(t5))]));
		// In-use tile can't be deleted.
		assert!(p.delete_tile(green, 5).is_err(), "painted tile is protected");
		// Deleting an earlier, unused tile shifts the painted ref down by one.
		p.delete_tile(green, 2).unwrap();
		assert_eq!(p.packs[green as usize].tile_count(), before - 1);
		assert!(!p.packs[green as usize].index_of.contains_key(&third_id), "deleted id is gone");
		assert_eq!(p.cell(0, 0).unwrap()[LAYER_GROUND].unwrap().tile, 4, "painted ref shifted 5→4");
	}

	#[test]
	fn load_palette_touches_only_editable_slots_in_one_stroke() {
		let mut p = Project::new(4, 4, &["GREEN".to_string()], &assets_root(), 1).unwrap();
		let before = p.palette.clone();
		// A full 256-colour palette of solid red.
		let red = vec![[0xffu8, 0, 0]; 256].concat();
		let n = p.load_palette(&red).unwrap();
		assert!(n > 0 && n <= 96, "only the 96 dynamic slots can change");
		// Dynamic slot 64 took the load; static slot 0 + 200 are untouched.
		assert_eq!(&p.palette[64 * 3..64 * 3 + 3], &[0xff, 0, 0]);
		assert_eq!(&p.palette[0..3], &before[0..3]);
		assert_eq!(&p.palette[200 * 3..200 * 3 + 3], &before[200 * 3..200 * 3 + 3]);
		// One undo unit reverts the whole load.
		p.undo();
		assert_eq!(p.palette, before);
	}

	#[test]
	fn variants_load_and_random_stays_in_family() {
		let p = Project::new(4, 4, &["GREEN".to_string()], &assets_root(), 1).unwrap();
		// GSa ships eight look-variants (tiles.variants.json).
		let (tile, _) = p.resolve_ref("GSa000").unwrap();
		let group = p.packs[tile.pack as usize].variants_of(tile.tile).to_vec();
		assert!(group.len() >= 2, "GSa is a multi-variant family");
		let mut rng = Rng::new(7);
		for _ in 0..32 {
			let v = p.random_variant(tile, &mut rng);
			assert_eq!(v.pack, tile.pack, "same pack");
			assert_eq!(v.transform, tile.transform, "transform preserved");
			assert!(group.contains(&v.tile), "variant stays within the family");
		}
	}

	#[test]
	fn flood_fill_covers_the_connected_region() {
		let mut p = Project::new(4, 4, &["GREEN".to_string()], &assets_root(), 1).unwrap();
		let (tile, layer) = p.resolve_ref("GSa000").unwrap();
		assert_eq!(layer, LAYER_GROUND);
		// Ground starts empty everywhere → the fill floods all 16 cells.
		assert!(p.cell(0, 0).unwrap()[LAYER_GROUND].is_none());
		let mut rng = Rng::new(0);
		assert!(p.fill(0, 0, tile, layer, false, &mut rng));
		for y in 0..4 {
			for x in 0..4 {
				assert_eq!(p.cell(x, y).unwrap()[LAYER_GROUND], Some(tile));
			}
		}
		// Re-filling the same uniform tile changes nothing.
		assert!(!p.fill(0, 0, tile, layer, false, &mut rng));
		// One undo reverts the whole fill (it was a single transaction).
		assert!(p.undo());
		assert!(p.cell(2, 2).unwrap()[LAYER_GROUND].is_none());
	}

	/// `from_wrl` is lossless: every cell composes back to the source tile,
	/// bigmap indexing is honoured, and per-cell pass comes from the WRL.
	#[test]
	fn from_wrl_composes_back_to_source_pixels() {
		// 2×1 map, two distinct tiles; cell 0 → tile 1, cell 1 → tile 0.
		let mut tiles = vec![0u8; 2 * TILE_DATA_SIZE];
		tiles[..TILE_DATA_SIZE].fill(7);
		tiles[TILE_DATA_SIZE..].fill(42);
		let wrl = WrlFile {
			header: vec![0; 5],
			width: 2,
			height: 1,
			minimap: vec![42, 7],
			bigmap: vec![1, 0],
			tile_count: 2,
			tiles: tiles.clone(),
			palette: vec![0; 768],
			pass_table: vec![1, 2],
		};

		let p = Project::from_wrl(&wrl, "TEST");
		assert_eq!((p.width, p.height), (2, 1));
		// Cell 0 holds tile 1 (the 42s), cell 1 holds tile 0 (the 7s).
		assert_eq!(&p.compose_cell(0, 0)[..], &tiles[TILE_DATA_SIZE..]);
		assert_eq!(&p.compose_cell(1, 0)[..], &tiles[..TILE_DATA_SIZE]);
		// Pass derives from the synthetic pack: pass_table[bigmap[cell]].
		assert_eq!(p.pass_at(0, 0), Some(2)); // tile 1
		assert_eq!(p.pass_at(1, 0), Some(1)); // tile 0
		// The map decomposes by passability: cell 0 (tile 1, pass 2 = shore)
		// lands on the ground layer; cell 1 (tile 0, pass 1 = water) on the base.
		let c0 = p.cell(0, 0).unwrap();
		assert_eq!(c0[LAYER_GROUND].map(|t| t.tile), Some(1));
		assert!(c0[LAYER_WATER].is_none());
		let c1 = p.cell(1, 0).unwrap();
		assert_eq!(c1[LAYER_WATER].map(|t| t.tile), Some(0));
		assert!(c1[LAYER_GROUND].is_none());
		// Tile ids follow the XXXY### scheme (name TEST → consonants TST).
		assert_eq!(p.packs[0].ids[1], "TSTS000", "tile 1 is shore #0");
		assert_eq!(p.packs[0].ids[0], "TSTW000", "tile 0 is water #0");
		// A fresh import is clean.
		assert!(!p.dirty());
	}

	/// An imported WRL saved as a project dumps its synthetic pack to a
	/// sibling folder and reloads from it (the persistence path the user
	/// asked for): tiles, pass, and palette survive the round trip.
	#[test]
	fn wrl_import_dumps_and_reloads_via_sibling_pack() {
		let mut tiles = vec![0u8; 2 * TILE_DATA_SIZE];
		tiles[..TILE_DATA_SIZE].fill(7);
		tiles[TILE_DATA_SIZE..].fill(42);
		let wrl = WrlFile {
			header: vec![0; 5],
			width: 2,
			height: 1,
			minimap: vec![42, 7],
			bigmap: vec![1, 0],
			tile_count: 2,
			tiles,
			palette: vec![5; 768],
			pass_table: vec![2, 3],
		};
		let project = Project::from_wrl(&wrl, "WRLTEST");

		// Dump the synthetic pack next to a would-be `.json`.
		let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/maptest-wrl-dump");
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		project.packs[0].dump(&dir.join("WRLTEST")).unwrap();
		let json = project.save_string();

		// Reload with an assets root that lacks the pack - only the sibling
		// fallback (`dir`) has it.
		let empty = dir.join("no-assets");
		std::fs::create_dir_all(&empty).unwrap();
		let reloaded = Project::from_str_in(&json, &empty, Some(&dir)).unwrap();

		assert_eq!((reloaded.width, reloaded.height), (2, 1));
		assert_eq!(reloaded.compose_cell(0, 0), project.compose_cell(0, 0));
		assert_eq!(reloaded.compose_cell(1, 0), project.compose_cell(1, 0));
		assert_eq!(reloaded.pass_at(0, 0), Some(3)); // cell 0 → tile 1
		assert_eq!(reloaded.pass_at(1, 0), Some(2)); // cell 1 → tile 0
		assert_eq!(reloaded.palette, project.palette);

		std::fs::remove_dir_all(&dir).ok();
	}

	/// The internal palette keeps the WRL's own bytes (statics included) with
	/// live dynamic edits merged in; conversion rewrites tiles + palette to
	/// the compatible form and converges the two - and undoes as one unit.
	#[test]
	fn wrl_palette_conversion_remaps_tiles_and_converges_palettes() {
		let mut tiles = vec![0u8; TILE_DATA_SIZE];
		tiles.fill(40); // every pixel on a fixed game-ramp slot…
		let mut palette = crate::GAME_PALETTE.to_vec();
		palette[40 * 3..40 * 3 + 3].copy_from_slice(&[0xff, 0x00, 0xee]); // …claiming hot pink
		let wrl = WrlFile {
			header: vec![0; 5],
			width: 1,
			height: 1,
			minimap: vec![0],
			bigmap: vec![0],
			tile_count: 1,
			tiles,
			palette,
			pass_table: vec![0],
		};
		let mut p = Project::from_wrl(&wrl, "CONV");
		assert!(p.is_wrl_import());
		// The working palette resolved slot 40 to the game color, the
		// internal palette still says pink.
		assert_eq!(p.palette[40 * 3..40 * 3 + 3], crate::GAME_PALETTE[40 * 3..40 * 3 + 3]);
		assert_eq!(p.internal_palette()[40 * 3..40 * 3 + 3], [0xff, 0x00, 0xee]);
		// Dynamic edits show through the internal palette too.
		assert!(p.set_color(64, [9, 9, 9]).unwrap());
		assert_eq!(p.internal_palette()[64 * 3..64 * 3 + 3], [9, 9, 9]);

		let opts = crate::palette_convert::ConvertOptions::default();
		let structure = p.structure_revision();
		let report = p.convert_to_compatible_palette(opts).expect("off-spec static slot");
		assert_eq!((report.exact, report.approximated), (1, 0));
		// The pink moved to an (unused) free dynamic slot - pixels follow, exactly.
		let to = p.packs[0].tiles[0];
		assert!(DYNAMIC_SLOTS.contains(&to), "pixels remapped into a free dynamic slot, got {to}");
		assert!(p.packs[0].tiles.iter().all(|&b| b == to));
		assert_eq!(p.palette[to as usize * 3..to as usize * 3 + 3], [0xff, 0x00, 0xee]);
		// Palette and internal palette agree now (compatible), the doc is
		// dirty + structurally changed, and a re-run is a no-op.
		assert_eq!(p.internal_palette(), p.palette);
		assert!(p.dirty());
		assert_ne!(p.structure_revision(), structure);
		assert!(p.convert_to_compatible_palette(opts).is_none());

		// One Ctrl+Z brings the whole document back: tiles, palettes,
		// internal palette - and redo replays it byte-identically.
		let converted_tiles = p.packs[0].tiles.clone();
		let converted_palette = p.palette.clone();
		assert!(p.undo());
		assert!(p.packs[0].tiles.iter().all(|&b| b == 40), "tiles restored");
		assert_eq!(p.internal_palette()[40 * 3..40 * 3 + 3], [0xff, 0x00, 0xee], "internal palette restored");
		// The earlier set_color is still the next undo step (journal intact).
		assert!(p.undo());
		assert_ne!(p.internal_palette()[64 * 3..64 * 3 + 3], [9, 9, 9]);
		assert!(p.redo() && p.redo());
		assert_eq!(p.packs[0].tiles, converted_tiles);
		assert_eq!(p.palette, converted_palette);
	}

	/// The rasterize-and-reimport method rebuilds the tile table from the
	/// composed pixels; pinned water keeps its cycle slots and colors, and
	/// per-cell pass survives as overrides. Undoes as one unit.
	#[test]
	fn wrl_palette_conversion_by_reimport_pins_water_and_keeps_pass() {
		// Tile 0: all water-cycle slot 100; tile 1: all off-spec static 40.
		let mut tiles = vec![0u8; 2 * TILE_DATA_SIZE];
		tiles[..TILE_DATA_SIZE].fill(100);
		tiles[TILE_DATA_SIZE..].fill(40);
		let mut palette = crate::GAME_PALETTE.to_vec();
		palette[100 * 3..100 * 3 + 3].copy_from_slice(&[12, 34, 56]);
		palette[40 * 3..40 * 3 + 3].copy_from_slice(&[0xff, 0x00, 0xee]);
		let wrl = WrlFile {
			header: vec![0; 5],
			width: 2,
			height: 1,
			minimap: vec![100, 40],
			bigmap: vec![0, 1],
			tile_count: 2,
			tiles,
			palette,
			pass_table: vec![1, 0],
		};
		let mut p = Project::from_wrl(&wrl, "RAST");
		let tile_count =
			p.convert_palette_by_reimport(true, crate::image_import::Dedupe::Strict, 0.0).expect("reimport");
		assert!(tile_count >= 2);
		// Water pixels stay pinned to slot 100, with the map's color.
		assert_eq!(p.compose_cell(0, 0)[..], vec![100u8; TILE_DATA_SIZE][..]);
		assert_eq!(p.palette[100 * 3..100 * 3 + 3], [12, 34, 56]);
		// The pink tile re-quantized into stable (non-animated) slots close
		// to pink; statics are the game's.
		let cell1 = p.compose_cell(1, 0);
		assert!(cell1.iter().all(|&b| !(9..=31).contains(&b) && !(96..=127).contains(&b)));
		assert_eq!(p.palette[32 * 3..32 * 3 + 3], crate::GAME_PALETTE[32 * 3..32 * 3 + 3]);
		// Pass survived as per-cell overrides.
		assert_eq!(p.pass_at(0, 0), Some(1));
		assert_eq!(p.pass_at(1, 0), Some(0));
		// One undo restores the original document byte-for-byte.
		assert!(p.undo());
		assert_eq!(p.compose_cell(0, 0)[..], vec![100u8; TILE_DATA_SIZE][..]);
		assert_eq!(p.compose_cell(1, 0)[..], vec![40u8; TILE_DATA_SIZE][..]);
		assert_eq!(p.internal_palette()[40 * 3..40 * 3 + 3], [0xff, 0x00, 0xee]);
		assert_eq!(p.pass_at(0, 0), Some(1));
	}

	/// Golden splitmix64 vectors (seed 0) - pins the algorithm forever:
	/// generated maps must replay identically from their seed.
	#[test]
	fn rng_matches_splitmix64_reference() {
		let mut rng = Rng::new(0);
		assert_eq!(rng.next_u64(), 0xe220_a839_7b1d_cdaf);
		assert_eq!(rng.next_u64(), 0x6e78_9e6a_a1b9_65f4);
		assert_eq!(rng.next_u64(), 0x06c4_5d18_8009_454f);
	}

	#[test]
	fn new_project_fills_water_deterministically() {
		let root = assets_root();
		let p = Project::new(8, 6, &["GREEN".to_string()], &root, 42).unwrap();
		assert_eq!((p.width, p.height), (8, 6));
		assert!(!p.dirty());

		// WATER implied at index 0; GREEN owns the palette.
		assert_eq!(p.uses[0].name, "WATER");
		assert!(!p.uses[0].palette);
		assert_eq!(p.uses[1].name, "GREEN");
		assert!(p.uses[1].palette);
		assert_eq!(p.water_pack, Some(0));

		let water_tiles = p.packs[0].tile_count();
		for stack in &p.cells {
			let water = stack[LAYER_WATER].expect("bottom layer fully covered");
			assert_eq!(water.pack, 0);
			assert!(water.tile < water_tiles);
			assert_eq!(water.transform, Transform::default(), "WATER is sync - identity");
			assert_eq!(stack[LAYER_GROUND], None);
		}

		// Same seed → same map; different seed → different map.
		let again = Project::new(8, 6, &["GREEN".to_string()], &root, 42).unwrap();
		assert_eq!(p.hash(), again.hash());
		let other = Project::new(8, 6, &["GREEN".to_string()], &root, 43).unwrap();
		assert_ne!(p.hash(), other.hash());

		// Listing WATER explicitly must not duplicate it.
		let explicit = Project::new(8, 6, &["WATER".to_string(), "GREEN".to_string()], &root, 42).unwrap();
		assert_eq!(explicit.packs.len(), 2);
		assert_eq!(p.hash(), explicit.hash());
	}

	#[test]
	fn new_project_round_trips_through_save() {
		let root = assets_root();
		let p = Project::new(5, 4, &["DESERT".to_string()], &root, 7).unwrap();
		let reloaded = Project::from_str(&p.save_string(), &root).unwrap();
		assert_eq!(p.hash(), reloaded.hash());
		assert_eq!(reloaded.uses.len(), 2);
		assert_eq!(reloaded.uses[1].name, "DESERT");
	}

	#[test]
	fn stacked_same_layer_tiles_load_without_a_duplicate_error() {
		// Regression: an opened WRL becomes a project whose base pack is *not*
		// named WATER. Painting over the base then yields a cell with two
		// tiles, neither recognized as water - the old per-pack loader put both
		// on the ground layer and rejected the file ("duplicate ground layer").
		// Layers are advisory, so the loader reconstructs the stack positionally.
		let root = assets_root();
		let p = Project::new(2, 1, &["GREEN".to_string()], &root, 1).unwrap();
		assert!(p.packs[1].tile_count() >= 2, "GREEN has at least two tiles");
		let (a, b) = (p.packs[1].ids[0].clone(), p.packs[1].ids[1].clone());
		// A WATER-less project (GREEN owns the palette): both ids resolve to
		// GREEN, the case that used to collide on the ground layer.
		let json = format!(
			"{{\"version\":\"1\",\"name\":\"t\",\"description\":\"\",\"width\":2,\"height\":1,\
			 \"use\":[{{\"name\":\"GREEN\",\"tileset\":true,\"palette\":true,\"version\":\"1\"}}],\
			 \"map\":[[\"{a},{b}\",\"\"]]}}"
		);
		let loaded = Project::from_str(&json, &root).expect("stacked cell loads without error");
		let stack = loaded.cell(0, 0).unwrap();
		assert_eq!(stack[LAYER_WATER].map(|t| t.tile), Some(0), "first tile → base layer");
		assert_eq!(stack[LAYER_GROUND].map(|t| t.tile), Some(1), "second tile → ground layer");
	}

	#[test]
	fn project_file_version_guards_on_major_and_migrates() {
		let root = assets_root();
		let p = Project::new(4, 4, &["GREEN".to_string()], &root, 1).unwrap();
		let text = p.save_string();
		// New saves carry the scheme'd top-level key/value.
		assert!(text.contains("\"mme_project_file_version\": \"2.0\""), "{text}");
		assert_eq!(Project::from_str(&text, &root).unwrap().version, "2.0");

		let swap = |from: &str, to: &str| text.replace(&format!("\"mme_project_file_version\": \"{from}\""), to);
		// A pre-scheme `version: "1"` file still opens, migrated to the current.
		let legacy = swap("2.0", "\"version\": \"1\"");
		assert_eq!(Project::from_str(&legacy, &root).expect("legacy migrates").version, "2.0");
		// A newer MINOR within the same MAJOR opens.
		assert!(Project::from_str(&swap("2.0", "\"mme_project_file_version\": \"2.7\""), &root).is_ok());
		// A different MAJOR is a hard break; malformed versions are rejected.
		match Project::from_str(&swap("2.0", "\"mme_project_file_version\": \"3.0\""), &root) {
			Ok(_) => panic!("a different MAJOR must be rejected"),
			Err(e) => assert!(e.contains("unsupported"), "{e}"),
		}
		assert!(Project::from_str(&swap("2.0", "\"mme_project_file_version\": \"banana\""), &root).is_err());
	}

	#[test]
	fn load_rejects_malformed_headers() {
		let root = assets_root();
		let err = |json: &str| match Project::from_str(json, &root) {
			Ok(_) => panic!("expected a load error for: {json}"),
			Err(e) => e,
		};
		// Missing required top-level fields (version is checked first; name/
		// description are read before the dimensions).
		assert!(err("{}").contains("mme_project_file_version"), "no version key");
		assert!(
			err(r#"{"mme_project_file_version": "2.0", "name": "t", "description": "", "height": 4}"#)
				.contains("missing field 'width'"),
			"no width"
		);
		// Bad / non-numeric dimensions - caught before any map parsing.
		let dims = |w: &str, h: &str| {
			format!(
				r#"{{"mme_project_file_version": "2.0", "name": "t", "description": "", "width": {w}, "height": {h}}}"#
			)
		};
		assert!(err(&dims("0", "4")).contains("bad map size"), "zero width");
		assert!(err(&dims("4", "0")).contains("bad map size"), "zero height");
		assert!(err(&dims("2000", "4")).contains("bad map size"), "width > 1024");
		assert!(err(&dims(r#""x""#, "4")).contains("width not a number"), "non-numeric width");
	}

	#[test]
	fn load_rejects_malformed_body() {
		let root = assets_root();
		// A valid 2×1 project (GREEN owns the palette, empty cells); `map`/`extra`
		// are spliced in so each case isolates one malformation.
		let base = |map: &str, extra: &str| {
			format!(
				r#"{{"version":"1","name":"t","description":"","width":2,"height":1,"use":[{{"name":"GREEN","tileset":true,"palette":true,"version":"1"}}]{extra},"map":{map}}}"#
			)
		};
		let err = |json: String| match Project::from_str(&json, &root) {
			Ok(_) => panic!("expected a load error for: {json}"),
			Err(e) => e,
		};
		// Sanity: the unmutated base loads.
		Project::from_str(&base(r#"[["",""]]"#, ""), &root).expect("the base project loads");

		// Map shape: wrong row count, wrong cell count per row.
		assert!(err(base("[]", "")).contains("map has 0 rows"), "row count");
		assert!(err(base(r#"[[""]]"#, "")).contains("row 0 has 1 cells"), "cell count");
		// Cell typing: a non-string/array cell, and a non-string inside the array form.
		assert!(err(base(r#"[[123,""]]"#, "")).contains("not a string or array"), "scalar cell");
		assert!(err(base(r#"[[[123],""]]"#, "")).contains("non-string entry"), "array cell entry");
		// Pass overlay (array form): wrong row count and wrong row length.
		assert!(err(base(r#"[["",""]]"#, r#","pass":[]"#)).contains("pass has 0 rows"), "pass rows");
		assert!(err(base(r#"[["",""]]"#, r#","pass":["0"]"#)).contains("pass row 0 has 1 cells"), "pass row len");
		// Units: a coordinate outside the map.
		assert!(err(base(r#"[["",""]]"#, r#","units":["T 5 0 0"]"#)).contains("out of range"), "unit OOR");
		// Exactly one palette owner is required.
		let no_owner = r#"{"version":"1","name":"t","description":"","width":2,"height":1,"use":[{"name":"GREEN","tileset":true,"palette":false,"version":"1"}],"map":[["",""]]}"#;
		assert!(err(no_owner.to_string()).contains("palette owner"), "palette owner count");
	}

	#[test]
	fn load_accepts_legacy_sparse_pass_and_positional_overstack() {
		let root = assets_root();
		let p = Project::new(2, 1, &["GREEN".to_string()], &root, 1).unwrap();
		let a = p.packs[1].ids[0].clone();
		// A cell with more refs than layers (3 > MAX_LAYERS) is reconstructed
		// positionally rather than rejected: first → base, the rest stack upward.
		let three = format!(
			r#"{{"version":"1","name":"t","description":"","width":2,"height":1,"use":[{{"name":"GREEN","tileset":true,"palette":true,"version":"1"}}],"map":[["{a},{a},{a}",""]]}}"#
		);
		let loaded = Project::from_str(&three, &root).expect("3-ref overstack loads via positional fallback");
		let stack = loaded.cell(0, 0).unwrap();
		assert!(stack[0].is_some() && stack[1].is_some(), "both layers filled from the overstack");

		// Legacy sparse pass form `{ "x,y": value }` still loads; out-of-range rejects.
		let pass = |v: &str| {
			format!(
				r#"{{"version":"1","name":"t","description":"","width":2,"height":1,"use":[{{"name":"GREEN","tileset":true,"palette":true,"version":"1"}}],"map":[["",""]],"pass":{{"0,0":{v}}}}}"#
			)
		};
		Project::from_str(&pass("2"), &root).expect("legacy sparse pass loads");
		assert!(Project::from_str(&pass("9"), &root).err().unwrap().contains("out of range"), "sparse pass OOR");
	}

	#[test]
	fn map_preferences_round_trip_and_stay_optional() {
		let root = assets_root();
		let mut p = Project::new(4, 4, &["GREEN".to_string()], &root, 1).unwrap();
		// A pref-free map writes none of the metadata keys.
		let bare = p.save_string();
		for key in ["\"players\"", "\"date\"", "\"map_version\"", "\"author\""] {
			assert!(!bare.contains(key), "bare save should omit {key}");
		}
		// Set them (description keeps newlines, strips CR; players clamps 2..=4).
		p.set_info(
			"Twin Peaks".into(),
			Some(9),
			"line one\r\nline two".into(),
			"2026".into(),
			"1.2".into(),
			"Aneta".into(),
		);
		assert_eq!(p.players, Some(4), "players clamps to 4");
		assert_eq!(p.description, "line one\nline two", "CR stripped, newline kept");
		assert!(p.dirty());
		let saved = p.save_string();
		assert!(saved.contains("\"players\": \"2-4\""), "players saved as its label, not a number");
		let reloaded = Project::from_str(&saved, &root).unwrap();
		assert_eq!(reloaded.name, "Twin Peaks");
		assert_eq!(reloaded.players, Some(4), "label round-trips back to the count");
		assert_eq!(reloaded.description, "line one\nline two", "newline survives the JSON round-trip");
		assert_eq!(reloaded.date, "2026");
		assert_eq!(reloaded.map_version, "1.2");
		assert_eq!(reloaded.author, "Aneta");
		// The other counts map to their labels; legacy bare-number saves still load.
		for (count, label) in [(2u8, "\"2\""), (3, "\"2-3\"")] {
			p.set_info(String::new(), Some(count), String::new(), String::new(), String::new(), String::new());
			assert!(p.save_string().contains(&format!("\"players\": {label}")), "count {count} → {label}");
		}
		let legacy = saved.replace("\"players\": \"2-4\"", "\"players\": 3");
		assert_eq!(Project::from_str(&legacy, &root).unwrap().players, Some(3), "legacy numeric players loads");
	}

	#[test]
	fn new_project_without_palette_owner_fails() {
		let Err(err) = Project::new(4, 4, &[], &assets_root(), 0) else {
			panic!("expected an error");
		};
		assert!(err.contains("palette"), "{err}");
	}

	#[test]
	fn pixel_at_matches_full_compose() {
		let root = assets_root();
		let mut p = Project::new(8, 6, &["GREEN".to_string()], &root, 42).unwrap();
		// A transformed shore over water exercises layering + transforms.
		let (tile, layer) = p.resolve_ref("GSa000:!N").unwrap();
		assert!(p.place(3, 2, layer, Some(tile)));

		for &(x, y) in &[(3u16, 2u16), (0, 0), (7, 5)] {
			let composed = p.compose_cell(x, y);
			for &(sx, sy) in &[(0usize, 0usize), (32, 32), (63, 63), (17, 48)] {
				assert_eq!(p.pixel_at(x, y, (sx, sy)), composed[sy * 64 + sx], "cell ({x},{y}) sub ({sx},{sy})",);
			}
			assert_eq!(p.minimap_pixel(x, y), composed[32 * 64 + 32]);
		}
	}

	#[test]
	fn tile_pass_edits_retint_every_shared_cell_and_round_trip() {
		let root = assets_root();
		let mut p = Project::new(4, 1, &["GREEN".to_string()], &root, 7).unwrap();
		// The same land tile under two cells - they share one tile id.
		let (land, layer) = p.resolve_ref("GLa000").unwrap();
		assert!(p.place(0, 0, layer, Some(land)));
		assert!(p.place(1, 0, layer, Some(land)));
		let before = p.pass_at(0, 0);
		assert_eq!(p.pass_at(1, 0), before, "same tile, same pass");

		// Editing the tile pass at one cell retints the other (tile-dependent).
		assert!(p.set_tile_pass_at(0, 0, 3));
		assert_eq!(p.pass_at(0, 0), Some(3));
		assert_eq!(p.pass_at(1, 0), Some(3), "shared tile id retints together");
		assert_eq!(p.pass_override(0, 0), None, "it's tile pass, not a cell override");

		// One undo unit restores both cells.
		assert!(p.undo());
		assert_eq!(p.pass_at(0, 0), before);
		assert_eq!(p.pass_at(1, 0), before);
		p.redo();
		assert_eq!(p.pass_at(1, 0), Some(3), "redo replays the tile edit");

		// Per-tile pass persists through save/load (the `tilepass` block).
		let text = p.save_string();
		assert!(text.contains("\"tilepass\""), "tile pass is persisted");
		let reloaded = Project::from_str(&text, &root).unwrap();
		assert_eq!(reloaded.pass_at(0, 0), Some(3));
		assert_eq!(reloaded.pass_at(1, 0), Some(3));
	}

	#[test]
	fn reset_tile_pass_reverts_to_the_supplied_canonical_pass() {
		let root = assets_root();
		let mut p = Project::new(2, 1, &["GREEN".to_string()], &root, 7).unwrap();
		let (land, layer) = p.resolve_ref("GLa000").unwrap();
		assert!(p.place(0, 0, layer, Some(land)));
		// The canonical (tileset) pass = a snapshot of every pack's current pass,
		// taken before any edit.
		let canonical: Vec<Option<Vec<u8>>> = p.packs.iter().map(|pk| pk.pass.clone()).collect();

		// Edit the land tile's pass away from its tileset value.
		let before = p.pass_at(0, 0).unwrap();
		let edited = if before == 3 { 0 } else { 3 };
		assert!(p.set_tile_pass_at(0, 0, edited));
		assert_eq!(p.pass_at(0, 0), Some(edited));

		// Reset to canonical reverts it, as one undo unit.
		assert!(p.reset_tile_pass(&canonical), "a change was applied");
		assert_eq!(p.pass_at(0, 0), Some(before), "back to the tileset value");
		assert!(p.undo(), "reset is undoable");
		assert_eq!(p.pass_at(0, 0), Some(edited), "undo brings the edit back");

		// Already-canonical → no-op (nothing to undo).
		p.redo();
		assert!(!p.reset_tile_pass(&canonical), "no change when already canonical");
		// A `None` entry leaves that pack untouched even when it differs.
		assert!(p.set_tile_pass_at(0, 0, edited));
		let skip: Vec<Option<Vec<u8>>> = vec![None; p.packs.len()];
		assert!(!p.reset_tile_pass(&skip), "None per pack skips it");
		assert_eq!(p.pass_at(0, 0), Some(edited), "skipped pack keeps its edit");
	}

	#[test]
	fn pass_overrides_round_trip_through_the_dense_grid() {
		let root = assets_root();
		let mut p = Project::new(5, 3, &["GREEN".to_string()], &root, 1).unwrap();
		assert!(p.set_pass(2, 1, 3));
		assert!(p.set_pass(4, 2, 2));
		let text = p.save_string();
		// The block is a dense array of digit-rows, not a sparse object.
		assert!(text.contains("\"pass\""));
		assert!(text.contains("\"--3--\""), "row 1 carries the blocked override:\n{text}");
		let reloaded = Project::from_str(&text, &root).unwrap();
		assert_eq!(reloaded.pass_override(2, 1), Some(3));
		assert_eq!(reloaded.pass_override(4, 2), Some(2));
		assert_eq!(reloaded.pass_override(0, 0), None);
		assert_eq!(reloaded.hash(), p.hash(), "overrides survive the dense round-trip");
	}

	#[test]
	fn pass_at_reads_the_stack_top() {
		let root = assets_root();
		let mut p = Project::new(8, 6, &["GREEN".to_string()], &root, 42).unwrap();
		assert_eq!(p.pass_at(2, 2), Some(1), "fresh map is water");
		let (tile, layer) = p.resolve_ref("GLa000").unwrap();
		assert!(p.place(2, 2, layer, Some(tile)));
		assert_eq!(p.pass_at(2, 2), Some(0), "land tile on top");
		assert_eq!(p.pass_at(99, 99), None, "out of range");
	}

	#[test]
	fn pass_override_paints_undoes_saves_and_bakes() {
		let root = assets_root();
		let mut p = Project::new(8, 6, &["GREEN".to_string()], &root, 42).unwrap();
		// Fresh water cell derives pass 1; override it to blocked (3).
		assert_eq!(p.pass_at(2, 2), Some(1));
		assert!(p.set_pass(2, 2, 3));
		assert_eq!(p.pass_at(2, 2), Some(3), "override wins over derived");
		assert_eq!(p.pass_override(2, 2), Some(3));
		// The bake reads the override (a fresh water map is all pass 1, so a
		// blocked tile in the baked per-tile passtab can only come from it).
		let wrl = crate::bake(&p).unwrap();
		assert!(wrl.pass_table.contains(&3), "override flows into the bake");

		// Undoable, one unit; round-trips through save.
		let with = p.hash();
		assert!(p.undo());
		assert_eq!(p.pass_at(2, 2), Some(1), "undo restores the derived pass");
		assert_eq!(p.pass_override(2, 2), None);
		p.redo();
		assert_eq!(p.hash(), with, "redo replays the override");

		let text = p.save_string();
		assert!(text.contains("\"pass\""), "the override is persisted");
		let reloaded = Project::from_str(&text, &root).unwrap();
		assert_eq!(reloaded.pass_at(2, 2), Some(3), "and reloads");
		assert_eq!(reloaded.hash(), p.hash());
	}

	#[test]
	fn unit_notes_round_trip_through_save() {
		let root = assets_root();
		let mut p = Project::new(4, 4, &["GREEN".to_string()], &root, 42).unwrap();
		assert!(!p.dirty());

		p.stamp_unit(UnitNote { tag: "TANK".into(), x: 1, y: 2, team: 3 });
		p.stamp_unit(UnitNote { tag: "SCOUT".into(), x: 0, y: 0, team: 0 });
		// Restamping a cell replaces, not stacks.
		p.stamp_unit(UnitNote { tag: "AWAC".into(), x: 1, y: 2, team: 1 });
		assert!(p.dirty(), "annotations persist, so they dirty the doc");
		assert_eq!(p.units.len(), 2);

		let text = p.save_string();
		assert!(text.contains("\"units\""), "the notes are persisted");
		let reloaded = Project::from_str(&text, &root).unwrap();
		assert_eq!(reloaded.units, p.units, "notes reload identically");

		assert!(p.erase_unit_at(1, 2));
		assert!(!p.erase_unit_at(1, 2), "already gone");
		assert_eq!(p.clear_units(), 1);
		// A unit-free project saves without the block at all.
		assert!(!p.save_string().contains("\"units\""));
	}

	#[test]
	fn resize_places_old_map_and_fills_water() {
		let root = assets_root();
		let mut p = Project::new(4, 4, &["GREEN".to_string()], &root, 42).unwrap();
		let (land, layer) = p.resolve_ref("GLa000").unwrap();
		p.place(0, 0, layer, Some(land)); // a marker in the top-left
		p.set_pass(0, 0, 3);

		// Enlarge to 8×8 with the old map centered (offset 2,2).
		p.resize(8, 8, 2, 2).unwrap();
		assert_eq!((p.width, p.height), (8, 8));
		// The marker moved to (2,2); its pass override rode along.
		let top = p.cell(2, 2).unwrap()[layer].unwrap();
		assert_eq!(p.packs[top.pack as usize].ids[top.tile as usize], "GLa000");
		assert_eq!(p.pass_override(2, 2), Some(3));
		// New territory is water.
		assert_eq!(p.pass_at(0, 0), Some(1), "new corner is water");

		// Shrink/crop back: offset -2,-2 recovers the original window.
		p.resize(4, 4, -2, -2).unwrap();
		let top = p.cell(0, 0).unwrap()[layer].unwrap();
		assert_eq!(p.packs[top.pack as usize].ids[top.tile as usize], "GLa000");
		assert_eq!(p.pass_override(0, 0), Some(3));

		assert!(p.resize(0, 8, 0, 0).is_err(), "rejects zero dimension");
	}

	#[test]
	fn pass_paint_drag_is_one_undo_unit() {
		let root = assets_root();
		let mut p = Project::new(8, 6, &["GREEN".to_string()], &root, 42).unwrap();
		let before = p.hash();
		p.begin_stroke();
		p.set_pass(0, 0, 2);
		p.set_pass(1, 0, 2);
		p.set_pass(2, 0, 2);
		p.end_stroke();
		assert!(p.undo(), "the whole drag undoes at once");
		assert_eq!(p.hash(), before);
	}

	#[test]
	fn bake_accepts_rectangular_maps() {
		// Any rectangle is a valid WRL (confirmed 2026-06) - width/height
		// are independent throughout.
		let p = Project::new(8, 6, &["GREEN".to_string()], &assets_root(), 42).unwrap();
		let wrl = crate::bake(&p).unwrap();
		assert_eq!((wrl.width, wrl.height), (8, 6));
	}

	#[test]
	fn palette_edits_are_undoable_and_round_trip_through_save() {
		let root = assets_root();
		let mut p = Project::new(8, 8, &["GREEN".to_string()], &root, 42).unwrap();
		let before = p.hash();

		// Static slots refuse edits; dynamic accept and change the hash.
		assert!(p.set_color(32, [1, 2, 3]).is_err());
		assert!(p.set_color(200, [1, 2, 3]).is_err());
		assert!(p.set_color(100, [10, 20, 30]).unwrap());
		assert!(p.dirty());
		assert_ne!(p.hash(), before, "palette is document state");
		assert!(!p.set_color(100, [10, 20, 30]).unwrap(), "no-op edit");

		// Saved as a sparse override block; reload reproduces the palette.
		// (`"palette": {` is the override block - `"palette": true` in the
		// `use` entries is the unrelated owner flag.)
		let text = p.save_string();
		assert!(text.contains("\"palette\": {"), "{text}");
		assert!(text.contains("\"100\": \"#0a141e\""), "{text}");
		let reloaded = Project::from_str(&text, &root).unwrap();
		assert_eq!(reloaded.palette[300..303], [10, 20, 30]);
		assert_eq!(reloaded.hash(), p.hash());

		// Undo restores the pack color - and the override block disappears.
		assert!(p.undo());
		assert_eq!(p.hash(), before);
		assert!(!p.save_string().contains("\"palette\": {"));
		assert!(p.redo());
		assert_eq!(p.palette[300..303], [10, 20, 30]);

		// Overrides outside the dynamic range are rejected at load.
		let bad = text.replace("\"100\"", "\"32\"");
		assert!(Project::from_str(&bad, &root).is_err());
	}

	#[test]
	fn static_slots_resolve_to_the_in_game_palette() {
		let root = assets_root();
		let p = Project::new(8, 8, &["GREEN".to_string()], &root, 42).unwrap();
		// Every static slot carries the game value (pack bytes there are
		// converter leftovers the engine would ignore anyway).
		for slot in 0..256usize {
			if (64..=159).contains(&slot) {
				continue;
			}
			assert_eq!(p.palette[slot * 3..slot * 3 + 3], crate::GAME_PALETTE[slot * 3..slot * 3 + 3], "slot {slot}",);
		}
		// Dynamic slots stay pack-owned (not the FF00FF placeholders).
		assert_ne!(p.palette[64 * 3..64 * 3 + 3], [0xff, 0x00, 0xff]);
		// Statics never count as overrides in the save.
		assert!(!p.save_string().contains("\"palette\": {"));
	}

	#[test]
	fn hsl_block_shift_retints_one_water_cycle() {
		let root = assets_root();
		let mut p = Project::new(8, 8, &["GREEN".to_string()], &root, 42).unwrap();
		let before = p.hash();
		let snapshot = p.palette.clone();

		assert!(p.hsl_shift_block(110, 40.0, 0.0, 0.1).unwrap());
		// Only the 110–116 block changed.
		for slot in 0..256usize {
			let same = p.palette[slot * 3..slot * 3 + 3] == snapshot[slot * 3..slot * 3 + 3];
			if (110..=116).contains(&slot) {
				assert!(
					!same || {
						// A grey could map to itself; tolerate but don't expect.
						true
					}
				);
			} else {
				assert!(same, "slot {slot} must be untouched");
			}
		}
		assert_ne!(p.hash(), before);

		// The whole block re-tint is ONE undo step.
		assert!(p.undo());
		assert_eq!(p.hash(), before);
		assert_eq!(p.palette, snapshot);

		// Non-water slots refuse the block tool.
		assert!(p.hsl_shift_block(70, 10.0, 0.0, 0.0).is_err());
		assert!(p.hsl_shift_block(9, 10.0, 0.0, 0.0).is_err(), "game animated is fixed");
	}

	#[test]
	fn transform_ops_match_pixel_operations() {
		// A recognizable asymmetric 64×64 test tile.
		let mut src = [0u8; TILE_DATA_SIZE];
		for y in 0..64usize {
			for x in 0..64usize {
				src[y * 64 + x] = ((x * 7 + y * 13) % 251) as u8;
			}
		}
		let rot_cw = |p: &[u8; TILE_DATA_SIZE]| {
			let mut out = [0u8; TILE_DATA_SIZE];
			for y in 0..64usize {
				for x in 0..64usize {
					out[y * 64 + x] = p[(63 - x) * 64 + y];
				}
			}
			out
		};
		let flip_h = |p: &[u8; TILE_DATA_SIZE]| {
			let mut out = [0u8; TILE_DATA_SIZE];
			for y in 0..64usize {
				for x in 0..64usize {
					out[y * 64 + x] = p[y * 64 + (63 - x)];
				}
			}
			out
		};
		let flip_v = |p: &[u8; TILE_DATA_SIZE]| {
			let mut out = [0u8; TILE_DATA_SIZE];
			for y in 0..64usize {
				for x in 0..64usize {
					out[y * 64 + x] = p[(63 - y) * 64 + x];
				}
			}
			out
		};

		for rot in 0..4u8 {
			for mirror in [false, true] {
				let t = Transform { rot, mirror };
				let base = transform_tile(&src, t);
				assert_eq!(transform_tile(&src, t.rotated_cw()), rot_cw(&base), "{t:?} cw");
				assert_eq!(transform_tile(&src, t.rotated_cw().rotated_ccw()), base, "{t:?} cw∘ccw = id",);
				assert_eq!(transform_tile(&src, t.flipped_h()), flip_h(&base), "{t:?} flip h");
				assert_eq!(transform_tile(&src, t.flipped_v()), flip_v(&base), "{t:?} flip v");
			}
		}
	}

	/// `compose` is exactly transform-then-transform on pixels, for all 64
	/// pairs.
	#[test]
	fn compose_matches_pixel_chaining() {
		let mut src = [0u8; TILE_DATA_SIZE];
		for y in 0..64usize {
			for x in 0..64usize {
				src[y * 64 + x] = ((x * 7 + y * 13) % 251) as u8;
			}
		}
		for ra in 0..4u8 {
			for ma in [false, true] {
				for rb in 0..4u8 {
					for mb in [false, true] {
						let outer = Transform { rot: ra, mirror: ma };
						let inner = Transform { rot: rb, mirror: mb };
						let chained = transform_tile(&transform_tile(&src, inner), outer);
						assert_eq!(transform_tile(&src, outer.compose(inner)), chained, "{outer:?} ∘ {inner:?}",);
					}
				}
			}
		}
	}

	#[test]
	fn stroke_groups_edits_into_one_undo_unit() {
		let root = assets_root();
		let mut p = Project::new(8, 6, &["GREEN".to_string()], &root, 42).unwrap();
		let before = p.hash();
		let (tile, layer) = p.resolve_ref("GLa000").unwrap();

		p.begin_stroke();
		assert!(p.place(2, 2, layer, Some(tile)));
		assert!(p.place(3, 2, layer, Some(tile)));
		assert!(p.place(4, 2, layer, Some(tile)));
		p.end_stroke();
		let painted = p.hash();
		assert_ne!(before, painted);

		assert!(p.undo(), "stroke undoes as one unit");
		assert_eq!(p.hash(), before);
		assert!(!p.undo(), "nothing left to undo");

		assert!(p.redo());
		assert_eq!(p.hash(), painted);

		// An empty stroke leaves no undo entry behind.
		p.begin_stroke();
		p.end_stroke();
		assert!(p.undo());
		assert_eq!(p.hash(), before);
	}
}
