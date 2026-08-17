//! DEV tool: build a map that lays out a tileset's match data as match-editor
//! style crosses - one per tile - with **every** tile taking a turn as the
//! centre, and on each ruled side the first tile that matches it (transform
//! applied). Crosses are gridded with a one-tile gap between them, and the map
//! is sized to host them all, so the whole `tiles.match.json` for a pack can be
//! eyeballed at a glance for correctness.
//!
//! Placement is deliberately the rule's own definition (centre at the base
//! orientation, neighbour at the spec's transform on that side), so a seam that
//! looks wrong here *is* wrong in the data.

use std::path::Path;

use crate::pack::TilePack;
use crate::project::{LAYER_GROUND, Project, TileRef, Transform};

/// Ring direction deltas, indexed N=0, E=1, S=2, W=3 (matching `MatchRule.dirs`).
const DELTA: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

/// One cross: a centre tile and, per ring direction, the first matching
/// neighbour (tile bin index + its transform) in the same pack.
struct Cross {
	center: u16,
	sides: [Option<(u16, Transform)>; 4],
}

/// One cross for **every tile** whose group has at least one matching tile
/// neighbour (tile order, deterministic). The centre is the tile itself; each
/// ruled side of its group takes the first non-wildcard spec whose candidate
/// group resolves to a real tile. Tiles whose group has no rule, or only
/// `__WATER__` / `__LAND__` wildcards (e.g. the all-water `WTR` group), have no
/// pairing to show and are skipped.
fn crosses(pack: &TilePack) -> Vec<Cross> {
	let mut out = Vec::new();
	for center in 0..pack.tile_count() {
		let Some(rule) = pack.matches.get(pack.group_of(center)) else { continue };
		let mut sides: [Option<(u16, Transform)>; 4] = Default::default();
		for (d, side) in sides.iter_mut().enumerate() {
			for spec in &rule.dirs[d] {
				if spec.starts_with("__") {
					continue; // __WATER__ / __LAND__ wildcards aren't tile pairings
				}
				let (id, t) = match spec.split_once(':') {
					Some((id, t)) => match Transform::parse(t) {
						Ok(t) => (id, t),
						Err(_) => continue,
					},
					None => (spec.as_str(), Transform::default()),
				};
				if let Some(&tile) = pack.group_tiles(id).first() {
					*side = Some((tile, t));
					break; // first matching candidate on this side
				}
			}
		}
		if sides.iter().any(Option::is_some) {
			out.push(Cross { center, sides });
		}
	}
	out
}

/// Build a new map for `pack_name` laying out every matching tile as a cross
/// (see the module docs). Crosses tile a square-ish grid on a 4-cell stride (a
/// 3×3 cross + a one-tile gap), sized to host them all, over the default
/// all-water fill so the gaps read as clear separators. The pack's land tiles
/// sit on the ground layer.
pub fn match_combos_map(pack_name: &str, assets_root: &Path, seed: u64) -> Result<Project, String> {
	let pack = TilePack::load(assets_root, pack_name).map_err(|e| format!("load {pack_name}: {e}"))?;
	let crosses = crosses(&pack);
	if crosses.is_empty() {
		return Err(format!("{pack_name}: no tile match rules to lay out"));
	}
	let n = crosses.len();
	// Square-ish grid: smallest `cols` with cols² ≥ n (integer ceil-sqrt, no trig).
	let mut cols = 1usize;
	while cols * cols < n {
		cols += 1;
	}
	let rows = n.div_ceil(cols);
	// 3×3 cross + 1-tile gap = a 4-cell stride, plus a 1-tile margin all round.
	let (w, h) = (4 * cols + 1, 4 * rows + 1);
	if w > 1024 || h > 1024 {
		return Err(format!("{pack_name}: {n} crosses need a {w}x{h} map (max 1024)"));
	}

	let mut project = Project::new(w as u16, h as u16, &[pack_name.to_string()], assets_root, seed)?;
	let pack_idx = project
		.packs
		.iter()
		.position(|p| p.name == pack_name)
		.ok_or_else(|| format!("{pack_name} missing from the new project"))? as u8;
	let tref = |tile, transform| Some(TileRef { pack: pack_idx, tile, transform });

	let mut edits = Vec::with_capacity(n * 5);
	for (k, cross) in crosses.iter().enumerate() {
		let (cx, cy) = (1 + 4 * (k % cols) + 1, 1 + 4 * (k / cols) + 1); // centre cell
		edits.push((cx as u16, cy as u16, LAYER_GROUND, tref(cross.center, Transform::default())));
		for (d, side) in cross.sides.iter().enumerate() {
			if let &Some((tile, t)) = side {
				let (nx, ny) = (cx as i32 + DELTA[d].0, cy as i32 + DELTA[d].1);
				edits.push((nx as u16, ny as u16, LAYER_GROUND, tref(tile, t)));
			}
		}
	}
	project.place_many(&edits);
	project.name = format!("{pack_name} match combos");
	Ok(project)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::project::LAYER_WATER;

	fn assets_root() -> std::path::PathBuf {
		std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/assets/tilepacks")
	}

	#[test]
	fn green_combos_map_places_crosses_on_water() {
		let p = match_combos_map("GREEN", &assets_root(), 1).unwrap();
		assert!(p.width >= 5 && p.height >= 5, "sized to fit at least one cross");
		// Every cell carries the all-water base on the bottom layer.
		assert!(p.cells.iter().all(|s| s[LAYER_WATER].is_some()), "water fill beneath");
		// Some ground tiles were placed (the crosses), but not the whole map -
		// the gaps and cross corners stay clear.
		let ground = p.cells.iter().filter(|s| s[LAYER_GROUND].is_some()).count();
		assert!(ground > 0, "crosses placed on the ground layer");
		assert!(ground < p.cells.len(), "gaps between crosses stay empty");
		// Centre tiles sit at the base orientation; only neighbours carry a spec
		// transform. At least one neighbour exists (GREEN has real match rules).
		assert!(p.name.contains("match combos"));
	}

	#[test]
	fn unmatched_pack_is_reported() {
		// WATER's only group is the all-`__WATER__` WTR family - no tile pairings.
		assert!(match_combos_map("WATER", &assets_root(), 1).is_err());
	}

	/// Match specs that resolve to nothing - a broken transform suffix
	/// (`AAA:bogus`) and a group with no tiles (`ZZZ`) - are skipped rather than
	/// panicking, and a pack whose every spec fails ends up with no crosses.
	#[test]
	fn unresolvable_specs_are_skipped_not_fatal() {
		use max_assets::wrl::TILE_DATA_SIZE;
		let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/mc-cov-combos");
		let _ = std::fs::remove_dir_all(&root);
		let dir = root.join("BADSPEC");
		std::fs::create_dir_all(&dir).unwrap();
		std::fs::write(dir.join("tiles-data.bin"), vec![0u8; TILE_DATA_SIZE]).unwrap();
		std::fs::write(dir.join("tiles-data.json"), r#"["AAA000"]"#).unwrap();
		std::fs::write(
			dir.join("tiles.match.json"),
			r#"{"AAA": {"N": ["AAA:bogus", "ZZZ"], "W": [], "S": [], "E": []}}"#,
		)
		.unwrap();
		let err = match_combos_map("BADSPEC", &root, 1).err().unwrap();
		assert!(err.contains("no tile match rules"), "every spec failed -> no crosses to lay out: {err}");
		std::fs::remove_dir_all(&root).ok();
	}
}
