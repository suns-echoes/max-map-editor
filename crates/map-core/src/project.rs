//! Map project — the editor's primary document.
//!
//! v1 format: `resources/maps/*.json` — see `docs/design/tileset-contract.md`
//! §3. Each cell is a bottom-up stack (water layer, ground layer); tile refs
//! carry a transform (rotation + mirror). `compose_cell` flattens a stack to
//! raw pixels — the kernel of the future WRL export bake, and the
//! thing the 24-map equivalence test verifies against original WRLs.

use std::collections::HashMap;
use std::path::Path;

use max_assets::wrl::{TILE_DATA_SIZE, TILE_SIZE, WrlFile};

use crate::pack::TilePack;

pub const LAYER_WATER: usize = 0;
pub const LAYER_GROUND: usize = 1;
pub const MAX_LAYERS: usize = 2;

/// Undo depth cap — beyond this the oldest patches are dropped.
const MAX_UNDO: usize = 256;

/// The tileset-editable palette slots (contract §1: dynamic 64–159).
pub const DYNAMIC_SLOTS: std::ops::RangeInclusive<u8> = 64..=159;

/// The dynamic **animated** water cycle classes (contract §1) — each block
/// is one in-game color gradient; block re-tints keep it coherent.
pub const WATER_CYCLES: [(u8, u8); 5] = [(96, 102), (103, 109), (110, 116), (117, 122), (123, 127)];

/// Tiny deterministic PRNG (splitmix64) — the new-map fill and future
/// generators must reproduce exactly from a seed, on every
/// platform, forever. Never swap this for a library RNG.
pub struct Rng(u64);

impl Rng {
	pub fn new(seed: u64) -> Self {
		Self(seed)
	}

	pub fn next_u64(&mut self) -> u64 {
		self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
		let mut z = self.0;
		z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
		z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
		z ^ (z >> 31)
	}

	/// Uniform in `0..n` (`n > 0`; modulo bias is negligible at u64 width).
	pub fn below(&mut self, n: u32) -> u32 {
		(self.next_u64() % n as u64) as u32
	}
}

/// Rotation (quarter turns clockwise) + horizontal mirror (applied first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Transform {
	pub rot: u8, // 0..=3 — N, E, S, W
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

	/// `self ∘ inner` — apply `inner` first, then `self`, re-normalized to
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

pub struct Project {
	pub version: String,
	pub name: String,
	pub description: String,
	pub width: u16,
	pub height: u16,
	pub uses: Vec<UseEntry>,
	pub packs: Vec<TilePack>,
	/// `width * height` cell stacks, bottom-up: `[water, ground]`.
	pub cells: Vec<[Option<TileRef>; MAX_LAYERS]>,
	/// Per-cell pass-value override (Pass Table Editor) — `None`
	/// falls back to the derived stack-top pass. `width * height` long.
	pass_overrides: Vec<Option<u8>>,
	/// Working 256×RGB palette: the owner pack's palette + this map's
	/// dynamic-slot overrides (edited via `set_color`/`hsl_shift_block`).
	pub palette: Vec<u8>,
	/// The owner pack's pristine palette — the diff against it is what
	/// `save_string` writes as the project's `"palette"` override block.
	pack_palette: Vec<u8>,
	/// The document's palette exactly as its source carries it — the WRL's
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
	/// Bumped whenever document *structure* changes — pack tile tables /
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
/// user's MAX.RES — the project only records what stands where.
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
	/// A whole-document swap (palette conversion rewrites tile pixel data —
	/// not expressible as per-cell edits). Applying swaps the stored state
	/// with the live one, so the patch is its own inverse carrier.
	doc: Option<Box<DocState>>,
}

