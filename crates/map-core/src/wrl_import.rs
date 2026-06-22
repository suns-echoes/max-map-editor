//! Import a WRL map *onto existing tilesets* - the smart counterpart to
//! [`Project::from_wrl`], which keeps the WRL's own tiles as a synthetic pack.
//!
//! A WRL "done with standard tiles" stores each cell as a final composited
//! 64×64 tile. This importer rebuilds the map in terms of the user's chosen
//! tilepacks by matching every *used* WRL tile against the packs' tiles by
//! palette index, then re-expressing the cells as references into those packs
//! (so the full editor - auto-shore, variants, repaints - applies again).
//!
//! Matching wildcards two pixel classes so a match survives cosmetic
//! differences a byte-compare would trip on:
//!   * the **animated coastal-water band** ([`COASTAL_WATER`], 96..=116) - the
//!     game re-tints it each frame, so a WRL bakes one phase into a coastal
//!     tile while the pack stores another (or the mask color);
//!   * each pack tile's own **shore mask index** (`tiles.props.json` `"mask"`,
//!     which is `0`) - the editor's shore tiles leave the water-show-through
//!     pixels transparent there, where the WRL baked animated water.
//!
//! Tiles with no match are reported as [`UnmappedTile`]s; the caller decides
//! via [`ExtrasDest`] whether to drop them, bundle them into a project-local
//! pack, or fold them into a reusable user tileset (deduped by exact pixels +
//! pass).

use std::collections::HashMap;
use std::path::Path;

use max_assets::wrl::{TILE_DATA_SIZE, WrlFile};

use crate::pack::TilePack;
use crate::project::{
	LAYER_GROUND, LAYER_WATER, Project, TileRef, Transform, pack_prefix, pass_class, pass_layer, transform_tile,
};

/// The 8 tile orientations (4 rotations × unmirrored/mirrored), identity first
/// so an untransformed match is preferred when several orientations collide.
const ORIENTATIONS: [Transform; 8] = [
	Transform { rot: 0, mirror: false },
	Transform { rot: 1, mirror: false },
	Transform { rot: 2, mirror: false },
	Transform { rot: 3, mirror: false },
	Transform { rot: 0, mirror: true },
	Transform { rot: 1, mirror: true },
	Transform { rot: 2, mirror: true },
	Transform { rot: 3, mirror: true },
];

/// The animated coastal-water palette band (contract §1, the first three water
/// cycles). Wildcarded on both sides of a match so an imported coastal tile
/// matches the pack's canonical art regardless of which animation phase the
/// WRL happened to bake in.
pub const COASTAL_WATER: std::ops::RangeInclusive<u8> = 96..=116;

/// The value every wildcarded pixel collapses to. It sits *inside*
/// [`COASTAL_WATER`], so no genuine non-water pixel can equal it after
/// canonicalisation - a pack tile's mask-`0` pixel and a WRL tile's animated
/// water pixel both fold here and compare equal, while real art is untouched.
const WILD: u8 = 96;

/// Where the unmatched ("missing") WRL tiles should go when the import commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtrasDest {
	/// Drop them - cells that used them keep only the water base.
	Ignore,
	/// Bundle them into a synthetic pack carried by this project (dumped beside
	/// the `.json` on save, like a plain WRL import). Contained, per-map.
	ProjectPack,
	/// Fold them into the user tileset mirroring the palette-owner pack
	/// (`resources/user/tilepacks/<owner>/`), deduped against existing tiles by
	/// exact pixels + pass, so they're reusable across maps.
	UserTileset,
}

/// One WRL tile that matched no pack tile, with how the map used it.
#[derive(Debug, Clone)]
pub struct UnmappedTile {
	/// Bin index into the WRL's tile table.
	pub index: u16,
	/// The tile's passability (0 land / 1 water / 2 shore / 3 blocked).
	pub pass: u8,
	/// How many map cells reference it.
	pub cells: usize,
	/// A synthetic display id (`{prefix}{class}{n}`), for the unmapped list.
	pub id: String,
}

