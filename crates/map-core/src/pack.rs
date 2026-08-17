//! Tile pack loading (`resources/assets/tilepacks/<PACK>/`) - see
//! `docs/design/tileset-contract.md` §2. Loads tile pixels, the index→id table,
//! the optional palette, pass values, the shore adjacency rules, and the family
//! props / variant groups / multi-tile patterns the editor tools (worldgen,
//! auto-shore, the randomizer) consume.

use std::collections::HashMap;
use std::path::Path;

use max_assets::wrl::TILE_DATA_SIZE;

use crate::project::Transform;

/// Directions are ring-indexed clockwise: N=0, E=1, S=2, W=3 (`shore.rs`
/// rotates them with this arithmetic).
pub(crate) const DIR_N: usize = 0;
pub(crate) const DIR_E: usize = 1;
pub(crate) const DIR_S: usize = 2;
pub(crate) const DIR_W: usize = 3;

/// A tile id's family: the id with its variant digits removed
/// (`"GSh004"` → `"GSh"`). Families key the match rules, variant groups,
/// and props.
pub fn family_of(id: &str) -> &str {
	id.trim_end_matches(|c: char| c.is_ascii_digit())
}

/// Semantic class of a tile family from `tiles.props.json` - what a family
/// *is* to editor logic (worldgen, auto-shore). Movement truth stays in
/// `tiles.pass.json`; this is not pass data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileKind {
	Water,
	Land,
	Shore,
	Obstruction,
}

/// May the editor transform a family's tiles? Restrictions exist to keep
/// baked light/shadow from being corrupted by a rotation the art was never
/// drawn for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Transformable {
	/// Not transformable (absent or `false`).
	#[default]
	No,
	/// Free rotation + mirror (`true`).
	Free,
	/// Transformable, but all placed tiles of the group keep their transform
	/// in sync - even ones already painted (`"sync"`).
	Sync,
	/// Only 0° or 180° rotation, no mirror (`"invert"`).
	Invert,
}

/// One family's `tiles.props.json` entry.
#[derive(Debug, Clone, Default)]
pub struct FamilyProps {
	/// The family is one interchangeable variant group (randomizer fodder).
	pub has_variants: bool,
	pub kind: Option<TileKind>,
	pub transformable: Transformable,
	/// The family's pixels use the mask color for transparency (shore tiles
	/// showing the water beneath).
	pub use_mask_color: bool,
	/// Palette index rendered transparent for this family (`tiles.props.json`
	/// `"mask"`). `None` = fully opaque; `Some(i)` = pixels equal to `i` fall
	/// through to the layer beneath. Only shore families carry one.
	pub mask: Option<u8>,
}

/// One multi-tile formation from `tiles.patterns.json` - extracted from the
/// original maps (`examples/extract_patterns.rs`), since the obstruction
/// adjacency that would live in `tiles.match.json` was never authored. A
/// pattern is one family's connected formation in its bounding box; holes
/// stay `None` (irregular shapes are not filled). The worldgen
/// stamps these whole.
#[derive(Clone)]
pub struct TilePattern {
	pub name: String,
	pub width: u16,
	pub height: u16,
	/// Row-major `width * height` bin indices; `None` = unpopulated.
	pub cells: Vec<Option<u16>>,
}

/// One tile family's adjacency rules from `tiles.match.json`, base
/// orientation: per direction, the allowed neighbors - tile specs like
/// `"GSa:!S"`, or the wildcards `"__WATER__"` / `"__LAND__"`.
#[derive(Clone)]
pub struct MatchRule {
	pub dirs: [Vec<String>; 4],
}

impl MatchRule {
	/// May this direction face water?
	pub fn allows_water(&self, dir: usize) -> bool {
		self.dirs[dir].iter().any(|s| s == "__WATER__")
	}

	/// Must this direction face water (nothing else listed)?
	pub fn requires_water(&self, dir: usize) -> bool {
		self.dirs[dir].len() == 1 && self.dirs[dir][0] == "__WATER__"
	}
}

#[derive(Clone)]
pub struct TilePack {
	pub name: String,
	/// Pack version from `info.json` (recorded in project `use` entries).
	pub version: String,
	/// `tile_count * 4096` bytes, 64×64 8-bit palette-indexed tiles.
	pub tiles: Vec<u8>,
	/// Bin index → tile id (`"GSd004"`).
	pub ids: Vec<String>,
	/// Tile id → bin index.
	pub index_of: HashMap<String, u16>,
	/// 256×RGB - present only on palette-owning packs.
	pub palette: Option<Vec<u8>>,
	/// The palette's display name from `palette.json`'s object form
	/// (`"Green Palette"`); `None` on palette-less packs and bare-array files.
	pub palette_name: Option<String>,
	/// Per-tile passability (0 land / 1 water / 2 shore / 3 blocked),
	/// indexed by bin index - from `tiles.pass.json` (recovered from the
	/// original WRL passtabs). `None` when the pack ships without it.
	pub pass: Option<Vec<u8>>,
	/// Family (`"GSa"`) → adjacency rules, from `tiles.match.json`
	/// (auto-shore, tile suggestions, diagnostics).
	/// Empty when the pack ships without it.
	pub matches: HashMap<String, MatchRule>,
	/// Interchangeable look-variant groups (lists of tile indices) from
	/// `tiles.variants.json` - the random-paint toggle picks among a
	/// tile's siblings. Empty when the pack ships without it. Read via
	/// [`TilePack::variants_of`].
	pub variant_groups: Vec<Vec<u16>>,
	/// Tile index → its `variant_groups` index (`None` = no variants).
	pub variant_of: Vec<Option<u16>>,
	/// Variant-group name → `variant_groups` index. The group name matches the
	/// tile family in every shipped pack (WATER's `WTR` group holds `WTR000…`);
	/// they only differ when tiles are linked across families in the match
	/// editor (resolved via [`TilePack::group_of`]).
	pub variant_named: HashMap<String, u16>,
	/// Group key → semantic props from `tiles.props.json` (worldgen,
	/// transform guards). Keys are variant-group names or tile-id families -
	/// resolve tiles via [`TilePack::group_tiles`]. Empty when the pack
	/// ships without it.
	pub props: HashMap<String, FamilyProps>,
	/// Multi-tile formations from `tiles.patterns.json` (worldgen).
	/// Empty when the pack ships without it.
	pub patterns: Vec<TilePattern>,
	/// A user-owned pack (`resources/user/tilepacks/<name>/`): editable without
	/// `--dev`, persisted to its own folder (not beside the project, not into
	/// the shipped `resources/assets`). Stock + synthetic-WRL packs are `false`.
	pub user: bool,
}

/// Write `bytes` to `path` only if it differs from what's already there;
/// returns whether a write happened (so Bake reports only real changes and
/// leaves unchanged files - and their git mtime - alone).
fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<bool, String> {
	if std::fs::read(path).is_ok_and(|cur| cur == bytes) {
		return Ok(false);
	}
	std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
	Ok(true)
}

/// Quote + escape a string as a JSON string literal. Tile ids and match specs
/// are ASCII (`[A-Za-z0-9_:!]`), so this only ever needs `"`/`\` escaping, but
/// it stays correct for arbitrary input.
fn json_string(s: &str) -> String {
	let mut out = String::with_capacity(s.len() + 2);
	out.push('"');
	for c in s.chars() {
		match c {
			'"' => out.push_str("\\\""),
			'\\' => out.push_str("\\\\"),
			'\n' => out.push_str("\\n"),
			'\t' => out.push_str("\\t"),
			c => out.push(c),
		}
	}
	out.push('"');
	out
}

/// Canonical sort key for a match-list entry: wildcards (`__LAND__`/`__WATER__`)
/// first, then concrete `group:suffix` entries by group name, then by the
/// neighbor transform's bit pattern (identity, the three rotations, then the
/// four mirrored). Gives a stable, diff-friendly order after editing (the
/// shipped files were in the offline extractor's discovery order).
fn entry_sort_key(s: &str) -> (u8, String, u8) {
	if s.starts_with("__") {
		return (0, s.to_string(), 0);
	}
	match s.split_once(':') {
		Some((g, t)) => (1, g.to_string(), Transform::parse(t).map(|t| t.bits() as u8).unwrap_or(u8::MAX)),
		None => (1, s.to_string(), 0),
	}
}