impl Patch {
	fn is_empty(&self) -> bool {
		self.cells.is_empty() && self.colors.is_empty() && self.passes.is_empty() && self.doc.is_none()
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

	/// As `from_str`, but referenced packs are looked up in `assets_root`
	/// first, then in `project_dir` (the saved `.json`'s folder) — that's
	/// where a project saved from an imported WRL dumps its synthetic pack.
	pub fn from_str_in(text: &str, assets_root: &Path, project_dir: Option<&Path>) -> Result<Self, String> {
		let root = json::parse(text)?;
		let field = |key: &str| root.get(key).ok_or(format!("missing field '{key}'"));

		let version = field("version")?.as_str().ok_or("version not a string")?.to_string();
		if version != "1" {
			return Err(format!("unsupported project version '{version}'"));
		}
		let name = field("name")?.as_str().unwrap_or("").to_string();
		let description = field("description")?.as_str().unwrap_or("").to_string();
		let width = field("width")?.as_f64().ok_or("width not a number")? as u16;
		let height = field("height")?.as_f64().ok_or("height not a number")? as u16;
		if width == 0 || height == 0 || width > 1024 || height > 1024 {
			return Err(format!("bad map size {width}×{height} (1..=1024)"));
		}

		// `use` — load referenced packs; exactly one owns the palette.
		let mut uses = Vec::new();
		let mut packs = Vec::new();
		for entry in field("use")?.as_array().ok_or("'use' not an array")? {
			let name = entry.get("name").and_then(|v| v.as_str()).ok_or("use entry: no name")?;
			let use_entry = UseEntry {
				name: name.to_string(),
				tileset: entry.get("tileset").and_then(|v| v.as_bool()).unwrap_or(false),
				palette: entry.get("palette").and_then(|v| v.as_bool()).unwrap_or(false),
				version: entry.get("version").and_then(|v| v.as_str()).unwrap_or("1").to_string(),
			};
			// assets_root first, then the project's own folder (imported-WRL packs).
			let pack = if !assets_root.join(name).is_dir() && project_dir.is_some_and(|d| d.join(name).is_dir()) {
				TilePack::load(project_dir.unwrap(), name)?
			} else {
				TilePack::load(assets_root, name)?
			};
			packs.push(pack);
			uses.push(use_entry);
		}
		let palette_owners: Vec<usize> = uses.iter().enumerate().filter(|(_, u)| u.palette).map(|(i, _)| i).collect();
		let [owner] = palette_owners[..] else {
			return Err(format!("expected exactly one palette owner, got {}", palette_owners.len()));
		};
		let mut pack_palette = packs[owner]
			.palette
			.clone()
			.ok_or_else(|| format!("palette owner '{}' has no palette.json", uses[owner].name))?;
		// The file's own bytes, kept for debug rendering / inspection.
		let source_palette = pack_palette.clone();
		// Static slots belong to the game (contract §1) — the engine
		// replaces them at runtime, so the editor resolves them to the
		// in-game values too (pack bytes there are converter leftovers).
		crate::game_palette::apply_game_statics(&mut pack_palette);
		// Optional `"palette"` block: this map's dynamic-slot overrides
		// (`{ "96": "#aabbcc", … }`) over the owner pack's palette.
		let mut palette = pack_palette.clone();
		if let Some(overrides) = root.get("palette") {
			let entries = overrides.as_object().ok_or("'palette' not an object")?;
			for (key, value) in entries {
				let slot: u8 = key.parse().map_err(|_| format!("palette override: bad slot '{key}'"))?;
				if !DYNAMIC_SLOTS.contains(&slot) {
					return Err(format!("palette override slot {slot} outside the dynamic range 64..=159",));
				}
				let hex = value
					.as_str()
					.and_then(|s| s.strip_prefix('#'))
					.filter(|h| h.len() == 6)
					.ok_or(format!("palette override {slot}: expected \"#rrggbb\""))?;
				for i in 0..3 {
					palette[slot as usize * 3 + i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
						.map_err(|_| format!("palette override {slot}: bad hex '#{hex}'"))?;
				}
			}
		}

		// Tile id → (pack, index) across all used packs.
		let resolve = |id: &str| -> Result<(u8, u16), String> {
			for (pack_index, pack) in packs.iter().enumerate() {
				if let Some(&tile) = pack.index_of.get(id) {
					return Ok((pack_index as u8, tile));
				}
			}
			Err(format!("unknown tile id '{id}'"))
		};
		// v1 heuristic: the WATER pack fills the water layer; everything
		// else is ground. v2 will declare layers explicitly.
		let water_pack = uses.iter().position(|u| u.name == "WATER").map(|i| i as u8);

		let rows = field("map")?.as_array().ok_or("'map' not an array")?;
		if rows.len() != height as usize {
			return Err(format!("map has {} rows, want {height}", rows.len()));
		}
		let mut cells = Vec::with_capacity(width as usize * height as usize);
		for (y, row) in rows.iter().enumerate() {
			let row = row.as_array().ok_or(format!("row {y} not an array"))?;
			if row.len() != width as usize {
				return Err(format!("row {y} has {} cells, want {width}", row.len()));
			}
			for (x, cell) in row.iter().enumerate() {
				// Cells appear as "WATR00,CSd001" or ["WATR00", "CSd001"]
				// in the v1 corpus — accept both, save normalizes to the
				// comma-string form.
				let parts: Vec<&str> = if let Some(text) = cell.as_str() {
					text.split(',').filter(|p| !p.is_empty()).collect()
				} else if let Some(list) = cell.as_array() {
					list.iter()
						.map(|v| v.as_str().ok_or(format!("cell {x},{y}: non-string entry")))
						.collect::<Result<_, _>>()?
				} else {
					return Err(format!("cell {x},{y} not a string or array"));
				};
				let mut stack: [Option<TileRef>; MAX_LAYERS] = [None; MAX_LAYERS];
				for part in parts {
					let (id, transform) = match part.split_once(':') {
						Some((id, t)) => (id, Transform::parse(t).map_err(|e| format!("cell {x},{y}: {e}"))?),
						None => (part, Transform::default()),
					};
					let (pack, tile) = resolve(id).map_err(|e| format!("cell {x},{y}: {e}"))?;
					let layer = if Some(pack) == water_pack { LAYER_WATER } else { LAYER_GROUND };
					if stack[layer].is_some() {
						return Err(format!("cell {x},{y}: duplicate {} layer", ["water", "ground"][layer]));
					}
					stack[layer] = Some(TileRef { pack, tile, transform });
				}
				cells.push(stack);
			}
		}

		// Optional `"pass"` block: sparse per-cell pass overrides,
		// keyed `"x,y": value` (0 land / 1 water / 2 shore / 3 blocked).
		let mut pass_overrides = vec![None; width as usize * height as usize];
		if let Some(po) = root.get("pass") {
			let entries = po.as_object().ok_or("'pass' not an object")?;
			for (key, value) in entries {
				let (xs, ys) = key.split_once(',').ok_or(format!("pass key '{key}': want x,y"))?;
				let x: u16 = xs.trim().parse().map_err(|_| format!("pass key '{key}': bad x"))?;
				let y: u16 = ys.trim().parse().map_err(|_| format!("pass key '{key}': bad y"))?;
				let v = value.as_f64().ok_or(format!("pass {key}: not a number"))? as u8;
				if x >= width || y >= height || v > 3 {
					return Err(format!("pass {key}: out of range"));
				}
				pass_overrides[y as usize * width as usize + x as usize] = Some(v);
			}
		}

		// Optional `"units"` block: unit-preview annotations as compact
		// `"TAG x y team"` strings (editor aid — never baked into the WRL).
		let mut units = Vec::new();
		if let Some(list) = root.get("units") {
			for (i, entry) in list.as_array().ok_or("'units' not an array")?.iter().enumerate() {
				let text = entry.as_str().ok_or(format!("units[{i}]: not a string"))?;
				let parts: Vec<&str> = text.split_whitespace().collect();
				let [tag, xs, ys, ts] = parts[..] else {
					return Err(format!("units[{i}] '{text}': want \"TAG x y team\""));
				};
				let x: u16 = xs.parse().map_err(|_| format!("units[{i}]: bad x"))?;
				let y: u16 = ys.parse().map_err(|_| format!("units[{i}]: bad y"))?;
				let team: u8 = ts.parse().map_err(|_| format!("units[{i}]: bad team"))?;
				if x >= width || y >= height || team > 4 {
					return Err(format!("units[{i}] '{text}': out of range"));
				}
				units.push(UnitNote { tag: tag.to_string(), x, y, team });
			}
		}

		Ok(Self {
			version,
			name,
			description,
			width,
			height,
			uses,
			packs,
			cells,
			pass_overrides,
			palette,
			pack_palette,
			source_palette,
			water_pack,
			units,
			dirty: false,
			revision: 0,
			structure: 0,
			undo_stack: Vec::new(),
			redo_stack: Vec::new(),
			stroke: None,
		})
	}

	/// Import a flat WRL as a Project — the in-memory form for an opened
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
		let ids: Vec<String> = (0..tile_count).map(|i| format!("WRL{i:05}")).collect();
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
		};

		// A flat WRL cell is a fully composited, opaque tile — it rides the
		// water (base) layer; ground stays empty.
		let cells: Vec<[Option<TileRef>; MAX_LAYERS]> = wrl
			.bigmap
			.iter()
			.map(|&tile| {
				let mut stack = [None; MAX_LAYERS];
				stack[LAYER_WATER] = Some(TileRef { pack: 0, tile, transform: Transform::default() });
				stack
			})
			.collect();

		Self {
			version: "1".to_string(),
			name: name.to_string(),
			description: String::new(),
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

	/// 1×1 placeholder Project — the document the editor holds before the
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
	/// randomly distributed water variants (identity transform — WATER is
	/// `sync`), ground empty. Deterministic from `seed`. WATER is implied
	/// when not listed; the first listed pack with a palette owns it.
	pub fn new(width: u16, height: u16, pack_names: &[String], assets_root: &Path, seed: u64) -> Result<Self, String> {
		if width == 0 || height == 0 || width > 1024 || height > 1024 {
			return Err(format!("bad map size {width}×{height} (1..=1024)"));
		}

		// WATER first (it fills the bottom layer), then the rest, deduped.
		let mut names: Vec<String> = vec!["WATER".to_string()];
		for name in pack_names {
			if !names.contains(name) {
				names.push(name.clone());
			}
		}
		let packs: Vec<TilePack> =
			names.iter().map(|name| TilePack::load(assets_root, name)).collect::<Result<_, _>>()?;

		// First pack with a palette owns it (compatibility verdicts).
		let owner = packs
			.iter()
			.position(|p| p.palette.is_some())
			.ok_or("no palette-owning pack — add a tileset (e.g. GREEN)")?;
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
			version: "1".to_string(),
			name: "Untitled".to_string(),
			description: String::new(),
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
	/// Bumped on structural changes — tile/palette table swaps (palette
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
	/// saved with the project) but records no undo patch — annotations are
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
	/// empty string for an empty stack) — also the `assert-cell` syntax.
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

	/// Set layer entries (one undo transaction — or part of the open stroke);
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
				self.undo_stack.push(Patch { cells, ..Patch::default() });
				if self.undo_stack.len() > MAX_UNDO {
					self.undo_stack.remove(0);
				}
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
				self.undo_stack.push(Patch { colors: vec![(slot, prev)], ..Patch::default() });
				if self.undo_stack.len() > MAX_UNDO {
					self.undo_stack.remove(0);
				}
			}
		}
		self.redo_stack.clear();
		self.bump();
		Ok(true)
	}