/// A prepared WRL import: the target project with every matched cell already
/// placed, plus the list of tiles that found no home. Built by [`Self::new`];
/// committed by [`Self::finish`].
pub struct WrlImport {
	name: String,
	wrl: WrlFile,
	/// The palette-owner pack name - the user tileset extras attach to.
	owner: String,
	project: Project,
	/// Per WRL tile index → matched `(pack, tile, transform)`; `None` for unused
	/// or unmatched tiles. `finish` fills in the extras it places (identity).
	matched: Vec<Option<(u8, u16, Transform)>>,
	/// Cell-use count per WRL tile index (0 = unused).
	used: Vec<usize>,
	unmapped: Vec<UnmappedTile>,
}

/// Canonicalise a 64×64 tile for matching: fold the animated coastal-water band
/// and the tile's own mask index (when it has one) onto [`WILD`]; leave all
/// other pixels untouched.
fn canon(pixels: &[u8], mask: Option<u8>) -> [u8; TILE_DATA_SIZE] {
	let mut key = [0u8; TILE_DATA_SIZE];
	for (dst, &p) in key.iter_mut().zip(pixels) {
		*dst = if COASTAL_WATER.contains(&p) || Some(p) == mask { WILD } else { p };
	}
	key
}

/// The pixels of WRL tile `t` (panics only on a malformed WRL, which the
/// parser already rejects).
fn wrl_tile(wrl: &WrlFile, t: usize) -> &[u8] {
	&wrl.tiles[t * TILE_DATA_SIZE..(t + 1) * TILE_DATA_SIZE]
}

impl WrlImport {
	/// Build the target project from the chosen packs and match every used WRL
	/// tile against them. `owner` is the palette-owner pack (the user-tileset
	/// extras target); `pack_names` are the selected packs (WATER is implied).
	pub fn new(
		wrl: WrlFile,
		name: &str,
		owner: &str,
		pack_names: &[String],
		assets_root: &Path,
		seed: u64,
	) -> Result<Self, String> {
		let mut project = Project::new(wrl.width, wrl.height, pack_names, assets_root, seed)?;
		project.name = name.to_string();

		// Index every pack tile under all 8 orientations by canonical key (the
		// WRL stores each rotation/mirror as its own composited tile, while a
		// pack keeps one tile the project references with a transform). Identity
		// is inserted first, and first-writer-wins, so an untransformed match -
		// and the lowest-numbered tile - is always preferred.
		let mut by_key: HashMap<[u8; TILE_DATA_SIZE], (u8, u16, Transform)> = HashMap::new();
		for (pi, pack) in project.packs.iter().enumerate() {
			for t in 0..pack.tile_count() {
				let mask = pack.tile_mask(t);
				for tf in ORIENTATIONS {
					let key = canon(&transform_tile(pack.tile_pixels(t), tf), mask);
					by_key.entry(key).or_insert((pi as u8, t, tf));
				}
			}
		}

		let tile_count = wrl.tile_count as usize;
		let mut used = vec![0usize; tile_count];
		for &t in &wrl.bigmap {
			if (t as usize) < tile_count {
				used[t as usize] += 1;
			}
		}

		let mut matched: Vec<Option<(u8, u16, Transform)>> = vec![None; tile_count];
		for (t, count) in used.iter().enumerate() {
			if *count > 0 {
				matched[t] = by_key.get(&canon(wrl_tile(&wrl, t), None)).copied();
			}
		}

		// Place the matched cells: a water-pack tile rides the opaque base
		// layer (overwriting the random fill), everything else sits on the
		// ground layer with the water fill kept beneath (so a shore tile's
		// mask-0 pixels show water, not black).
		let water_pack = project.water_pack.unwrap_or(0);
		for (i, &t) in wrl.bigmap.iter().enumerate() {
			if let Some((pi, ti, tf)) = matched.get(t as usize).copied().flatten() {
				let layer = if pi == water_pack { LAYER_WATER } else { LAYER_GROUND };
				project.cells[i][layer] = Some(TileRef { pack: pi, tile: ti, transform: tf });
			}
		}

		let prefix = pack_prefix(name);
		let mut class_seq = [0u32; 4];
		let unmapped = (0..tile_count)
			.filter(|&t| used[t] > 0 && matched[t].is_none())
			.map(|t| {
				let pass = wrl.pass_table.get(t).copied().unwrap_or(0);
				let (letter, slot) = pass_class(pass);
				let n = class_seq[slot];
				class_seq[slot] += 1;
				UnmappedTile { index: t as u16, pass, cells: used[t], id: format!("{prefix}{letter}{n:03}") }
			})
			.collect();

		Ok(Self { name: name.to_string(), wrl, owner: owner.to_string(), project, matched, used, unmapped })
	}