/// Serialize match rules to the shipped `tiles.match.json` text: tab-indented,
/// group keys sorted, directions in file order N/W/S/E (storage is ring-indexed
/// N/E/S/W), each direction's entries canonically sorted. Hand-rolled because
/// the `json` dep doesn't control whitespace; round-trips the shipped files
/// (modulo the offline extractor's original entry ordering).
pub fn serialize_matches(matches: &HashMap<String, MatchRule>) -> String {
	let mut keys: Vec<&String> = matches.keys().collect();
	keys.sort();
	let mut out = String::from("{\n");
	for (ki, key) in keys.iter().enumerate() {
		let rule = &matches[*key];
		out.push('\t');
		out.push_str(&json_string(key));
		out.push_str(": {\n");
		const FILE_DIRS: [(&str, usize); 4] = [("N", DIR_N), ("W", DIR_W), ("S", DIR_S), ("E", DIR_E)];
		for (di, (label, ring)) in FILE_DIRS.iter().enumerate() {
			let mut entries = rule.dirs[*ring].clone();
			entries.sort_by_key(|e| entry_sort_key(e));
			entries.dedup();
			if entries.is_empty() {
				out.push_str(&format!("\t\t\"{label}\": []"));
			} else {
				out.push_str(&format!("\t\t\"{label}\": [\n"));
				for (ei, e) in entries.iter().enumerate() {
					out.push_str("\t\t\t");
					out.push_str(&json_string(e));
					out.push_str(if ei + 1 < entries.len() { ",\n" } else { "\n" });
				}
				out.push_str("\t\t]");
			}
			out.push_str(if di + 1 < FILE_DIRS.len() { ",\n" } else { "\n" });
		}
		out.push_str("\t}");
		out.push_str(if ki + 1 < keys.len() { ",\n" } else { "\n" });
	}
	out.push_str("}\n");
	out
}

/// Serialize variant groups to the shipped `tiles.variants.json` text:
/// tab-indented, group keys sorted, tile ids in the given order.
pub fn serialize_variants(groups: &[(String, Vec<String>)]) -> String {
	let mut g: Vec<&(String, Vec<String>)> = groups.iter().filter(|(_, t)| !t.is_empty()).collect();
	g.sort_by(|a, b| a.0.cmp(&b.0));
	let mut out = String::from("{\n");
	for (gi, (name, tiles)) in g.iter().enumerate() {
		out.push('\t');
		out.push_str(&json_string(name));
		out.push_str(": [\n");
		for (ti, t) in tiles.iter().enumerate() {
			out.push_str("\t\t");
			out.push_str(&json_string(t));
			out.push_str(if ti + 1 < tiles.len() { ",\n" } else { "\n" });
		}
		out.push_str("\t]");
		out.push_str(if gi + 1 < g.len() { ",\n" } else { "\n" });
	}
	out.push_str("}\n");
	out
}

/// Serialize the id table to `tiles-data.json` (array form, tab-indented).
pub fn serialize_ids(ids: &[String]) -> String {
	let mut out = String::from("[\n");
	for (i, id) in ids.iter().enumerate() {
		out.push('\t');
		out.push_str(&json_string(id));
		out.push_str(if i + 1 < ids.len() { ",\n" } else { "\n" });
	}
	out.push_str("]\n");
	out
}

/// Serialize the pass table to `tiles.pass.json` (`{ "<id>": <0..3>, … }`, in id
/// order, tab-indented).
pub fn serialize_pass(ids: &[String], pass: &[u8]) -> String {
	let mut out = String::from("{\n");
	let n = ids.len().min(pass.len());
	for i in 0..n {
		out.push('\t');
		out.push_str(&json_string(&ids[i]));
		out.push_str(&format!(": {}", pass[i]));
		out.push_str(if i + 1 < n { ",\n" } else { "\n" });
	}
	out.push_str("}\n");
	out
}

/// Replace whole tile-id tokens `old`→`new` in cell/JSON text - only where `old`
/// is bounded by a JSON string quote or a cell separator (`"` / `,` before, and
/// `"` / `,` / `:` after), so renaming `GSa00` never touches `GSa000`. Returns the
/// rewritten text and the replacement count. The match editor's id-rename cascade
/// runs this over every shipped map + template.
pub fn replace_id_token(text: &str, old: &str, new: &str) -> (String, usize) {
	if old.is_empty() || old == new {
		return (text.to_string(), 0);
	}
	let bytes = text.as_bytes();
	let mut out = String::with_capacity(text.len());
	let mut i = 0;
	let mut count = 0;
	while i < text.len() {
		if text[i..].starts_with(old) {
			let before = i.checked_sub(1).map(|j| bytes[j]);
			let after = bytes.get(i + old.len()).copied();
			let lb = matches!(before, Some(b'"') | Some(b','));
			let rb = matches!(after, Some(b'"') | Some(b',') | Some(b':'));
			if lb && rb {
				out.push_str(new);
				i += old.len();
				count += 1;
				continue;
			}
		}
		let ch = text[i..].chars().next().unwrap();
		out.push(ch);
		i += ch.len_utf8();
	}
	(out, count)
}

impl TilePack {
	pub fn tile_count(&self) -> u16 {
		(self.tiles.len() / TILE_DATA_SIZE) as u16
	}

	pub fn tile_pixels(&self, index: u16) -> &[u8] {
		let at = index as usize * TILE_DATA_SIZE;
		&self.tiles[at..at + TILE_DATA_SIZE]
	}

	/// Overwrite a tile's 64×64 palette indices (the Tile Painter's repaint).
	pub fn set_tile_pixels(&mut self, index: u16, pixels: &[u8]) {
		debug_assert_eq!(pixels.len(), TILE_DATA_SIZE, "tile pixels must be 64x64");
		let at = index as usize * TILE_DATA_SIZE;
		self.tiles[at..at + TILE_DATA_SIZE].copy_from_slice(pixels);
	}

	/// Set a tile's passability (0 land / 1 water / 2 shore / 3 blocked),
	/// allocating the pass table if the pack shipped without one.
	pub fn set_tile_pass(&mut self, index: u16, pass: u8) {
		let count = self.tile_count() as usize;
		self.pass.get_or_insert_with(|| vec![0u8; count])[index as usize] = pass;
	}

	/// Rename tile `index` (the Tile Painter's id edit): swaps the id table entry
	/// and re-keys `index_of`. Variant groups (index-keyed) are unaffected;
	/// match/pattern files that reference the old id by string are not rewritten.
	pub fn rename_tile(&mut self, index: u16, new_id: &str) {
		let old = std::mem::replace(&mut self.ids[index as usize], new_id.to_string());
		self.index_of.remove(&old);
		self.index_of.insert(new_id.to_string(), index);
	}

	/// The transparency mask color of tile `index` - its family's `"mask"` from
	/// `tiles.props.json`, or `None` when the family is fully opaque.
	pub fn tile_mask(&self, index: u16) -> Option<u8> {
		self.props.get(family_of(&self.ids[index as usize])).and_then(|p| p.mask)
	}

	/// Tile `index`'s semantic props, resolved through its variant-group name
	/// when it has one (else its id family) - the same keying
	/// [`Self::group_tiles`] reverses.
	pub fn tile_props(&self, index: u16) -> Option<&FamilyProps> {
		if let Some(g) = self.variant_of.get(index as usize).copied().flatten() {
			if let Some((name, _)) = self.variant_named.iter().find(|&(_, &gi)| gi == g) {
				if let Some(p) = self.props.get(name) {
					return Some(p);
				}
			}
		}
		self.props.get(family_of(&self.ids[index as usize]))
	}

	/// How tile `index`'s family may be transformed (`tiles.props.json`
	/// `transformable`); families with no props entry are [`Transformable::No`].
	pub fn tile_transformable(&self, index: u16) -> Transformable {
		self.tile_props(index).map(|p| p.transformable).unwrap_or_default()
	}

	/// Load a pack from the `assets_root/name` directory (the fs entry point).
	pub fn load(assets_root: &Path, name: &str) -> Result<Self, String> {
		let dir = assets_root.join(name);
		Self::from_reader(
			name,
			|file| std::fs::read(dir.join(file)).map_err(|e| format!("{name}/{file}: {e}")),
			|file| match std::fs::read_to_string(dir.join(file)) {
				Err(_) => Ok(None), // absent (or unreadable) optional sidecar
				Ok(text) => Ok(Some(text)),
			},
		)
	}