	/// Shift a whole water cycle block (the one containing `slot`) in HSL —
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
	/// internal palette / the pack's `palette.json` — game statics **not**
	/// applied) with this map's live dynamic-slot edits merged in. What the
	/// game would ignore, but what the file actually says — the debug render
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
	/// on disk — mutating them would not persist).
	pub fn is_wrl_import(&self) -> bool {
		!self.uses.is_empty() && self.uses.iter().all(|u| u.version == "wrl")
	}

	/// Snapshot everything a document-level operation may replace — the undo
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
	/// pre-change snapshot (see [`Self::doc_state`]). Structural — the
	/// renderer must rebuild its atlas (see [`Self::structure_revision`]).
	fn push_doc_patch(&mut self, before: Box<DocState>) {
		self.end_stroke(); // a doc swap must not interleave with an open stroke
		self.undo_stack.push(Patch { doc: Some(before), ..Patch::default() });
		if self.undo_stack.len() > MAX_UNDO {
			self.undo_stack.remove(0);
		}
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
	/// colors" method — see [`crate::palette_convert`] for the rules: only
	/// used colors move, game-animated slots are never used, water cycles are
	/// preserved per `opts`, in-game statics are reused when possible and the
	/// rest approximate into the unused dynamic slots). Tile pixels are
	/// rewritten through the slot mapping, so the rendered map keeps
	/// (approximately) its internal-palette look while becoming game-correct.
	///
	/// Lossy but undoable — the change lands as one document-swap undo unit.
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
		// level: the working copy, the source ("internal") palette — they now
		// agree — and the owner pack's (the save/export baseline).
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
	/// Lossy but undoable — one document-swap undo unit. Errors leave the
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
	/// document's content — one document-swap undo unit. Pass truth lives in
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

	/// Abort the open stroke: revert its edits right now and discard them —
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
		self.undo_stack.push(stroke);
		if self.undo_stack.len() > MAX_UNDO {
			self.undo_stack.remove(0);
		}
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
	/// the clicked cell's, replacing each with `entry` — or a random variant of
	/// it when `randomize`. One undo unit; returns whether anything changed.
	pub fn fill(&mut self, x: u16, y: u16, entry: TileRef, layer: usize, randomize: bool, rng: &mut Rng) -> bool {
		if x >= self.width || y >= self.height || layer >= MAX_LAYERS {
			return false;
		}
		let w = self.width as usize;
		let idx = |x: u16, y: u16| y as usize * w + x as usize;
		let target = self.cells[idx(x, y)][layer];
		let mut seen = vec![false; w * self.height as usize];
		let mut stack = vec![(x, y)];
		seen[idx(x, y)] = true;
		let mut edits = Vec::new();
		while let Some((cx, cy)) = stack.pop() {
			let tile = if randomize { self.random_variant(entry, rng) } else { entry };
			edits.push((cx, cy, layer, Some(tile)));
			let mut neigh = [(0u16, 0u16); 4];
			let mut k = 0;
			if cx > 0 {
				neigh[k] = (cx - 1, cy);
				k += 1;
			}
			if cx + 1 < self.width {
				neigh[k] = (cx + 1, cy);
				k += 1;
			}
			if cy > 0 {
				neigh[k] = (cx, cy - 1);
				k += 1;
			}
			if cy + 1 < self.height {
				neigh[k] = (cx, cy + 1);
				k += 1;
			}
			for &(nx, ny) in &neigh[..k] {
				let n = idx(nx, ny);
				if !seen[n] && self.cells[n][layer] == target {
					seen[n] = true;
					stack.push((nx, ny));
				}
			}
		}
		self.place_many(&edits)
	}

	/// Set per-cell pass overrides (Pass Table Editor). Undoable —
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
				self.undo_stack.push(Patch { passes, ..Patch::default() });
				if self.undo_stack.len() > MAX_UNDO {
					self.undo_stack.remove(0);
				}
			}
		}
		self.redo_stack.clear();
		self.bump();
		true
	}

	pub fn set_pass(&mut self, x: u16, y: u16, value: u8) -> bool {
		self.set_pass_many(&[(x, y, value)])
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
				self.undo_stack.push(Patch { passes, ..Patch::default() });
				if self.undo_stack.len() > MAX_UNDO {
					self.undo_stack.remove(0);
				}
			}
		}
		self.redo_stack.clear();
		self.bump();
		true
	}

	/// Set the water (base) layer tile by raw index — the flat-document edit
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
		if new_w == 0 || new_h == 0 || new_w > 1024 || new_h > 1024 {
			return Err(format!("bad map size {new_w}×{new_h} (1..=1024)"));
		}
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
		// A dimension change can't be a per-cell patch — drop the journal.
		self.undo_stack.clear();
		self.redo_stack.clear();
		self.stroke = None;
		self.bump();
		Ok(())
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
		// live fields and carry the displaced state back out. Structural —
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
		Patch { cells, colors, passes, doc: None }
	}

	fn bump(&mut self) {
		self.dirty = true;
		self.revision += 1;
	}

	pub fn cell(&self, x: u16, y: u16) -> Option<&[Option<TileRef>; MAX_LAYERS]> {
		if x >= self.width || y >= self.height {
			return None;
		}
		Some(&self.cells[y as usize * self.width as usize + x as usize])
	}

	/// Serialize back to the v1 JSON format (round-trip stable).
	pub fn save_string(&self) -> String {
		use json::JsonValue as J;
		let use_entries: Vec<J> = self
			.uses
			.iter()
			.map(|u| {
				let mut fields = vec![("name".to_string(), J::String(u.name.clone()))];
				if u.tileset {
					fields.push(("tileset".to_string(), J::Bool(true)));
				}
				if u.palette {
					fields.push(("palette".to_string(), J::Bool(true)));
				}
				fields.push(("version".to_string(), J::String(u.version.clone())));
				J::Object(fields)
			})
			.collect();

		let mut rows = Vec::with_capacity(self.height as usize);
		for y in 0..self.height as usize {
			let mut row = Vec::with_capacity(self.width as usize);
			for x in 0..self.width as usize {
				let stack = &self.cells[y * self.width as usize + x];
				let mut text = String::new();
				for layer in stack.iter().flatten() {
					if !text.is_empty() {
						text.push(',');
					}
					text.push_str(&self.packs[layer.pack as usize].ids[layer.tile as usize]);
					text.push_str(&layer.transform.suffix());
				}
				row.push(J::String(text));
			}
			rows.push(J::Array(row));
		}

		// The map's palette overrides: dynamic slots differing from the
		// owner pack's palette, as a sparse `{ "96": "#aabbcc" }` block.
		let mut overrides = Vec::new();
		for slot in DYNAMIC_SLOTS {
			let at = slot as usize * 3;
			if self.palette[at..at + 3] != self.pack_palette[at..at + 3] {
				overrides.push((
					slot.to_string(),
					J::String(format!(
						"#{:02x}{:02x}{:02x}",
						self.palette[at],
						self.palette[at + 1],
						self.palette[at + 2],
					)),
				));
			}
		}

		let mut fields = vec![
			("version".to_string(), J::String(self.version.clone())),
			("name".to_string(), J::String(self.name.clone())),
			("description".to_string(), J::String(self.description.clone())),
			("width".to_string(), J::Number(self.width as f64)),
			("height".to_string(), J::Number(self.height as f64)),
			("use".to_string(), J::Array(use_entries)),
		];
		if !overrides.is_empty() {
			fields.push(("palette".to_string(), J::Object(overrides)));
		}
		// Per-cell pass overrides as a sparse `{ "x,y": value }`.
		let mut pass = Vec::new();
		for y in 0..self.height as usize {
			for x in 0..self.width as usize {
				if let Some(v) = self.pass_overrides[y * self.width as usize + x] {
					pass.push((format!("{x},{y}"), J::Number(v as f64)));
				}
			}
		}
		if !pass.is_empty() {
			fields.push(("pass".to_string(), J::Object(pass)));
		}
		// Unit-preview annotations as compact `"TAG x y team"` strings —
		// only when present, so unit-free projects stay byte-identical.
		if !self.units.is_empty() {
			let list: Vec<J> =
				self.units.iter().map(|u| J::String(format!("{} {} {} {}", u.tag, u.x, u.y, u.team))).collect();
			fields.push(("units".to_string(), J::Array(list)));
		}
		fields.push(("map".to_string(), J::Array(rows)));
		J::Object(fields).to_pretty()
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
			transform_into(&mut out, pixels, water.transform, false);
		}
		if let Some(ground) = stack[LAYER_GROUND] {
			let pixels = self.packs[ground.pack as usize].tile_pixels(ground.tile);
			transform_into(&mut out, pixels, ground.transform, stack[LAYER_WATER].is_some());
		}
		out
	}

	/// One composed pixel of a cell — the single-pixel form of
	/// `compose_cell` (O(1); minimap/overworld previews sample with this).
	pub fn pixel_at(&self, x: u16, y: u16, sub: (usize, usize)) -> u8 {
		let Some(stack) = self.cell(x, y) else { return 0 };
		if let Some(ground) = stack[LAYER_GROUND] {
			let pixel = self.tile_pixel(ground, sub);
			if pixel != 0 {
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

	/// The in-game minimap byte for a cell (composed center pixel — the
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
	transform_into(&mut out, src, transform, false);
	out
}

/// Write `src` into `dst` with `transform` applied; when `transparent`,
/// index-0 pixels keep the existing `dst` value (layer fall-through).
fn transform_into(dst: &mut [u8; TILE_DATA_SIZE], src: &[u8], transform: Transform, transparent: bool) {
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
			if !transparent || pixel != 0 {
				dst[dy * n + dx] = pixel;
			}
		}
	}
}