	/// The WRL's cell dimensions.
	pub fn dims(&self) -> (u16, u16) {
		(self.wrl.width, self.wrl.height)
	}

	/// Distinct tiles the map actually uses.
	pub fn used_tiles(&self) -> usize {
		self.used.iter().filter(|&&c| c > 0).count()
	}

	/// How many used tiles found a match.
	pub fn matched_tiles(&self) -> usize {
		self.matched.iter().enumerate().filter(|(t, m)| self.used[*t] > 0 && m.is_some()).count()
	}

	/// The tiles that matched nothing (what the unmapped modal lists).
	pub fn unmapped(&self) -> &[UnmappedTile] {
		&self.unmapped
	}

	/// Commit the import: place the unmatched tiles per `dest`, then hand back
	/// the finished project. Returns the user-pack name to persist when extras
	/// were folded into the user tileset (`None` otherwise).
	pub fn finish(mut self, dest: ExtrasDest) -> (Project, Option<String>) {
		if self.unmapped.is_empty() {
			return (self.project, None);
		}
		match dest {
			ExtrasDest::Ignore => (self.project, None),
			ExtrasDest::ProjectPack => {
				let pack_name = self.extras_pack_name();
				let mut pack = TilePack::empty_user(&pack_name);
				// A project-bundled pack is synthetic, not a user pack: that's
				// what `write_project` dumps beside the `.json` so it reloads.
				pack.user = false;
				pack.version = "wrl".to_string();
				self.project.packs.push(pack);
				let pi = (self.project.packs.len() - 1) as u8;
				self.place_extras(pi, false);
				(self.project, None)
			}
			ExtrasDest::UserTileset => {
				let owner = self.owner.clone();
				let pi = match self.project.packs.iter().position(|p| p.user && p.name == owner) {
					Some(i) => i,
					None => {
						self.project.packs.push(TilePack::empty_user(&owner));
						self.project.packs.len() - 1
					}
				} as u8;
				self.place_extras(pi, true);
				(self.project, Some(owner))
			}
		}
	}

	/// Append the unmapped tiles to pack `pi` and point their cells at it. With
	/// `dedupe`, an unmapped tile whose exact pixels + pass already exist in the
	/// pack reuses that tile instead of adding a duplicate (also collapses
	/// byte-identical tiles within this import).
	fn place_extras(&mut self, pi: u8, dedupe: bool) {
		let prefix = pack_prefix(&self.name);
		// Seed the dedupe map from whatever the target pack already holds.
		let mut seen: HashMap<(Vec<u8>, u8), u16> = HashMap::new();
		if dedupe {
			let pack = &self.project.packs[pi as usize];
			for t in 0..pack.tile_count() {
				let pass = pack.pass.as_ref().and_then(|p| p.get(t as usize)).copied().unwrap_or(0);
				seen.insert((pack.tile_pixels(t).to_vec(), pass), t);
			}
		}
		// Snapshot the indices first - we mutate `self.project` in the loop.
		let extras: Vec<(usize, u8)> = self.unmapped.iter().map(|u| (u.index as usize, u.pass)).collect();
		for (t, pass) in extras {
			let pixels = wrl_tile(&self.wrl, t).to_vec();
			let local = if let Some(&i) = seen.get(&(pixels.clone(), pass)) {
				i
			} else {
				let id = unique_id(&self.project.packs[pi as usize], &prefix, pass);
				let i = self.project.packs[pi as usize].push_tile(id, &pixels, pass);
				seen.insert((pixels, pass), i);
				i
			};
			self.matched[t] = Some((pi, local, Transform::default()));
		}

		// Place every cell that used an extra tile. Extras reproduce the WRL's
		// composited pixels verbatim (no mask, identity transform), so they go on
		// their passability layer, opaque - covering the water fill where land/shore.
		for (i, &t) in self.wrl.bigmap.iter().enumerate() {
			let t = t as usize;
			if t >= self.used.len() || self.used[t] == 0 {
				continue;
			}
			let Some((mp, mt, tf)) = self.matched[t] else { continue };
			if mp != pi {
				continue; // already placed in `new`
			}
			let pass = self.wrl.pass_table.get(t).copied().unwrap_or(0);
			self.project.cells[i][pass_layer(pass)] = Some(TileRef { pack: mp, tile: mt, transform: tf });
		}
	}