	/// Build a pack from injected file readers, decoupled from the filesystem so
	/// the parse/validation paths are unit-testable without on-disk fixtures.
	/// `read_bin` returns a required file's raw bytes (errors if absent);
	/// `read_text` returns a file's text or `None` (absent). Error strings are
	/// prefixed `name/file`, matching [`load`](Self::load).
	fn from_reader(
		name: &str,
		read_bin: impl Fn(&str) -> Result<Vec<u8>, String>,
		read_text: impl Fn(&str) -> Result<Option<String>, String>,
	) -> Result<Self, String> {
		// Read + parse an *optional* sidecar JSON (`Ok(None)` when the file is
		// absent); parse errors are prefixed `name/file`.
		let read_json_opt = |file: &str| -> Result<Option<json::JsonValue>, String> {
			match read_text(file)? {
				None => Ok(None),
				Some(text) => json::parse(&text).map(Some).map_err(|e| format!("{name}/{file}: {e}")),
			}
		};

		let tiles = read_bin("tiles-data.bin")?;
		if tiles.len() % TILE_DATA_SIZE != 0 {
			return Err(format!("{name}/tiles-data.bin: not a multiple of {TILE_DATA_SIZE}"));
		}
		let tile_count = tiles.len() / TILE_DATA_SIZE;

		// tiles-data.json: bin index → tile id, in either shape found in the
		// shipped packs: `["WTR000", …]` (index = position) or
		// `{ "0": "SCa000", … }`. TODO: normalize the packs to one shape.
		let id_map = read_json_opt("tiles-data.json")?.ok_or(format!("{name}/tiles-data.json: not found"))?;
		let mut ids = vec![String::new(); tile_count];
		let mut index_of = HashMap::with_capacity(tile_count);
		let mut put = |index: usize, id: &str| -> Result<(), String> {
			if index >= tile_count {
				return Err(format!("{name}: index {index} out of range"));
			}
			ids[index] = id.to_string();
			index_of.insert(id.to_string(), index as u16);
			Ok(())
		};
		let entry_count = match (&id_map.as_array(), &id_map.as_object()) {
			(Some(list), _) => {
				for (index, value) in list.iter().enumerate() {
					let id = value.as_str().ok_or(format!("{name}: id {index} not a string"))?;
					put(index, id)?;
				}
				list.len()
			}
			(_, Some(entries)) => {
				for (key, value) in entries.iter() {
					let index: usize = key.parse().map_err(|_| format!("{name}: bad index '{key}'"))?;
					let id = value.as_str().ok_or(format!("{name}: id for '{key}' not a string"))?;
					put(index, id)?;
				}
				entries.len()
			}
			_ => return Err(format!("{name}/tiles-data.json: not an array or object")),
		};
		if entry_count != tile_count {
			return Err(format!("{name}: tiles-data.json has {entry_count} entries, bin has {tile_count} tiles",));
		}

		// palette.json: the bare ["#rrggbb", ...×256] array or the object form
		// { "name", "version", "colors": [...] } the shipped packs and user
		// palettes share (crate::palette reads both) - optional (WATER has none).
		let (palette, palette_name) = match read_text("palette.json")? {
			None => (None, None),
			Some(text) => {
				let rgb = crate::palette::parse_palette(&text).map_err(|e| format!("{name}/palette.json: {e}"))?;
				let pal_name =
					json::parse(&text).ok().and_then(|v| v.get("name").and_then(|n| n.as_str().map(String::from)));
				(Some(rgb), pal_name)
			}
		};

		// info.json: pack metadata - only `version` is consumed here (lenient:
		// a missing or malformed file just defaults the version).
		let version = read_text("info.json")
			.ok()
			.flatten()
			.and_then(|text| json::parse(&text).ok())
			.and_then(|v| v.get("version").and_then(|v| v.as_str().map(String::from)))
			.unwrap_or_else(|| "1".to_string());

		// tiles.pass.json: { "GSd004": 2, ... } - optional.
		let pass = match read_json_opt("tiles.pass.json")? {
			None => None,
			Some(value) => {
				let entries = value.as_object().ok_or(format!("{name}/tiles.pass.json: not an object"))?;
				let mut pass = vec![0u8; tile_count];
				for (id, v) in entries {
					let Some(&index) = index_of.get(id.as_str()) else {
						return Err(format!("{name}/tiles.pass.json: unknown tile '{id}'"));
					};
					let value = v.as_f64().ok_or(format!("{name}/tiles.pass.json: '{id}' not a number"))? as u8;
					if value > 3 {
						return Err(format!("{name}/tiles.pass.json: '{id}' pass {value} out of range"));
					}
					pass[index as usize] = value;
				}
				Some(pass)
			}
		};

		// tiles.match.json: { "GSa": { "N": [...], "W": [...], ... } } -
		// optional. File order is N/W/S/E; stored ring-indexed N/E/S/W.
		let mut matches = HashMap::new();
		if let Some(value) = read_json_opt("tiles.match.json")? {
			let families = value.as_object().ok_or(format!("{name}/tiles.match.json: not an object"))?;
			for (family, rule) in families {
				let mut dirs: [Vec<String>; 4] = Default::default();
				for (key, dir) in [("N", DIR_N), ("E", DIR_E), ("S", DIR_S), ("W", DIR_W)] {
					let Some(list) = rule.get(key) else { continue };
					let list =
						list.as_array().ok_or(format!("{name}/tiles.match.json: {family}.{key} not an array"))?;
					for entry in list {
						let spec =
							entry.as_str().ok_or(format!("{name}/tiles.match.json: {family}.{key} bad entry"))?;
						dirs[dir].push(spec.to_string());
					}
				}
				matches.insert(family.to_string(), MatchRule { dirs });
			}
		}

		// tiles.variants.json: { "GSa": ["GSa000", ...] } - optional. Each list
		// is a set of interchangeable look-variants; map the ids to bin indices
		// and record which group each tile belongs to.
		let mut variant_groups: Vec<Vec<u16>> = Vec::new();
		let mut variant_of: Vec<Option<u16>> = vec![None; tile_count];
		let mut variant_named: HashMap<String, u16> = HashMap::new();
		if let Some(value) = read_json_opt("tiles.variants.json")? {
			let families = value.as_object().ok_or(format!("{name}/tiles.variants.json: not an object"))?;
			for (family, list) in families {
				let list = list.as_array().ok_or(format!("{name}/tiles.variants.json: {family} not an array"))?;
				let mut group = Vec::new();
				for entry in list {
					let id = entry.as_str().ok_or(format!("{name}/tiles.variants.json: {family} bad entry"))?;
					if let Some(&idx) = index_of.get(id) {
						group.push(idx);
					}
				}
				if !group.is_empty() {
					let g = variant_groups.len() as u16;
					for &idx in &group {
						variant_of[idx as usize] = Some(g);
					}
					variant_named.insert(family.to_string(), g);
					variant_groups.push(group);
				}
			}
		}

		// tiles.props.json: { "GLa": { "hasVariants": true, "type": "LAND",
		// "transformable": true, "useMaskColor": false } } - optional.
		// Semantic family classes for editor tools (worldgen).
		let mut props = HashMap::new();
		if let Some(value) = read_json_opt("tiles.props.json")? {
			let families = value.as_object().ok_or(format!("{name}/tiles.props.json: not an object"))?;
			for (family, entry) in families {
				let kind = match entry.get("type").and_then(|v| v.as_str()) {
					None => None,
					Some("WATER") => Some(TileKind::Water),
					Some("LAND") => Some(TileKind::Land),
					Some("SHORE") => Some(TileKind::Shore),
					Some("OBSTRUCTION") => Some(TileKind::Obstruction),
					Some(other) => {
						return Err(format!("{name}/tiles.props.json: {family}: unknown type '{other}'"));
					}
				};
				let transformable = match entry.get("transformable") {
					None => Transformable::No,
					Some(v) => match (v.as_bool(), v.as_str()) {
						(Some(true), _) => Transformable::Free,
						(Some(false), _) => Transformable::No,
						(_, Some("sync")) => Transformable::Sync,
						(_, Some("invert")) => Transformable::Invert,
						_ => {
							return Err(format!(
								"{name}/tiles.props.json: {family}: bad transformable (true|false|\"sync\"|\"invert\")",
							));
						}
					},
				};
				props.insert(
					family.to_string(),
					FamilyProps {
						has_variants: entry.get("hasVariants").and_then(|v| v.as_bool()).unwrap_or(false),
						kind,
						transformable,
						use_mask_color: entry.get("useMaskColor").and_then(|v| v.as_bool()).unwrap_or(false),
						mask: entry.get("mask").and_then(|v| v.as_f64()).map(|n| n as u8),
					},
				);
			}
		}

		// tiles.patterns.json: [{ "name", "width", "height", "pattern":
		// [["CMa000", null, …], …] }] - optional. Formations extracted from
		// the original maps; `null` cells are holes.
		let mut patterns = Vec::new();
		if let Some(value) = read_json_opt("tiles.patterns.json")? {
			let list = value.as_array().ok_or(format!("{name}/tiles.patterns.json: not an array"))?;
			for entry in list {
				let pname = entry.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
				let width = entry.get("width").and_then(|v| v.as_f64()).unwrap_or(0.0) as u16;
				let height = entry.get("height").and_then(|v| v.as_f64()).unwrap_or(0.0) as u16;
				let rows = entry
					.get("pattern")
					.and_then(|v| v.as_array())
					.ok_or(format!("{name}/tiles.patterns.json: '{pname}' has no pattern rows"))?;
				if width == 0 || rows.len() != height as usize {
					return Err(format!("{name}/tiles.patterns.json: '{pname}' size mismatch"));
				}
				let mut cells = Vec::with_capacity(width as usize * height as usize);
				for row in rows {
					let row =
						row.as_array().ok_or(format!("{name}/tiles.patterns.json: '{pname}' row not an array"))?;
					if row.len() != width as usize {
						return Err(format!("{name}/tiles.patterns.json: '{pname}' row width mismatch"));
					}
					for cell in row {
						cells.push(match cell.as_str() {
							None => None, // null = hole
							Some(id) => Some(
								*index_of
									.get(id)
									.ok_or(format!("{name}/tiles.patterns.json: '{pname}': unknown tile '{id}'"))?,
							),
						});
					}
				}
				patterns.push(TilePattern { name: pname, width, height, cells });
			}
		}

		Ok(Self {
			name: name.to_string(),
			version,
			tiles,
			ids,
			index_of,
			palette,
			palette_name,
			pass,
			matches,
			variant_groups,
			variant_of,
			variant_named,
			props,
			patterns,
			user: false,
		})
	}