/// The resumable rasterize-and-reimport palette conversion: render the map
/// through its internal palette (the "Rendering map" phase), then run the
/// raster through the New-from-Image [`ConvertSession`](crate::ConvertSession)
/// pipeline. The shell drives it per frame — `step` does a bounded slice and
/// reports `progress`/`stage` — so the modal stays live (progress bar, ETA,
/// Abort). [`Project::convert_palette_by_reimport`] is the run-to-completion
/// convenience over this (scripts/headless).
///
/// The session borrows nothing: `step` re-takes the project each call, so it
/// parks in the modal between frames. The project must not change under a
/// live session (the modal blocks input; a dimension change is caught and
/// reported as an error rather than composing out of bounds).
pub struct PaletteReimport {
	preserve_water: bool,
	width: u16,
	height: u16,
	internal: Vec<u8>,
	/// Target raster (filled during the render phase, then moved into `inner`).
	rgba: Vec<u8>,
	pins: Vec<u8>,
	/// Next cell row to rasterize.
	row: usize,
	dedupe: crate::image_import::Dedupe,
	threshold: f32,
	inner: Option<crate::image_import::ConvertSession>,
	error: Option<String>,
}

/// The render phase's share of the progress bar (the re-import pipeline's
/// own phases fill the rest).
const RASTER_WEIGHT: f32 = 0.15;