	/// A filesystem-safe, project-unique name for the bundled extras pack.
	fn extras_pack_name(&self) -> String {
		let base: String =
			self.name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
		let base = if base.is_empty() { "import".to_string() } else { base };
		if self.project.packs.iter().any(|p| p.name == base) { format!("{base}_extra") } else { base }
	}
}

/// A `{prefix}{class}{NNN}` id not yet present in `pack`.
fn unique_id(pack: &TilePack, prefix: &str, pass: u8) -> String {
	let (letter, _) = pass_class(pass);
	(0..).map(|n| format!("{prefix}{letter}{n:03}")).find(|id| !pack.index_of.contains_key(id)).unwrap()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::project::{LAYER_GROUND, LAYER_WATER};

	fn assets_root() -> std::path::PathBuf {
		std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/assets/tilepacks")
	}

	const BOGUS: u8 = 200; // a pixel value no shipped tile is solid-filled with

	/// A 1×3 WRL: a WATER tile, a plain GREEN land tile, and a solid-`BOGUS`
	/// tile that matches nothing - built by copying real pack pixels.
	fn fixture() -> WrlFile {
		let root = assets_root();
		let water = TilePack::load(&root, "WATER").unwrap();
		let green = TilePack::load(&root, "GREEN").unwrap();
		// A GREEN tile with no mask and no coastal-water pixels matches exactly
		// (its canonical key equals the WRL copy's, with nothing wildcarded).
		let land = (0..green.tile_count())
			.find(|&t| {
				green.tile_mask(t).is_none() && !green.tile_pixels(t).iter().any(|&p| COASTAL_WATER.contains(&p))
			})
			.expect("a plain GREEN land tile");

		let mut tiles = Vec::new();
		tiles.extend_from_slice(water.tile_pixels(0));
		tiles.extend_from_slice(green.tile_pixels(land));
		tiles.extend_from_slice(&[BOGUS; TILE_DATA_SIZE]);
		WrlFile {
			header: vec![0; 5],
			width: 3,
			height: 1,
			minimap: vec![0; 3],
			bigmap: vec![0, 1, 2],
			tile_count: 3,
			tiles,
			palette: vec![0; 768],
			pass_table: vec![1, 0, 0],
		}
	}

	fn import() -> WrlImport {
		WrlImport::new(fixture(), "TestMap", "GREEN", &["GREEN".to_string()], &assets_root(), 0).unwrap()
	}

	#[test]
	fn matches_a_rotated_tile_via_a_transform() {
		let root = assets_root();
		let green = TilePack::load(&root, "GREEN").unwrap();
		let cw = Transform { rot: 1, mirror: false };
		// A plain land tile that actually changes under a quarter turn, so the
		// match must come back carrying a non-identity transform.
		let land = (0..green.tile_count())
			.find(|&t| {
				green.tile_mask(t).is_none()
					&& !green.tile_pixels(t).iter().any(|&p| COASTAL_WATER.contains(&p))
					&& transform_tile(green.tile_pixels(t), cw)[..] != *green.tile_pixels(t)
			})
			.expect("an asymmetric GREEN land tile");

		let rotated = transform_tile(green.tile_pixels(land), cw);
		let wrl = WrlFile {
			header: vec![0; 5],
			width: 1,
			height: 1,
			minimap: vec![0],
			bigmap: vec![0],
			tile_count: 1,
			tiles: rotated.to_vec(),
			palette: vec![0; 768],
			pass_table: vec![0],
		};
		let imp = WrlImport::new(wrl, "Rot", "GREEN", &["GREEN".to_string()], &root, 0).unwrap();
		assert_eq!(imp.unmapped().len(), 0, "a rotation matches the base tile, not unmapped");
		let cell = imp.project.cells[0][LAYER_GROUND].expect("placed on the ground layer");
		assert_ne!(cell.transform, Transform::default(), "matched via a non-identity transform");
	}

	#[test]
	fn matches_water_and_land_leaving_one_unmapped() {
		let imp = import();
		assert_eq!(imp.dims(), (3, 1));
		assert_eq!(imp.used_tiles(), 3);
		assert_eq!(imp.matched_tiles(), 2);
		assert_eq!(imp.unmapped().len(), 1);
		let u = &imp.unmapped()[0];
		assert_eq!((u.index, u.cells), (2, 1));
	}

	#[test]
	fn matched_cells_land_on_the_right_layer() {
		let imp = import();
		// Water rides the base layer; land sits on the ground layer over the fill.
		assert_eq!(imp.project.cells[0][LAYER_WATER].map(|r| r.pack), Some(0));
		assert!(imp.project.cells[1][LAYER_GROUND].is_some());
		assert_ne!(imp.project.cells[1][LAYER_GROUND].unwrap().pack, 0, "land is not the water pack");
		assert!(imp.project.cells[1][LAYER_WATER].is_some(), "water fill kept beneath land");
	}

	#[test]
	fn ignore_leaves_the_unmapped_cell_bare() {
		let (project, persist) = import().finish(ExtrasDest::Ignore);
		assert_eq!(persist, None);
		assert!(project.cells[2][LAYER_GROUND].is_none(), "no ground tile for the dropped one");
	}

	#[test]
	fn project_pack_bundles_extras_as_a_synthetic_pack() {
		let (project, persist) = import().finish(ExtrasDest::ProjectPack);
		assert_eq!(persist, None, "a project pack is not persisted to the user tileset");
		let ground = project.cells[2][LAYER_GROUND].expect("the extra tile placed");
		let pack = &project.packs[ground.pack as usize];
		assert!(!pack.user, "bundled extras ride a synthetic (non-user) pack");
		assert!(pack.tile_pixels(ground.tile).iter().all(|&p| p == BOGUS));
	}

	#[test]
	fn user_tileset_folds_extras_into_the_owner_pack() {
		let (project, persist) = import().finish(ExtrasDest::UserTileset);
		assert_eq!(persist.as_deref(), Some("GREEN"), "owner user pack is the persist target");
		let ground = project.cells[2][LAYER_GROUND].expect("the extra tile placed");
		let pack = &project.packs[ground.pack as usize];
		assert!(pack.user && pack.name == "GREEN");
		assert!(pack.tile_pixels(ground.tile).iter().all(|&p| p == BOGUS));
	}

	#[test]
	fn user_tileset_dedupes_identical_extras() {
		// Two cells using two byte-identical bogus tiles collapse to one tile.
		let mut wrl = fixture();
		wrl.tiles.extend_from_slice(&[BOGUS; TILE_DATA_SIZE]); // tile 3 == tile 2
		wrl.tile_count = 4;
		wrl.pass_table = vec![1, 0, 0, 0];
		wrl.bigmap = vec![2, 3, 0, 1];
		wrl.width = 4;
		let imp = WrlImport::new(wrl, "Dedupe", "GREEN", &["GREEN".to_string()], &assets_root(), 0).unwrap();
		assert_eq!(imp.unmapped().len(), 2, "both bogus tiles are unmatched");
		let (project, _) = imp.finish(ExtrasDest::UserTileset);
		let a = project.cells[0][LAYER_GROUND].unwrap();
		let b = project.cells[1][LAYER_GROUND].unwrap();
		assert_eq!((a.pack, a.tile), (b.pack, b.tile), "identical extras share one tile");
	}
}