	/// An empty user-owned pack (`resources/user/tilepacks/<name>/`), ready to
	/// receive new/cloned tiles. Carries no palette - it borrows the project's,
	/// like any non-owner pack.
	pub fn empty_user(name: &str) -> Self {
		Self {
			name: name.to_string(),
			version: "1".to_string(),
			tiles: Vec::new(),
			ids: Vec::new(),
			index_of: HashMap::new(),
			palette: None,
			palette_name: None,
			pass: None,
			matches: HashMap::new(),
			variant_groups: Vec::new(),
			variant_of: Vec::new(),
			variant_named: HashMap::new(),
			props: HashMap::new(),
			patterns: Vec::new(),
			user: true,
		}
	}

	/// Append a tile (64×64 indices), its id, and passability; returns the new
	/// bin index. The id must be unique within the pack. The caller seeds
	/// `props` for the tile's family if the pack lacks it (so the mask/kind
	/// resolve). New tiles join no variant group.
	pub fn push_tile(&mut self, id: String, pixels: &[u8], pass: u8) -> u16 {
		debug_assert_eq!(pixels.len(), TILE_DATA_SIZE, "tile pixels must be 64x64");
		let index = self.tile_count();
		self.tiles.extend_from_slice(pixels);
		self.index_of.insert(id.clone(), index);
		self.ids.push(id);
		self.variant_of.push(None);
		match &mut self.pass {
			Some(p) => p.push(pass),
			None if pass != 0 => {
				let mut p = vec![0u8; index as usize];
				p.push(pass);
				self.pass = Some(p);
			}
			None => {}
		}
		index
	}

	/// The tiles a props/variants group key covers: the variant group
	/// registered under that name when one exists (WATER's `WTR` group holds
	/// the `WTR…` tiles), else every tile whose id family matches the key.
	pub fn group_tiles(&self, key: &str) -> Vec<u16> {
		if let Some(&g) = self.variant_named.get(key) {
			return self.variant_groups[g as usize].clone();
		}
		(0..self.tile_count()).filter(|&i| family_of(&self.ids[i as usize]) == key).collect()
	}

	/// A tile's interchangeable look-variants (bin indices, including `tile`
	/// itself); empty when the pack ships no variant group for it.
	pub fn variants_of(&self, tile: u16) -> &[u16] {
		match self.variant_of.get(tile as usize).copied().flatten() {
			Some(g) => &self.variant_groups[g as usize],
			None => &[],
		}
	}

	/// The name a variant group (by index) was registered under, if any.
	pub fn group_name(&self, g: u16) -> Option<&str> {
		self.variant_named.iter().find(|&(_, &gi)| gi == g).map(|(name, _)| name.as_str())
	}

	/// The match/props group key a tile belongs to: its variant-group name when
	/// it has one (WATER's `WTR` covers the `WTR…` tiles), else its id family.
	/// The auto-shore engine resolves a placed tile to its adjacency rule
	/// through this, so linking tiles into a group (even across id families) is
	/// what makes matching honor the link. Mirrors [`Self::tile_props`]'s
	/// resolution.
	pub fn group_of(&self, index: u16) -> &str {
		if let Some(g) = self.variant_of.get(index as usize).copied().flatten() {
			if let Some(name) = self.group_name(g) {
				return name;
			}
		}
		family_of(&self.ids[index as usize])
	}

	/// Replace this pack's match rules and variant grouping in one shot - the
	/// match-data editor's commit. `groups` is `(group name → member bin
	/// indices)`; empty groups are dropped. Rebuilds `variant_groups` /
	/// `variant_of` / `variant_named` so [`Self::group_of`] (and thus the live
	/// auto-shore engine) immediately reflects the new links.
	pub fn set_match_data(&mut self, groups: Vec<(String, Vec<u16>)>, matches: HashMap<String, MatchRule>) {
		let count = self.tile_count() as usize;
		let mut variant_groups: Vec<Vec<u16>> = Vec::new();
		let mut variant_of: Vec<Option<u16>> = vec![None; count];
		let mut variant_named: HashMap<String, u16> = HashMap::new();
		for (name, mut tiles) in groups {
			tiles.retain(|&t| (t as usize) < count);
			tiles.sort_unstable();
			tiles.dedup();
			if tiles.is_empty() {
				continue;
			}
			let g = variant_groups.len() as u16;
			for &t in &tiles {
				variant_of[t as usize] = Some(g);
			}
			variant_named.insert(name, g);
			variant_groups.push(tiles);
		}
		self.variant_groups = variant_groups;
		self.variant_of = variant_of;
		self.variant_named = variant_named;
		self.matches = matches;
	}

	/// Write this pack's match rules + variant groups to `dir/tiles.match.json`
	/// and `dir/tiles.variants.json` (only the files that actually changed).
	/// The match-data editor's save; the only writer of `tiles.match.json` (it
	/// is otherwise shipped read-only and [`Self::dump`] skips it). Returns
	/// whether anything was written.
	pub fn save_match_data(&self, dir: &Path) -> Result<bool, String> {
		let mut groups: Vec<(String, Vec<String>)> = self
			.variant_named
			.iter()
			.map(|(name, &g)| {
				let mut idxs = self.variant_groups[g as usize].clone();
				idxs.sort_unstable();
				(name.clone(), idxs.iter().map(|&i| self.ids[i as usize].clone()).collect())
			})
			.collect();
		groups.sort_by(|a, b| a.0.cmp(&b.0));
		let wrote_m = write_if_changed(&dir.join("tiles.match.json"), serialize_matches(&self.matches).as_bytes())?;
		let wrote_v = write_if_changed(&dir.join("tiles.variants.json"), serialize_variants(&groups).as_bytes())?;
		Ok(wrote_m || wrote_v)
	}

	/// Write `tiles-data.json` (the id table) + `tiles.pass.json` (when present) -
	/// the match editor's id-rename / pass save. Returns whether anything changed.
	pub fn save_ids_pass(&self, dir: &Path) -> Result<bool, String> {
		let mut wrote = write_if_changed(&dir.join("tiles-data.json"), serialize_ids(&self.ids).as_bytes())?;
		if let Some(pass) = &self.pass {
			wrote |= write_if_changed(&dir.join("tiles.pass.json"), serialize_pass(&self.ids, pass).as_bytes())?;
		}
		Ok(wrote)
	}

	/// Write this pack to `dir/` in the on-disk asset format (`load`'s
	/// inverse). Persists everything held in memory: pixels, the id table, the
	/// palette (when owned), the pass table, and - when non-empty -
	/// `tiles.props.json` + `tiles.variants.json` (so a user pack's mask/kind
	/// and a baked stock pack's variant groups survive a reload). Files this
	/// pack can't reconstruct from memory - `tiles.match.json` (shore
	/// adjacency) and `tiles.patterns.json` (worldgen formations) - are *not*
	/// written, so an existing one on disk is left intact (Bake overwrites a
	/// stock pack's pixels/props without clobbering its match/pattern data).
	pub fn dump(&self, dir: &Path) -> Result<(), String> {
		std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
		let order = self.tile_order();
		write_if_changed(&dir.join("tiles-data.bin"), &self.bin_ordered(&order))?;
		write_if_changed(&dir.join("tiles-data.json"), self.ids_json(&order).as_bytes())?;
		if let Some(colors) = self.palette_json() {
			write_if_changed(&dir.join("palette.json"), colors.as_bytes())?;
		}
		if let Some(pass) = self.pass_json(&order) {
			write_if_changed(&dir.join("tiles.pass.json"), pass.as_bytes())?;
		}
		if let Some(props) = self.props_json() {
			write_if_changed(&dir.join("tiles.props.json"), props.as_bytes())?;
		}
		if let Some(variants) = self.variants_json() {
			write_if_changed(&dir.join("tiles.variants.json"), variants.as_bytes())?;
		}
		write_if_changed(&dir.join("info.json"), format!("{{\"version\":\"{}\"}}", self.version).as_bytes())?;
		Ok(())
	}

