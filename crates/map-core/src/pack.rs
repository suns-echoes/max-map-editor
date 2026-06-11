//! Tile pack loading (`resources/assets/<PACK>/`) — see
//! `docs/design/tileset-contract.md` §2. Loads what rendering and composition
//! need today: tile pixels, the index→id table, the optional palette, pass
//! values, and the shore adjacency rules.
//! (props/variants join when the tools that consume them land.)

use std::collections::HashMap;
use std::path::Path;

use max_assets::wrl::TILE_DATA_SIZE;

/// Directions are ring-indexed clockwise: N=0, E=1, S=2, W=3 (`shore.rs`
/// rotates them with this arithmetic).
pub const DIR_N: usize = 0;
pub const DIR_E: usize = 1;
pub const DIR_S: usize = 2;
pub const DIR_W: usize = 3;

/// A tile id's family: the id with its variant digits removed
/// (`"GSh004"` → `"GSh"`). Families key the match rules, variant groups,
/// and props.
pub fn family_of(id: &str) -> &str {
	id.trim_end_matches(|c: char| c.is_ascii_digit())
}

/// Semantic class of a tile family from `tiles.props.json` — what a family
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
	/// in sync — even ones already painted (`"sync"`).
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
}

/// One multi-tile formation from `tiles.patterns.json` — extracted from the
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
/// orientation: per direction, the allowed neighbors — tile specs like
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
	/// 256×RGB — present only on palette-owning packs.
	pub palette: Option<Vec<u8>>,
	/// Per-tile passability (0 land / 1 water / 2 shore / 3 blocked),
	/// indexed by bin index — from `tiles.pass.json` (recovered from the
	/// original WRL passtabs). `None` when the pack ships without it.
	pub pass: Option<Vec<u8>>,
	/// Family (`"GSa"`) → adjacency rules, from `tiles.match.json`
	/// (auto-shore, tile suggestions, diagnostics).
	/// Empty when the pack ships without it.
	pub matches: HashMap<String, MatchRule>,
	/// Interchangeable look-variant groups (lists of tile indices) from
	/// `tiles.variants.json` — the random-paint toggle picks among a
	/// tile's siblings. Empty when the pack ships without it. Read via
	/// [`TilePack::variants_of`].
	pub variant_groups: Vec<Vec<u16>>,
	/// Tile index → its `variant_groups` index (`None` = no variants).
	pub variant_of: Vec<Option<u16>>,
	/// Variant-group name → `variant_groups` index (group names usually
	/// match tile families, but not always: WATER's group is `WTR`, its
	/// tile ids `WATR00…`).
	pub variant_named: HashMap<String, u16>,
	/// Group key → semantic props from `tiles.props.json` (worldgen,
	/// transform guards). Keys are variant-group names or tile-id families —
	/// resolve tiles via [`TilePack::group_tiles`]. Empty when the pack
	/// ships without it.
	pub props: HashMap<String, FamilyProps>,
	/// Multi-tile formations from `tiles.patterns.json` (worldgen).
	/// Empty when the pack ships without it.
	pub patterns: Vec<TilePattern>,
}

impl TilePack {
	pub fn tile_count(&self) -> u16 {
		(self.tiles.len() / TILE_DATA_SIZE) as u16
	}

	pub fn tile_pixels(&self, index: u16) -> &[u8] {
		let at = index as usize * TILE_DATA_SIZE;
		&self.tiles[at..at + TILE_DATA_SIZE]
	}