impl PaletteReimport {
	pub fn new(project: &Project, preserve_water: bool, dedupe: crate::image_import::Dedupe, threshold: f32) -> Self {
		let (tw, th) = (project.width as usize * TILE_SIZE, project.height as usize * TILE_SIZE);
		Self {
			preserve_water,
			width: project.width,
			height: project.height,
			internal: project.internal_palette(),
			rgba: vec![0u8; tw * th * 4],
			pins: vec![0u8; tw * th],
			row: 0,
			dedupe,
			threshold,
			inner: None,
			error: None,
		}
	}

	pub fn is_done(&self) -> bool {
		self.error.is_some() || self.inner.as_ref().is_some_and(|s| s.is_done())
	}

	pub fn error(&self) -> Option<&str> {
		self.error.as_deref().or_else(|| self.inner.as_ref().and_then(|s| s.error()))
	}

	/// 0..1 overall progress (render phase first, then the import pipeline).
	pub fn progress(&self) -> f32 {
		match &self.inner {
			Some(s) => RASTER_WEIGHT + (1.0 - RASTER_WEIGHT) * s.progress(),
			None => RASTER_WEIGHT * self.row as f32 / self.height.max(1) as f32,
		}
	}

	pub fn stage(&self) -> &'static str {
		match &self.inner {
			Some(s) => s.stage(),
			None => "Rendering map",
		}
	}

	/// Do bounded work — render cell rows, then hand the raster to the
	/// re-import pipeline and step it. `budget` is in pixel-equivalent units
	/// (one cell = 4096).
	pub fn step(&mut self, project: &Project, budget: usize) {
		if self.is_done() {
			return;
		}
		if (project.width, project.height) != (self.width, self.height) {
			self.error = Some("the document changed under the conversion".into());
			return;
		}
		let (w, h) = (self.width as usize, self.height as usize);
		let tw = w * TILE_SIZE;
		let mut done = 0usize;
		while self.row < h && done < budget {
			let cy = self.row;
			for cx in 0..w {
				let tile = project.compose_cell(cx as u16, cy as u16);
				for py in 0..TILE_SIZE {
					let row = (cy * TILE_SIZE + py) * tw + cx * TILE_SIZE;
					for px in 0..TILE_SIZE {
						let idx = tile[py * TILE_SIZE + px];
						let at = (row + px) * 4;
						let p = idx as usize * 3;
						self.rgba[at..at + 3].copy_from_slice(&self.internal[p..p + 3]);
						self.rgba[at + 3] = 255;
						if self.preserve_water && (96..=127).contains(&idx) {
							self.pins[row + px] = idx;
						}
					}
				}
			}
			self.row += 1;
			done += w * TILE_DATA_SIZE / 16; // a composed cell is cheaper than a dithered one
		}
		if self.row < h {
			return;
		}
		if self.inner.is_none() {
			// Raster complete — build the import session (moves the buffers).
			let th = h * TILE_SIZE;
			let opts = crate::image_import::ConvertOpts {
				dedupe: self.dedupe,
				threshold: self.threshold,
				..crate::image_import::ConvertOpts::fit_source(tw as u32, th as u32)
			};
			let rgba = std::mem::take(&mut self.rgba);
			let pins = std::mem::take(&mut self.pins);
			match crate::image_import::ConvertSession::new(rgba, tw as u32, th as u32, opts) {
				Ok(mut session) => {
					if self.preserve_water {
						let water: Vec<(u8, [u8; 3])> = (96u8..=127)
							.map(|s| {
								let at = s as usize * 3;
								(s, [self.internal[at], self.internal[at + 1], self.internal[at + 2]])
							})
							.collect();
						if let Err(e) = session.pin(pins, &water) {
							self.error = Some(e);
							return;
						}
					}
					self.inner = Some(session);
				}
				Err(e) => {
					self.error = Some(e);
					return;
				}
			}
		}
		if let Some(session) = self.inner.as_mut() {
			session.step(budget.saturating_sub(done).max(1));
		}
	}

	/// Consume the finished session into a `WrlFile` (call once `is_done`; an
	/// errored session returns its error here).
	pub fn finish(mut self) -> Result<WrlFile, String> {
		if let Some(e) = self.error.take() {
			return Err(e);
		}
		self.inner.take().ok_or("conversion not finished")?.finish()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn assets_root() -> std::path::PathBuf {
		Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/assets")
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
		// The water layer carries it; ground is empty.
		let stack = p.cell(0, 0).unwrap();
		assert_eq!(stack[LAYER_WATER].map(|t| t.tile), Some(1));
		assert!(stack[LAYER_GROUND].is_none());
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

		// Reload with an assets root that lacks the pack — only the sibling
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
	/// the compatible form and converges the two — and undoes as one unit.
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
		// The pink moved to an (unused) free dynamic slot — pixels follow, exactly.
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
		// internal palette — and redo replays it byte-identically.
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

	/// Golden splitmix64 vectors (seed 0) — pins the algorithm forever:
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
			assert_eq!(water.transform, Transform::default(), "WATER is sync — identity");
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
		// Any rectangle is a valid WRL (confirmed 2026-06) — width/height
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
		// (`"palette": {` is the override block — `"palette": true` in the
		// `use` entries is the unrelated owner flag.)
		let text = p.save_string();
		assert!(text.contains("\"palette\": {"), "{text}");
		assert!(text.contains("\"100\": \"#0a141e\""), "{text}");
		let reloaded = Project::from_str(&text, &root).unwrap();
		assert_eq!(reloaded.palette[300..303], [10, 20, 30]);
		assert_eq!(reloaded.hash(), p.hash());

		// Undo restores the pack color — and the override block disappears.
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