	/// Bake the pack's tile data back to `dir/`: the bin + id table reordered to
	/// **ascending id**, a **dense** pass table (every surviving tile, including
	/// passability 0 - Bake never silently drops a tile's movement type), props,
	/// and variants. Writes only the files whose bytes actually differ
	/// (palette / `info.json` / match / pattern files are left as-is). Tiles are
	/// never dropped except those the user deleted from the pack. Returns the
	/// file names written, for the Bake report.
	pub fn bake_changed(&self, dir: &Path) -> Result<Vec<&'static str>, String> {
		std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
		let order = self.tile_order();
		let mut wrote = Vec::new();
		// bin + the id table share `order`, so they're always written together.
		if write_if_changed(&dir.join("tiles-data.bin"), &self.bin_ordered(&order))? {
			wrote.push("tiles-data.bin");
		}
		if write_if_changed(&dir.join("tiles-data.json"), self.ids_json(&order).as_bytes())? {
			wrote.push("tiles-data.json");
		}
		if let Some(pass) = self.pass_json(&order) {
			if write_if_changed(&dir.join("tiles.pass.json"), pass.as_bytes())? {
				wrote.push("tiles.pass.json");
			}
		}
		if let Some(props) = self.props_json() {
			if write_if_changed(&dir.join("tiles.props.json"), props.as_bytes())? {
				wrote.push("tiles.props.json");
			}
		}
		if let Some(variants) = self.variants_json() {
			if write_if_changed(&dir.join("tiles.variants.json"), variants.as_bytes())? {
				wrote.push("tiles.variants.json");
			}
		}
		Ok(wrote)
	}

	/// Tile indices in ascending-id order - Bake reorders the bin + id table by
	/// this, so a pack stays sorted however its tiles were added/cloned.
	fn tile_order(&self) -> Vec<u16> {
		let mut order: Vec<u16> = (0..self.tile_count()).collect();
		order.sort_by(|&a, &b| self.ids[a as usize].cmp(&self.ids[b as usize]));
		order
	}

	/// All tile pixels concatenated in `order`.
	fn bin_ordered(&self, order: &[u16]) -> Vec<u8> {
		let mut out = Vec::with_capacity(self.tiles.len());
		for &i in order {
			out.extend_from_slice(self.tile_pixels(i));
		}
		out
	}

	/// `tiles-data.json` body in `order`. Delegates to [`serialize_ids`] so this
	/// file has exactly one writer: Bake and the match editor's id-rename used to
	/// emit different bytes for the same table (compact-and-unescaped here,
	/// tab-indented there), so alternating between the two tools rewrote the file
	/// back and forth under `write_if_changed` for no semantic change.
	fn ids_json(&self, order: &[u16]) -> String {
		let ordered: Vec<String> = order.iter().map(|&i| self.ids[i as usize].clone()).collect();
		serialize_ids(&ordered)
	}

	fn palette_json(&self) -> Option<String> {
		// The object form the user palettes share; a pack whose file carried no
		// name gets one derived from the pack.
		let palette = self.palette.as_ref()?;
		let name = self.palette_name.clone().unwrap_or_else(|| format!("{} Palette", self.name));
		Some(crate::palette::write_palette(palette, &name))
	}

	/// `tiles.pass.json` body - **dense** (every tile, including passability 0),
	/// in `order`, so a Bake preserves every tile's movement type rather than
	/// dropping the zeros as "defaults".
	fn pass_json(&self, order: &[u16]) -> Option<String> {
		let pass = self.pass.as_ref()?;
		let ids: Vec<String> = order.iter().map(|&i| self.ids[i as usize].clone()).collect();
		let dense: Vec<u8> = order.iter().map(|&i| pass[i as usize]).collect();
		Some(serialize_pass(&ids, &dense))
	}

	/// `tiles.props.json` body, families sorted for a stable (git-friendly) diff.
	fn props_json(&self) -> Option<String> {
		if self.props.is_empty() {
			return None;
		}
		let mut keys: Vec<&String> = self.props.keys().collect();
		keys.sort();
		let entries = keys
			.iter()
			.map(|family| {
				let p = &self.props[*family];
				let mut fields = vec![format!("\"hasVariants\":{}", p.has_variants)];
				if let Some(kind) = p.kind {
					let name = match kind {
						TileKind::Water => "WATER",
						TileKind::Land => "LAND",
						TileKind::Shore => "SHORE",
						TileKind::Obstruction => "OBSTRUCTION",
					};
					fields.push(format!("\"type\":\"{name}\""));
				}
				let t = match p.transformable {
					Transformable::No => "false".to_string(),
					Transformable::Free => "true".to_string(),
					Transformable::Sync => "\"sync\"".to_string(),
					Transformable::Invert => "\"invert\"".to_string(),
				};
				fields.push(format!("\"transformable\":{t}"));
				if p.use_mask_color {
					fields.push("\"useMaskColor\":true".to_string());
				}
				if let Some(m) = p.mask {
					fields.push(format!("\"mask\":{m}"));
				}
				format!("\"{family}\":{{{}}}", fields.join(","))
			})
			.collect::<Vec<_>>()
			.join(",");
		Some(format!("{{{entries}}}"))
	}

	/// `tiles.variants.json` body: group name → its tile ids (sorted by name).
	fn variants_json(&self) -> Option<String> {
		if self.variant_named.is_empty() {
			return None;
		}
		let mut groups: Vec<(&String, &u16)> = self.variant_named.iter().collect();
		groups.sort_by(|a, b| a.0.cmp(b.0));
		let entries = groups
			.into_iter()
			.map(|(name, &g)| {
				let ids = self.variant_groups[g as usize]
					.iter()
					.map(|&i| format!("\"{}\"", self.ids[i as usize]))
					.collect::<Vec<_>>()
					.join(",");
				format!("\"{name}\":[{ids}]")
			})
			.collect::<Vec<_>>()
			.join(",");
		Some(format!("{{{entries}}}"))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn assets_root() -> std::path::PathBuf {
		std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/assets/tilepacks")
	}

	/// Build a pack from in-memory files (no fs) - exercises the reader-injected
	/// `from_reader` split. Sidecars absent from `files` read as missing.
	fn try_pack(bin: Vec<u8>, files: &[(&str, &str)]) -> Result<TilePack, String> {
		let map: HashMap<&str, &str> = files.iter().copied().collect();
		TilePack::from_reader(
			"TEST",
			|f| (f == "tiles-data.bin").then(|| bin.clone()).ok_or(format!("TEST/{f}: absent")),
			|f| Ok(map.get(f).map(|s| s.to_string())),
		)
	}

	#[test]
	fn from_reader_loads_in_memory_and_surfaces_errors() {
		let one = vec![0u8; TILE_DATA_SIZE];
		// A minimal valid pack: one tile, array-form id map.
		let ok = try_pack(one.clone(), &[("tiles-data.json", r#"["GSa000"]"#)]).expect("minimal pack loads");
		assert_eq!(ok.tile_count(), 1);
		assert_eq!(ok.ids[0], "GSa000");
		// bin not a whole number of 64×64 tiles.
		let e = try_pack(vec![0u8; TILE_DATA_SIZE + 1], &[("tiles-data.json", r#"["A"]"#)]).err().unwrap();
		assert!(e.contains("not a multiple"), "{e}");
		// id-count vs bin-tile-count mismatch (2-tile bin, one id).
		let e = try_pack(vec![0u8; TILE_DATA_SIZE * 2], &[("tiles-data.json", r#"["A"]"#)]).err().unwrap();
		assert!(e.contains("entries"), "{e}");
		// object-form id map index out of range.
		let e = try_pack(one.clone(), &[("tiles-data.json", r#"{"5":"A"}"#)]).err().unwrap();
		assert!(e.contains("out of range"), "{e}");
		// the required tiles-data.json is missing.
		let e = try_pack(one.clone(), &[]).err().unwrap();
		assert!(e.contains("tiles-data.json"), "{e}");
		// a malformed optional sidecar surfaces with its own prefix.
		let e = try_pack(one, &[("tiles-data.json", r#"["A"]"#), ("tiles.pass.json", "nope")]).err().unwrap();
		assert!(e.contains("tiles.pass.json"), "{e}");
	}

	/// Parse a serialized `tiles.match.json` string back into the ring-indexed
	/// dir form, mirroring `from_reader`'s mapping (file N/E/S/W → ring).
	fn reparse_matches(text: &str) -> HashMap<String, [Vec<String>; 4]> {
		let value = json::parse(text).expect("valid json");
		let families = value.as_object().expect("object");
		let mut out = HashMap::new();
		for (family, rule) in families {
			let mut dirs: [Vec<String>; 4] = Default::default();
			for (key, dir) in [("N", DIR_N), ("E", DIR_E), ("S", DIR_S), ("W", DIR_W)] {
				let Some(list) = rule.get(key) else { continue };
				for entry in list.as_array().expect("array") {
					dirs[dir].push(entry.as_str().expect("str").to_string());
				}
			}
			out.insert(family.to_string(), dirs);
		}
		out
	}

	/// `serialize_matches` preserves every family/direction's entry set across a
	/// round-trip through the shipped GREEN rules (order is canonicalized, so
	/// compare as sets).
	#[test]
	fn serialize_matches_round_trips_green() {
		let pack = TilePack::load(&assets_root(), "GREEN").expect("GREEN loads");
		let text = serialize_matches(&pack.matches);
		let back = reparse_matches(&text);
		assert_eq!(back.len(), pack.matches.len(), "family count");
		for (fam, rule) in &pack.matches {
			let got = back.get(fam).unwrap_or_else(|| panic!("missing family {fam}"));
			for d in 0..4 {
				let mut a = rule.dirs[d].clone();
				let mut b = got[d].clone();
				a.sort();
				b.sort();
				assert_eq!(a, b, "{fam} dir {d}");
			}
		}
	}

	/// `serialize_variants` preserves the GREEN variant groups across a
	/// round-trip (group name → member id set).
	#[test]
	fn serialize_variants_round_trips_green() {
		let pack = TilePack::load(&assets_root(), "GREEN").expect("GREEN loads");
		let groups: Vec<(String, Vec<String>)> = pack
			.variant_named
			.iter()
			.map(|(name, &g)| {
				(name.clone(), pack.variant_groups[g as usize].iter().map(|&i| pack.ids[i as usize].clone()).collect())
			})
			.collect();
		let text = serialize_variants(&groups);
		let value = json::parse(&text).expect("valid json");
		let obj = value.as_object().expect("object");
		let mut count = 0;
		for (name, list) in obj {
			count += 1;
			let mut got: Vec<String> =
				list.as_array().unwrap().iter().map(|e| e.as_str().unwrap().to_string()).collect();
			let (_, mut want) = groups.iter().find(|(n, _)| n == name).expect("group present").clone();
			got.sort();
			want.sort();
			assert_eq!(got, want, "group {name}");
		}
		assert_eq!(count, groups.len(), "group count");
	}

	/// `set_match_data` rebuilds grouping so `group_of` (and the live engine)
	/// honors a fresh link, even across id-prefixes.
	#[test]
	fn set_match_data_relinks_group_of() {
		let mut pack = TilePack::load(&assets_root(), "GREEN").expect("GREEN loads");
		let donor = pack.index_of["GSa000"]; // family GSa
		// Link GSa000 into the GSh group; everything else keeps its group.
		let mut groups: Vec<(String, Vec<u16>)> = pack
			.variant_named
			.iter()
			.map(|(name, &g)| (name.clone(), pack.variant_groups[g as usize].clone()))
			.collect();
		for (name, tiles) in &mut groups {
			tiles.retain(|&t| t != donor);
			if name == "GSh" {
				tiles.push(donor);
			}
		}
		let matches = pack.matches.clone();
		pack.set_match_data(groups, matches);
		assert_eq!(pack.group_of(donor), "GSh", "GSa000 now resolves to the GSh group");
	}

	/// `save_match_data` writes both sidecars to a directory, and they reparse to
	/// the same content (the full editor → disk → reload path). Writes to a
	/// throwaway dir, never the shipped assets.
	#[test]
	fn save_match_data_writes_reparseable_files() {
		let pack = TilePack::load(&assets_root(), "GREEN").expect("GREEN loads");
		let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/test-matchsave");
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).expect("temp dir");
		let wrote = pack.save_match_data(&dir).expect("save");
		assert!(wrote, "first save writes the files");
		assert!(dir.join("tiles.match.json").exists());
		assert!(dir.join("tiles.variants.json").exists());
		// Reparse the match file and compare entry sets.
		let text = std::fs::read_to_string(dir.join("tiles.match.json")).unwrap();
		let back = reparse_matches(&text);
		for (fam, rule) in &pack.matches {
			let got = back.get(fam).unwrap_or_else(|| panic!("missing {fam}"));
			for d in 0..4 {
				let (mut a, mut b) = (rule.dirs[d].clone(), got[d].clone());
				a.sort();
				b.sort();
				assert_eq!(a, b, "{fam} dir {d}");
			}
		}
		// A second save is a no-op (content unchanged).
		assert!(!pack.save_match_data(&dir).expect("save2"), "unchanged save writes nothing");
		std::fs::remove_dir_all(&dir).ok();
	}

	/// `save_ids_pass` writes a reparseable id table + pass table (the match
	/// editor's id-rename / pass-edit save). Throwaway dir, never shipped assets.
	#[test]
	fn save_ids_pass_round_trips() {
		let mut pack = TilePack::load(&assets_root(), "GREEN").expect("GREEN loads");
		pack.rename_tile(0, "GLz000"); // a staged-style rename applied in memory
		pack.set_tile_pass(0, 3);
		let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/test-idpass");
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).expect("temp dir");
		assert!(pack.save_ids_pass(&dir).expect("save"));
		let ids: json::JsonValue = json::parse(&std::fs::read_to_string(dir.join("tiles-data.json")).unwrap()).unwrap();
		assert_eq!(ids.as_array().unwrap()[0].as_str().unwrap(), "GLz000", "renamed id written");
		let pass: json::JsonValue =
			json::parse(&std::fs::read_to_string(dir.join("tiles.pass.json")).unwrap()).unwrap();
		assert_eq!(pass.get("GLz000").and_then(|v| v.as_f64()).map(|f| f as u8), Some(3), "pass keyed by the new id");
		assert!(!pack.save_ids_pass(&dir).expect("save2"), "unchanged save writes nothing");
		std::fs::remove_dir_all(&dir).ok();
	}

	#[test]
	fn from_reader_surfaces_sidecar_validation_errors() {
		let one = vec![0u8; TILE_DATA_SIZE];
		let ids = r#"["GSa000"]"#;
		let bad = |sidecar: &str, body: &str| {
			try_pack(one.clone(), &[("tiles-data.json", ids), (sidecar, body)]).err().expect("expected an error")
		};
		// tiles.pass.json: id not in the pack, and an out-of-range pass value.
		assert!(bad("tiles.pass.json", r#"{"ZZZ999": 1}"#).contains("unknown tile"));
		assert!(bad("tiles.pass.json", r#"{"GSa000": 9}"#).contains("out of range"));
		// tiles.props.json: unknown kind + bad transformable value.
		assert!(bad("tiles.props.json", r#"{"GSa": {"type": "BOGUS"}}"#).contains("unknown type"));
		assert!(bad("tiles.props.json", r#"{"GSa": {"transformable": 42}}"#).contains("bad transformable"));
		// tiles.patterns.json: declared size doesn't match the rows/cells.
		assert!(
			bad("tiles.patterns.json", r#"[{"name":"P","width":2,"height":1,"pattern":[["GSa000"]]}]"#)
				.contains("mismatch")
		);
		assert!(
			bad("tiles.patterns.json", r#"[{"name":"P","width":1,"height":1,"pattern":[["ZZZ"]]}]"#)
				.contains("unknown tile")
		);
	}

	#[test]
	fn props_load_with_kinds_and_transform_freedom() {
		let green = TilePack::load(&assets_root(), "GREEN").unwrap();
		let gla = green.props.get("GLa").expect("GLa props");
		assert_eq!((gla.kind, gla.has_variants, gla.transformable), (Some(TileKind::Land), true, Transformable::Free));
		let gsa = green.props.get("GSa").expect("GSa props");
		assert_eq!((gsa.kind, gsa.use_mask_color), (Some(TileKind::Shore), true));
		// Shore families carry the transparency mask (color 0); land/obstruction
		// families are opaque (no mask).
		assert_eq!(gsa.mask, Some(0), "shore family masks color 0");
		assert_eq!(gla.mask, None, "land family is opaque");
		assert_eq!(green.props.get("GMa").and_then(|p| p.kind), Some(TileKind::Obstruction));
		assert_eq!(green.props.get("GMa").and_then(|p| p.mask), None, "obstruction is opaque");
		// `tile_mask` resolves a tile index to its family's mask.
		let shore_tile = green.group_tiles("GSa")[0];
		assert_eq!(green.tile_mask(shore_tile), Some(0));
		let land_tile = green.group_tiles("GLa")[0];
		assert_eq!(green.tile_mask(land_tile), None);

		let water = TilePack::load(&assets_root(), "WATER").unwrap();
		let wtr = water.props.get("WTR").expect("WTR props");
		assert_eq!((wtr.kind, wtr.transformable), (Some(TileKind::Water), Transformable::Sync));

		let desert = TilePack::load(&assets_root(), "DESERT").unwrap();
		assert_eq!(desert.props.get("DLb").map(|p| p.transformable), Some(Transformable::Invert));
	}

	#[test]
	fn every_shipped_pack_props_key_names_real_tiles() {
		// Guards against tiles.props.json typos (a SNOW_DARK copy-paste slip
		// once listed DESERT's DMA..DME families): every props key must
		// resolve to tiles - via its variant group (WATER's `WTR`) or its
		// id family.
		for pack_dir in ["CRATER", "DESERT", "GREEN", "SNOW", "SNOW_DARK", "WATER"] {
			let pack = TilePack::load(&assets_root(), pack_dir).unwrap();
			for key in pack.props.keys() {
				assert!(
					!pack.group_tiles(key).is_empty(),
					"{pack_dir}/tiles.props.json: key '{key}' resolves to no tiles",
				);
			}
		}
	}

	#[test]
	fn dump_round_trips_pixels_props_and_variants() {
		// dump → load must preserve pixels, pass, props (kind/transform/mask),
		// and variant groups per id - the data a user pack / a Bake relies on.
		// dump reorders to ascending id, so compare by id, not by index.
		let green = TilePack::load(&assets_root(), "GREEN").unwrap();
		let tmp = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/.dump-test-green");
		let _ = std::fs::remove_dir_all(&tmp);
		green.dump(&tmp).unwrap();
		let back = TilePack::load(tmp.parent().unwrap(), tmp.file_name().unwrap().to_str().unwrap()).unwrap();
		assert_eq!(green.tile_count(), back.tile_count(), "tile count");
		assert!(back.ids.windows(2).all(|w| w[0] <= w[1]), "ids are ascending after dump");
		for (i, id) in green.ids.iter().enumerate() {
			let bi = back.index_of[id];
			assert_eq!(green.tile_pixels(i as u16), back.tile_pixels(bi), "pixels for {id}");
			let gp = green.pass.as_ref().map(|p| p[i]);
			let bp = back.pass.as_ref().map(|p| p[bi as usize]);
			assert_eq!(gp, bp, "pass for {id}");
		}
		let gla = |p: &TilePack| p.props.get("GLa").map(|f| (f.kind, f.has_variants, f.transformable, f.mask));
		assert_eq!(gla(&green), gla(&back), "GLa props");
		assert_eq!(
			green.props.get("GSa").and_then(|p| p.mask),
			back.props.get("GSa").and_then(|p| p.mask),
			"shore mask"
		);
		assert_eq!(green.variant_named.len(), back.variant_named.len(), "variant groups");
		assert_eq!(green.group_tiles("GLa").len(), back.group_tiles("GLa").len(), "GLa group");
		let _ = std::fs::remove_dir_all(&tmp);
	}

	#[test]
	fn bake_changed_writes_only_differing_files_and_sorts() {
		let green = TilePack::load(&assets_root(), "GREEN").unwrap();
		let tmp = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/.bake-changed-green");
		let _ = std::fs::remove_dir_all(&tmp);
		green.dump(&tmp).unwrap(); // sorted baseline on disk
		// Repaint one tile (ids/pass/props/variants unchanged) → only the bin differs.
		let mut edited = green.clone();
		let t = edited.index_of["GLa000"];
		edited.set_tile_pixels(t, &[9u8; TILE_DATA_SIZE]);
		assert_eq!(edited.bake_changed(&tmp).unwrap(), vec!["tiles-data.bin"], "only the bin changed");
		// Re-baking with no change writes nothing (write-if-changed).
		assert!(edited.bake_changed(&tmp).unwrap().is_empty(), "unchanged bytes are not rewritten");
		// The on-disk id table is sorted ascending.
		let ids: Vec<String> = json::parse(&std::fs::read_to_string(tmp.join("tiles-data.json")).unwrap())
			.unwrap()
			.as_array()
			.unwrap()
			.iter()
			.map(|v| v.as_str().unwrap().to_string())
			.collect();
		assert!(ids.windows(2).all(|w| w[0] <= w[1]), "baked id table is ascending");
		let _ = std::fs::remove_dir_all(&tmp);
	}

	#[test]
	fn push_tile_appends_with_pass() {
		let mut pack = TilePack::empty_user("GREEN");
		assert_eq!(pack.tile_count(), 0);
		let i = pack.push_tile("GLa900".to_string(), &[7u8; TILE_DATA_SIZE], 2);
		assert_eq!(i, 0);
		assert_eq!(pack.tile_count(), 1);
		assert_eq!(pack.index_of.get("GLa900"), Some(&0));
		assert_eq!(pack.tile_pixels(0), &[7u8; TILE_DATA_SIZE]);
		assert_eq!(pack.pass.as_ref().map(|p| p[0]), Some(2));
		assert!(pack.user);
	}

	#[test]
	fn replace_id_token_is_boundary_precise() {
		// Whole-token only: renaming WTR00 must not touch WTR000.
		let (out, n) = replace_id_token(r#"["WTR000","WTR00:!N","GSd004"]"#, "WTR00", "XYZ00");
		assert_eq!(n, 1);
		assert_eq!(out, r#"["WTR000","XYZ00:!N","GSd004"]"#);
		// Either end of a comma-separated multi-tile cell.
		let (o2, c2) = replace_id_token("\"WTR000,GSd004:!N\"", "GSd004", "GSd009");
		assert_eq!((o2.as_str(), c2), ("\"WTR000,GSd009:!N\"", 1));
		let (o3, c3) = replace_id_token("\"GSd004,WTR000\"", "GSd004", "GSd009");
		assert_eq!((o3.as_str(), c3), ("\"GSd009,WTR000\"", 1));
		// No-op when old==new or absent.
		assert_eq!(replace_id_token("\"GLa000\"", "GLa000", "GLa000").1, 0);
		assert_eq!(replace_id_token("\"GLa000\"", "ZZZ999", "X").1, 0);
	}

	#[test]
	fn family_of_strips_variant_digits() {
		assert_eq!(family_of("GSh004"), "GSh");
		assert_eq!(family_of("WTR003"), "WTR");
		assert_eq!(family_of("SLA000"), "SLA");
	}

	#[test]
	fn match_rule_water_wildcard_queries() {
		let rule = MatchRule {
			dirs: [
				vec!["__WATER__".to_string()],                    // N: water only
				vec!["__WATER__".to_string(), "GSa".to_string()], // E: water allowed, not required
				vec!["GSa".to_string()],                          // S: no water
				vec![],                                           // W: nothing listed
			],
		};
		assert!(rule.allows_water(DIR_N) && rule.requires_water(DIR_N), "a lone __WATER__ both allows and requires");
		assert!(rule.allows_water(DIR_E) && !rule.requires_water(DIR_E), "mixed list allows but doesn't require");
		assert!(!rule.allows_water(DIR_S) && !rule.requires_water(DIR_S), "no wildcard -> no water");
		assert!(!rule.allows_water(DIR_W) && !rule.requires_water(DIR_W), "empty dir -> no water");
	}

	/// `json_string` must emit a valid JSON literal for arbitrary input, not
	/// just the ASCII id alphabet (its documented safety margin).
	#[test]
	fn json_string_escapes_the_specials() {
		assert_eq!(json_string("GSa000"), "\"GSa000\"");
		assert_eq!(json_string("a\"b\\c\nd\te"), "\"a\\\"b\\\\c\\nd\\te\"");
	}

	/// Canonical match-list order (wildcards first, then groups by name and
	/// transform bits) and the `[]` form for an empty direction.
	#[test]
	fn serialize_matches_orders_entries_and_writes_empty_dirs() {
		let mut matches = HashMap::new();
		matches.insert(
			"AAA".to_string(),
			MatchRule {
				dirs: [
					vec!["GSa:!N".to_string(), "GSa".to_string(), "__WATER__".to_string()], // N (unsorted)
					vec![],                                                                 // E
					vec![],                                                                 // S
					vec![],                                                                 // W
				],
			},
		);
		let text = serialize_matches(&matches);
		let n_at = text.find("\"__WATER__\"").expect("wildcard present");
		let base_at = text.find("\"GSa\"").expect("identity entry present");
		let mirrored_at = text.find("\"GSa:!N\"").expect("mirrored entry present");
		assert!(n_at < base_at && base_at < mirrored_at, "wildcard, then identity, then mirrored:\n{text}");
		assert!(text.contains("\"W\": []"), "an empty direction serializes as []:\n{text}");
		// And the whole thing still reparses to the same entry sets.
		let back = reparse_matches(&text);
		assert_eq!(back["AAA"][DIR_N].len(), 3);
		assert!(back["AAA"][DIR_W].is_empty());
	}

	#[test]
	fn object_form_id_map_loads_and_scalar_form_is_rejected() {
		let one = vec![0u8; TILE_DATA_SIZE];
		// `{ "0": "SCa000" }` - the object shape some shipped packs use.
		let ok = try_pack(one.clone(), &[("tiles-data.json", r#"{"0": "SCa000"}"#)]).expect("object form loads");
		assert_eq!(ok.ids[0], "SCa000");
		assert_eq!(ok.index_of.get("SCa000"), Some(&0));
		// A bare scalar is neither an array nor an object.
		let e = try_pack(one, &[("tiles-data.json", r#""SCa000""#)]).err().unwrap();
		assert!(e.contains("not an array or object"), "{e}");
	}

	/// A variants group whose ids all miss the pack is dropped (no empty
	/// groups), and a tile in no group reports no variants.
	#[test]
	fn variants_with_only_unknown_ids_are_dropped() {
		let two = vec![0u8; TILE_DATA_SIZE * 2];
		let pack = try_pack(
			two,
			&[
				("tiles-data.json", r#"["GSa000", "GSa001"]"#),
				("tiles.variants.json", r#"{"GHOST": ["ZZZ000", "ZZZ001"], "GSa": ["GSa000", "GSa001"]}"#),
			],
		)
		.expect("pack loads");
		assert!(!pack.variant_named.contains_key("GHOST"), "the all-unknown group is dropped");
		assert_eq!(pack.variants_of(0).len(), 2, "the resolvable group survives");
	}

	#[test]
	fn variants_of_without_a_group_is_empty() {
		let mut pack = TilePack::empty_user("GREEN");
		pack.push_tile("GLa900".to_string(), &[0u8; TILE_DATA_SIZE], 0);
		assert!(pack.variants_of(0).is_empty(), "a groupless tile has no variants");
	}

	/// When a tile's variant-group name has no props entry, `tile_props` falls
	/// back to the tile's id family (the documented resolution order).
	#[test]
	fn tile_props_falls_back_to_the_id_family() {
		let one = vec![0u8; TILE_DATA_SIZE];
		let pack = try_pack(
			one,
			&[
				("tiles-data.json", r#"["GSa000"]"#),
				("tiles.variants.json", r#"{"LINKED": ["GSa000"]}"#),
				("tiles.props.json", r#"{"GSa": {"type": "SHORE", "mask": 5}}"#),
			],
		)
		.expect("pack loads");
		assert_eq!(pack.group_of(0), "LINKED", "the tile resolves to its group name");
		let props = pack.tile_props(0).expect("family props found via the fallback");
		assert_eq!(props.kind, Some(TileKind::Shore), "props came from the GSa family entry");
		assert_eq!(pack.tile_mask(0), Some(5), "the mask resolves through the family too");
	}

	#[test]
	fn transformable_defaults_to_no_when_absent() {
		let one = vec![0u8; TILE_DATA_SIZE];
		let pack = try_pack(
			one,
			&[("tiles-data.json", r#"["GSa000"]"#), ("tiles.props.json", r#"{"GSa": {"type": "LAND"}}"#)],
		)
		.expect("pack loads");
		assert_eq!(pack.tile_transformable(0), Transformable::No, "no `transformable` key -> No");
	}

	#[test]
	fn pattern_row_count_must_match_height() {
		let one = vec![0u8; TILE_DATA_SIZE];
		let e = try_pack(
			one,
			&[
				("tiles-data.json", r#"["GSa000"]"#),
				("tiles.patterns.json", r#"[{"name":"P","width":1,"height":2,"pattern":[["GSa000"]]}]"#),
			],
		)
		.err()
		.unwrap();
		assert!(e.contains("size mismatch"), "1 row against height 2: {e}");
	}

	/// `push_tile`'s pass-table paths: an existing table grows, and a pass-0
	/// push on a pass-less pack allocates nothing (0 is the default anyway).
	#[test]
	fn push_tile_pass_table_paths() {
		let mut pack = TilePack::empty_user("GREEN");
		pack.push_tile("GLa900".to_string(), &[1u8; TILE_DATA_SIZE], 2); // allocates [2]
		pack.push_tile("GLa901".to_string(), &[2u8; TILE_DATA_SIZE], 3); // appends to it
		assert_eq!(pack.pass.as_deref(), Some(&[2u8, 3][..]), "the existing table grew");
		let mut plain = TilePack::empty_user("GREEN");
		plain.push_tile("GLa900".to_string(), &[1u8; TILE_DATA_SIZE], 0);
		assert!(plain.pass.is_none(), "an all-default pass table is not allocated");
	}

	#[test]
	fn set_match_data_drops_empty_and_out_of_range_groups() {
		let mut pack = TilePack::empty_user("GREEN");
		pack.push_tile("GLa900".to_string(), &[0u8; TILE_DATA_SIZE], 0);
		pack.set_match_data(vec![("GHOST".to_string(), vec![99, 200]), ("GLa".to_string(), vec![0])], HashMap::new());
		assert!(!pack.variant_named.contains_key("GHOST"), "a group of out-of-range tiles is dropped");
		assert_eq!(pack.group_of(0), "GLa", "the valid group registered");
	}

	/// `save_ids_pass` on a pack without a pass table writes only the id table.
	#[test]
	fn save_ids_pass_without_a_pass_table() {
		let mut pack = TilePack::empty_user("GREEN");
		pack.push_tile("GLa900".to_string(), &[0u8; TILE_DATA_SIZE], 0);
		let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/mc-cov-idsonly");
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).expect("temp dir");
		assert!(pack.save_ids_pass(&dir).expect("save"), "the id table was written");
		assert!(dir.join("tiles-data.json").exists());
		assert!(!dir.join("tiles.pass.json").exists(), "no pass table -> no pass file");
		std::fs::remove_dir_all(&dir).ok();
	}

	/// The props kinds/transformables the shipped packs never dump (WATER,
	/// sync, invert, an entry with no type) survive a dump → load round trip;
	/// absent palette/pass files are simply not written.
	#[test]
	fn dump_round_trips_every_kind_and_transformable() {
		let three = vec![0u8; TILE_DATA_SIZE * 3];
		let pack = try_pack(
			three,
			&[
				("tiles-data.json", r#"["WAT000", "INV000", "NOK000"]"#),
				(
					"tiles.props.json",
					r#"{"WAT": {"type": "WATER", "transformable": "sync"},
					    "INV": {"transformable": "invert"},
					    "NOK": {"hasVariants": true}}"#,
				),
			],
		)
		.expect("pack loads");
		let tmp = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/mc-cov-dump-kinds");
		let _ = std::fs::remove_dir_all(&tmp);
		pack.dump(&tmp).unwrap();
		assert!(!tmp.join("palette.json").exists(), "no palette -> not written");
		assert!(!tmp.join("tiles.pass.json").exists(), "no pass table -> not written");
		let back = TilePack::load(tmp.parent().unwrap(), tmp.file_name().unwrap().to_str().unwrap()).unwrap();
		let wat = back.props.get("WAT").expect("WAT props survive");
		assert_eq!((wat.kind, wat.transformable), (Some(TileKind::Water), Transformable::Sync));
		assert_eq!(back.props.get("INV").map(|p| p.transformable), Some(Transformable::Invert));
		let nok = back.props.get("NOK").expect("NOK props survive");
		assert_eq!((nok.kind, nok.has_variants), (None, true), "a kind-less entry stays kind-less");
		let _ = std::fs::remove_dir_all(&tmp);
	}

	/// Baking into a fresh directory reports every sidecar it wrote.
	#[test]
	fn bake_changed_into_a_fresh_dir_writes_every_file() {
		let green = TilePack::load(&assets_root(), "GREEN").unwrap();
		let tmp = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/mc-cov-bake-fresh");
		let _ = std::fs::remove_dir_all(&tmp);
		let wrote = green.bake_changed(&tmp).unwrap();
		for file in ["tiles-data.bin", "tiles-data.json", "tiles.pass.json", "tiles.props.json", "tiles.variants.json"]
		{
			assert!(wrote.contains(&file), "{file} written on a fresh bake: {wrote:?}");
		}
		let _ = std::fs::remove_dir_all(&tmp);

		// A pack with none of the optional tables writes only the bin + ids.
		let mut bare = TilePack::empty_user("BARE");
		bare.push_tile("BLa000".to_string(), &[0u8; TILE_DATA_SIZE], 0);
		let tmp2 = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/mc-cov-bake-bare");
		let _ = std::fs::remove_dir_all(&tmp2);
		assert_eq!(bare.bake_changed(&tmp2).unwrap(), vec!["tiles-data.bin", "tiles-data.json"]);
		let _ = std::fs::remove_dir_all(&tmp2);
	}

	/// One writer per file, so the two tools that write the id/pass tables agree
	/// byte for byte with what is shipped. They used to disagree - Bake emitted a
	/// compact single line, the match editor's save emitted tab-indented rows -
	/// so alternating between them rewrote these files back and forth under
	/// `write_if_changed`, churning the repo for no semantic change.
	#[test]
	fn baking_a_shipped_pack_reproduces_its_id_and_pass_files_exactly() {
		let root = assets_root();
		if !root.join("GREEN").is_dir() {
			eprintln!("skipping: no shipped packs at {}", root.display());
			return;
		}
		for name in ["GREEN", "SNOW", "DESERT", "CRATER"] {
			let pack = TilePack::load(&root, name).expect("shipped pack loads");
			let order = pack.tile_order();
			let on_disk = std::fs::read_to_string(root.join(name).join("tiles-data.json")).unwrap();
			assert_eq!(pack.ids_json(&order), on_disk, "{name}: a Bake would rewrite tiles-data.json");
			if let Some(pass) = pack.pass_json(&order) {
				let on_disk = std::fs::read_to_string(root.join(name).join("tiles.pass.json")).unwrap();
				assert_eq!(pass, on_disk, "{name}: a Bake would rewrite tiles.pass.json");
			}
			// The match editor's save path must produce those same bytes.
			assert_eq!(serialize_ids(&pack.ids), pack.ids_json(&order), "{name}: the two id writers disagree");
		}
	}
}