	pub fn load(assets_root: &Path, name: &str) -> Result<Self, String> {
		let dir = assets_root.join(name);
		let read = |file: &str| std::fs::read_to_string(dir.join(file)).map_err(|e| format!("{name}/{file}: {e}"));

		let tiles = std::fs::read(dir.join("tiles-data.bin")).map_err(|e| format!("{name}/tiles-data.bin: {e}"))?;
		if tiles.len() % TILE_DATA_SIZE != 0 {
			return Err(format!("{name}/tiles-data.bin: not a multiple of {TILE_DATA_SIZE}"));
		}
		let tile_count = tiles.len() / TILE_DATA_SIZE;

		// tiles-data.json: bin index → tile id, in either shape found in the
		// shipped packs: `["WATR00", …]` (index = position) or
		// `{ "0": "SCa000", … }`. TODO: normalize the packs to one shape.
		let id_map = json::parse(&read("tiles-data.json")?).map_err(|e| format!("{name}/tiles-data.json: {e}"))?;
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

		// palette.json: ["#rrggbb", ...] — optional (WATER has none).
		let palette = match std::fs::read_to_string(dir.join("palette.json")) {
			Err(_) => None,
			Ok(text) => {
				let value = json::parse(&text).map_err(|e| format!("{name}/palette.json: {e}"))?;
				let colors = value.as_array().ok_or(format!("{name}/palette.json: not an array"))?;
				if colors.len() != 256 {
					return Err(format!("{name}/palette.json: {} colors, want 256", colors.len()));
				}
				let mut rgb = Vec::with_capacity(768);
				for color in colors {
					let hex = color
						.as_str()
						.and_then(|s| s.strip_prefix('#'))
						.ok_or(format!("{name}/palette.json: bad color entry"))?;
					if hex.len() != 6 && hex.len() != 8 {
						return Err(format!("{name}/palette.json: bad color '#{hex}'"));
					}
					for i in 0..3 {
						let byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
							.map_err(|_| format!("{name}/palette.json: bad hex '#{hex}'"))?;
						rgb.push(byte);
					}
				}
				Some(rgb)
			}
		};

		// info.json: pack metadata — only `version` is consumed here.
		let version = std::fs::read_to_string(dir.join("info.json"))
			.ok()
			.and_then(|text| json::parse(&text).ok())
			.and_then(|v| v.get("version").and_then(|v| v.as_str().map(String::from)))
			.unwrap_or_else(|| "1".to_string());

		// tiles.pass.json: { "GSd004": 2, ... } — optional.
		let pass = match std::fs::read_to_string(dir.join("tiles.pass.json")) {
			Err(_) => None,
			Ok(text) => {
				let value = json::parse(&text).map_err(|e| format!("{name}/tiles.pass.json: {e}"))?;
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

		// tiles.match.json: { "GSa": { "N": [...], "W": [...], ... } } —
		// optional. File order is N/W/S/E; stored ring-indexed N/E/S/W.
		let mut matches = HashMap::new();
		if let Ok(text) = std::fs::read_to_string(dir.join("tiles.match.json")) {
			let value = json::parse(&text).map_err(|e| format!("{name}/tiles.match.json: {e}"))?;
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

		// tiles.variants.json: { "GSa": ["GSa000", ...] } — optional. Each list
		// is a set of interchangeable look-variants; map the ids to bin indices
		// and record which group each tile belongs to.
		let mut variant_groups: Vec<Vec<u16>> = Vec::new();
		let mut variant_of: Vec<Option<u16>> = vec![None; tile_count];
		let mut variant_named: HashMap<String, u16> = HashMap::new();
		if let Ok(text) = std::fs::read_to_string(dir.join("tiles.variants.json")) {
			let value = json::parse(&text).map_err(|e| format!("{name}/tiles.variants.json: {e}"))?;
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
		// "transformable": true, "useMaskColor": false } } — optional.
		// Semantic family classes for editor tools (worldgen).
		let mut props = HashMap::new();
		if let Ok(text) = std::fs::read_to_string(dir.join("tiles.props.json")) {
			let value = json::parse(&text).map_err(|e| format!("{name}/tiles.props.json: {e}"))?;
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
					},
				);
			}
		}

		// tiles.patterns.json: [{ "name", "width", "height", "pattern":
		// [["CMa000", null, …], …] }] — optional. Formations extracted from
		// the original maps; `null` cells are holes.
		let mut patterns = Vec::new();
		if let Ok(text) = std::fs::read_to_string(dir.join("tiles.patterns.json")) {
			let value = json::parse(&text).map_err(|e| format!("{name}/tiles.patterns.json: {e}"))?;
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
			pass,
			matches,
			variant_groups,
			variant_of,
			variant_named,
			props,
			patterns,
		})
	}

	/// The tiles a props/variants group key covers: the variant group
	/// registered under that name when one exists (WATER's `WTR` group holds
	/// the `WATR…` tiles), else every tile whose id family matches the key.
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

	/// Write this pack to `dir/` in the on-disk asset format (`load`'s
	/// inverse) — used to persist a synthetic pack built by
	/// `Project::from_wrl` next to a saved `.json`. Only data known with
	/// certainty is written: `tiles.match.json` (shore adjacency) and other
	/// editor metadata are omitted, since they can't be inferred from a flat
	/// WRL. Match rules etc. fill in later as the import improves.
	pub fn dump(&self, dir: &Path) -> Result<(), String> {
		std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
		let write = |file: &str, data: &[u8]| {
			std::fs::write(dir.join(file), data).map_err(|e| format!("{}/{file}: {e}", self.name))
		};

		write("tiles-data.bin", &self.tiles)?;

		let ids = self.ids.iter().map(|id| format!("\"{id}\"")).collect::<Vec<_>>().join(",");
		write("tiles-data.json", format!("[{ids}]").as_bytes())?;

		if let Some(palette) = &self.palette {
			let colors = palette
				.chunks_exact(3)
				.map(|c| format!("\"#{:02x}{:02x}{:02x}\"", c[0], c[1], c[2]))
				.collect::<Vec<_>>()
				.join(",");
			write("palette.json", format!("[{colors}]").as_bytes())?;
		}

		if let Some(pass) = &self.pass {
			// Sparse — the loader defaults the unlisted tiles to 0 (land).
			let entries = pass
				.iter()
				.enumerate()
				.filter(|&(_, &v)| v != 0)
				.map(|(i, v)| format!("\"{}\":{}", self.ids[i], v))
				.collect::<Vec<_>>()
				.join(",");
			write("tiles.pass.json", format!("{{{entries}}}").as_bytes())?;
		}

		write("info.json", format!("{{\"version\":\"{}\"}}", self.version).as_bytes())?;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn assets_root() -> std::path::PathBuf {
		std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/assets")
	}

	#[test]
	fn props_load_with_kinds_and_transform_freedom() {
		let green = TilePack::load(&assets_root(), "GREEN").unwrap();
		let gla = green.props.get("GLa").expect("GLa props");
		assert_eq!((gla.kind, gla.has_variants, gla.transformable), (Some(TileKind::Land), true, Transformable::Free));
		let gsa = green.props.get("GSa").expect("GSa props");
		assert_eq!((gsa.kind, gsa.use_mask_color), (Some(TileKind::Shore), true));
		assert_eq!(green.props.get("GMa").and_then(|p| p.kind), Some(TileKind::Obstruction));

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
		// resolve to tiles — via its variant group (WATER's `WTR`) or its
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
	fn family_of_strips_variant_digits() {
		assert_eq!(family_of("GSh004"), "GSh");
		assert_eq!(family_of("WATR03"), "WATR");
		assert_eq!(family_of("SLA000"), "SLA");
	}
}
