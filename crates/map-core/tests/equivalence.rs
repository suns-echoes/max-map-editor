//! The 24-map equivalence proof:
//! every cell of every converted project in `resources/assets/maps/`, composed via
//! `Project::compose_cell`, must be pixel-identical to the corresponding
//! tile of the original `.WRL`. This pins the transform convention, the
//! layer fall-through rule, and the pack data in one sweep.
//!
//! Four principled exceptions are tolerated: phase-free animated palette
//! classes (sea/effects sparkle, see [`animated_class`]), tiles the shipped
//! packs have deliberately re-authored away from the 1996 art (see
//! [`REAUTHORED`]), map cells the shipped projects deliberately fix against the
//! original placement (see [`FIXED_CELLS`]), and LAND / OBSTRUCTION cells the
//! data fix de-animated - a pixel the packs moved off a cycled slot onto a
//! static look-alike where the WRL still holds the animated index (see
//! [`deanimated_equal`]). Every other cell must match, so accidental
//! regressions are still caught.
//!
//! Reads the original WRLs from the gitignored `testdata/originals/` (they
//! are copyrighted game data and not in the repo - run
//! `tools/fetch-testdata.sh` or set the `MAX_DIR` env var); skips **loudly**
//! if that directory is absent.

use std::path::{Path, PathBuf};

use map_core::{Project, TileKind, family_of};
use max_assets::wrl::read_wrl_file;

/// The directory of original `.WRL` maps: `MAX_DIR` if set, else the local
/// fixture dir `testdata/originals/`.
fn wrl_dir() -> PathBuf {
	std::env::var("MAX_DIR")
		.map(PathBuf::from)
		.unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/originals"))
}

/// Palette indices the game color-cycles (water shimmer + effects) - see
/// `docs/design/tileset-contract.md` §1. Pixels here are *phase-free*: the
/// conversion to packs canonicalized interchangeable sparkle/sea-phase
/// variants, so composed output may legally differ from the original in
/// these indices (and only these).
/// Stand down a fixture-gated proof - and refuse to, under `MAX_REQUIRE_FIXTURES=1`.
///
/// The original maps are copyrighted and not in the repo, so without them these
/// tests print a line and pass, which makes a run that proved nothing look
/// exactly like the 24-map proof. On a machine that has the fixtures, set
/// `MAX_REQUIRE_FIXTURES=1` and a skip becomes a failure instead.
#[track_caller]
fn skip_without_fixtures(dir: &Path) {
	assert!(
		std::env::var_os("MAX_REQUIRE_FIXTURES").is_none_or(|v| v != "1"),
		"MAX_REQUIRE_FIXTURES=1, but the original-map proof skipped: no fixtures at {}",
		dir.display()
	);
	eprintln!("SKIPPED: original-map proof - no fixtures at {}", dir.display());
	eprintln!("         run tools/fetch-testdata.sh (or set MAX_DIR) to restore this coverage");
}

fn animated_class(index: u8) -> bool {
	(9..=31).contains(&index) || (96..=127).contains(&index)
}

/// Pixel equality modulo the phase-free animated classes.
fn phase_equal(a: &[u8], b: &[u8]) -> bool {
	a.iter().zip(b).all(|(&c, &o)| c == o || (animated_class(c) && animated_class(o)))
}

/// Tiles the shipped packs **intentionally re-author** away from the original
/// 1996 art (hand-edited in the Tile Painter, then Baked into `resources/assets`).
/// A cell whose top tile is one of these legally differs from the original WRL;
/// every other cell must still match, so accidental regressions in the transform
/// convention, the fall-through rule, or any *unedited* tile are still caught.
/// Keyed by `(pack name, tile id)`.
const REAUTHORED: &[(&str, &str)] = &[
	("CRATER", "CMa060"),
	("CRATER", "CMa064"),
	("CRATER", "CMa094"),
	("CRATER", "CMa095"),
	("CRATER", "CMa096"),
	("CRATER", "CMa097"),
	("GREEN", "GMa151"),
	("GREEN", "GMa152"),
	("GREEN", "GMa167"),
	("GREEN", "GMa168"),
];

/// Does cell `(x, y)`'s stack rest on a deliberately re-authored tile?
fn cell_reauthored(project: &Project, x: u16, y: u16) -> bool {
	let Some(stack) = project.cell(x, y) else { return false };
	stack.iter().rev().flatten().any(|t| {
		let pack = &project.packs[t.pack as usize];
		REAUTHORED.contains(&(pack.name.as_str(), pack.ids[t.tile as usize].as_str()))
	})
}

/// Map cells the shipped projects **intentionally fix** against the original
/// 1996 placement (a wrong tile in the original map, corrected in the
/// converted project - user-confirmed 2026-07-03). Keyed by (map stem, cell);
/// every other cell of that map must still match the original WRL.
const FIXED_CELLS: &[(&str, (u16, u16))] = &[("SNOW_5", (31, 28)), ("SNOW_5", (54, 65))];

/// Is the cell's topmost placed tile a LAND or OBSTRUCTION tile - one of the
/// classes the data fix de-animates? Shore / water tiles keep their cycled
/// pixels, so a difference there is never a de-animation.
fn cell_top_deanimatable(project: &Project, x: u16, y: u16) -> bool {
	let Some(stack) = project.cell(x, y) else { return false };
	stack.iter().rev().flatten().next().is_some_and(|t| {
		let pack = &project.packs[t.pack as usize];
		let id = &pack.ids[t.tile as usize];
		matches!(pack.props.get(family_of(id)).and_then(|p| p.kind), Some(TileKind::Land | TileKind::Obstruction))
	})
}

/// Cell equality modulo the de-animation the shipped packs applied to LAND /
/// OBSTRUCTION tiles: a pixel may legally differ only when the original WRL
/// holds an animated index and the composed pack pixel now holds a *non*-animated
/// one (the static look-alike it was remapped to). Any other difference - a
/// non-animated pixel changing, or a pixel becoming animated - is a real bug.
fn deanimated_equal(composed: &[u8], original: &[u8]) -> bool {
	composed.iter().zip(original).all(|(&c, &o)| c == o || (animated_class(o) && !animated_class(c)))
}

#[test]
fn projects_compose_identical_to_original_wrls() {
	let max_dir = wrl_dir();
	if !max_dir.is_dir() {
		skip_without_fixtures(&max_dir);
		return;
	}

	let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources");
	let assets = resources.join("assets/tilepacks");
	let maps = resources.join("assets/maps");

	let mut checked_maps = 0;
	let mut failed_maps = Vec::new();

	let mut entries: Vec<_> = std::fs::read_dir(&maps)
		.expect("read resources/assets/maps")
		.filter_map(|e| e.ok())
		.map(|e| e.path())
		.filter(|p| p.extension().is_some_and(|x| x == "json"))
		.collect();
	entries.sort();

	for project_path in entries {
		let stem = project_path.file_stem().unwrap().to_string_lossy().to_string();
		let wrl_path = max_dir.join(format!("{stem}.WRL"));
		if !wrl_path.is_file() {
			eprintln!("{stem}: no original WRL - skipped");
			continue;
		}

		let project =
			Project::load(&project_path, &assets).unwrap_or_else(|e| panic!("{stem}: project load failed: {e}"));
		let wrl = read_wrl_file(&wrl_path).unwrap();
		assert_eq!((project.width, project.height), (wrl.width, wrl.height), "{stem}: size");

		let mut bad_cells = 0;
		let mut phase_cells = 0;
		let mut edited_cells = 0;
		let mut fixed_cells = 0;
		let mut deanimated_cells = 0;
		let mut first_bad = None;
		for y in 0..project.height {
			for x in 0..project.width {
				let composed = project.compose_cell(x, y);
				let tile_index = wrl.bigmap[y as usize * wrl.width as usize + x as usize] as usize;
				let original = &wrl.tiles[tile_index * 4096..(tile_index + 1) * 4096];
				if composed == *original {
					continue;
				}
				if phase_equal(&composed, original) {
					phase_cells += 1;
				} else if cell_reauthored(&project, x, y) {
					edited_cells += 1; // a deliberately re-authored tile (Baked over the original)
				} else if FIXED_CELLS.contains(&(stem.as_str(), (x, y))) {
					fixed_cells += 1; // a deliberately fixed map cell
				} else if cell_top_deanimatable(&project, x, y) && deanimated_equal(&composed, original) {
					deanimated_cells += 1; // a de-animated land/obstruction tile
				} else {
					bad_cells += 1;
					// List the first few offenders - enough to key an
					// exception or spot a pattern without drowning the log.
					if bad_cells <= 8 {
						let stack = project.cell(x, y).unwrap();
						let line = format!("({x},{y}) stack {stack:?}");
						first_bad = Some(match first_bad {
							None => format!("mismatches: {line}"),
							Some(prev) => format!("{prev}; {line}"),
						});
					}
				}
			}
		}

		let total = project.width as u32 * project.height as u32;
		let extra = match (phase_cells, edited_cells, fixed_cells, deanimated_cells) {
			(0, 0, 0, 0) => String::new(),
			(p, e, f, d) => format!(" ({p} animated-phase, {e} re-authored, {f} fixed, {d} de-animated cells)"),
		};
		if bad_cells > 0 {
			eprintln!("{stem}: {bad_cells}/{total} cells differ - {}", first_bad.unwrap());
			failed_maps.push(stem.clone());
		} else {
			eprintln!("{stem}: {total}/{total} ok{extra}");
		}
		checked_maps += 1;

		// Save round-trip while we're here: load(save(p)) must equal p.
		let saved = project.save_string();
		let reloaded = Project::from_str(&saved, &assets)
			.unwrap_or_else(|e| panic!("{stem}: reload of saved project failed: {e}"));
		assert_eq!(project.hash(), reloaded.hash(), "{stem}: save round-trip hash");
	}

	assert!(checked_maps > 0, "found a M.A.X. dir but checked nothing");
	assert!(failed_maps.is_empty(), "{} of {checked_maps} maps mismatch: {failed_maps:?}", failed_maps.len(),);
}

/// Bake: projects bake to valid WRLs - dedup at least as tight as
/// the originals, byte round-trip through the writer/reader, and pass
/// values matching the original passtabs per cell. Two sources of legal
/// deviation: majority-rule pass data (Interplay assigned the same tile
/// different values across maps, ~0.1% of cells) and the shipped packs'
/// deliberate pass fixes (wrong original values corrected in tiles.pass -
/// the "map updates" data revisions, user-confirmed 2026-07-03).
#[test]
fn projects_bake_to_valid_wrls() {
	let max_dir = wrl_dir();
	if !max_dir.is_dir() {
		skip_without_fixtures(&max_dir);
		return;
	}
	let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources");
	let assets = resources.join("assets/tilepacks");

	let mut entries: Vec<_> = std::fs::read_dir(resources.join("assets/maps"))
		.expect("read resources/assets/maps")
		.filter_map(|e| e.ok())
		.map(|e| e.path())
		.filter(|p| p.extension().is_some_and(|x| x == "json"))
		.collect();
	entries.sort();

	let mut checked = 0;
	let mut total_pass_deviation = 0u32;
	for project_path in entries {
		let stem = project_path.file_stem().unwrap().to_string_lossy().to_string();
		let wrl_path = max_dir.join(format!("{stem}.WRL"));
		if !wrl_path.is_file() {
			continue;
		}
		let project = Project::load(&project_path, &assets).unwrap();
		let original = read_wrl_file(&wrl_path).unwrap();

		let baked = map_core::bake(&project).unwrap_or_else(|e| panic!("{stem}: {e}"));
		assert!(
			baked.tile_count <= original.tile_count,
			"{stem}: baked {} tiles > original {}",
			baked.tile_count,
			original.tile_count,
		);

		// Byte round-trip through the writer.
		let bytes = max_assets::wrl::wrl_to_bytes(&baked).unwrap();
		let reread = {
			let tmp = std::env::temp_dir().join(format!("bake-{stem}.wrl"));
			std::fs::write(&tmp, &bytes).unwrap();
			let r = read_wrl_file(&tmp).unwrap();
			let _ = std::fs::remove_file(&tmp);
			r
		};
		assert_eq!(max_assets::wrl::wrl_to_bytes(&reread).unwrap(), bytes, "{stem}: round-trip");

		// Baked cells reproduce the composition; pass per cell vs original.
		let mut pass_deviation = 0u32;
		for y in 0..project.height {
			for x in 0..project.width {
				let i = y as usize * project.width as usize + x as usize;
				let bi = baked.bigmap[i] as usize;
				let baked_tile = &baked.tiles[bi * 4096..(bi + 1) * 4096];
				// Equal modulo animated classes - the bake canonicalizes
				// the sea phase under ground cut-outs.
				assert!(phase_equal(baked_tile, &project.compose_cell(x, y)), "{stem}: cell ({x},{y}) bake != compose",);
				let oi = original.bigmap[i] as usize;
				if baked.pass_table[bi] != original.pass_table[oi] {
					pass_deviation += 1;
				}
			}
		}
		total_pass_deviation += pass_deviation;
		eprintln!(
			"{stem}: baked {} tiles (original {}), pass deviation {pass_deviation}",
			baked.tile_count, original.tile_count,
		);
		checked += 1;
	}
	assert!(checked > 0, "found a M.A.X. dir but baked nothing");
	eprintln!("total pass deviation: {total_pass_deviation} cells");
	// 1141 observed across the 24 maps with the current (fixed) pass data;
	// headroom for majority-rule wobble, tight enough to catch a pass-table
	// regression (which shifts this by thousands).
	assert!(total_pass_deviation <= 1200, "pass deviation {total_pass_deviation} exceeds the known bound",);
}

/// `WrlImport` (Import WRL onto existing tilesets) matches nearly every tile of
/// an original standard-tile map back to its native pack: the packs were
/// derived from these very WRLs, so coastal-water (96..=116) + shore-mask
/// wildcarding should re-find almost all of them. A high match rate proves the
/// matcher copes with the WRL's composited shore/water vs the pack's masked
/// overlays - the whole point of the feature. (A handful of re-authored or
/// effect-phase tiles legitimately won't match, hence a floor, not 100%.)
#[test]
fn import_matches_real_wrls_to_their_pack() {
	let max_dir = wrl_dir();
	if !max_dir.is_dir() {
		skip_without_fixtures(&max_dir);
		return;
	}
	let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/assets/tilepacks");

	let mut entries: Vec<_> = std::fs::read_dir(&max_dir)
		.expect("read testdata/originals")
		.filter_map(|e| e.ok())
		.map(|e| e.path())
		.filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("wrl")))
		.collect();
	entries.sort();

	let mut checked = 0;
	let mut worst = 1.0_f32;
	for wrl_path in entries {
		let stem = wrl_path.file_stem().unwrap().to_string_lossy().to_string();
		// The map's native land pack is its name prefix (GREEN_1 → GREEN); snow
		// maps split between the SNOW and SNOW_DARK packs, so offer both.
		let prefix = stem.split('_').next().unwrap_or(&stem).to_string();
		let packs: Vec<String> =
			if prefix == "SNOW" { vec!["SNOW".into(), "SNOW_DARK".into()] } else { vec![prefix.clone()] };
		if !assets.join(&packs[0]).is_dir() {
			continue;
		}
		let wrl = read_wrl_file(&wrl_path).unwrap();
		let import = map_core::WrlImport::new(wrl, &stem, &packs[0], &packs, &assets, 0)
			.unwrap_or_else(|e| panic!("{stem}: {e}"));
		let (used, matched) = (import.used_tiles(), import.matched_tiles());
		let rate = matched as f32 / used.max(1) as f32;
		eprintln!("{stem}: {matched}/{used} tiles matched against {} ({:.0}%)", packs.join("+"), rate * 100.0);
		worst = worst.min(rate);
		checked += 1;
	}
	assert!(checked > 0, "found a M.A.X. dir but matched nothing");
	// Transform-aware matching re-finds ~97-100% of each map's tiles in its pack
	// (only deliberately re-authored tiles miss); a regression below this floor
	// means the matcher stopped reusing existing tiles.
	assert!(worst >= 0.95, "worst match rate {:.0}% below the 95% floor", worst * 100.0);
}

/// `Project::from_wrl` (the document-model convergence: a `.WRL` opens as a
/// `Project`) is lossless - importing a WRL and composing every cell
/// reproduces the original tile **byte-for-byte**. No phase tolerance: the
/// import copies pixels verbatim, unlike the pack conversion above.
#[test]
fn from_wrl_imports_losslessly() {
	let max_dir = wrl_dir();
	if !max_dir.is_dir() {
		skip_without_fixtures(&max_dir);
		return;
	}

	let maps = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/assets/maps");
	let mut entries: Vec<_> = std::fs::read_dir(&maps)
		.expect("read resources/assets/maps")
		.filter_map(|e| e.ok())
		.map(|e| e.path())
		.filter(|p| p.extension().is_some_and(|x| x == "json"))
		.collect();
	entries.sort();

	let mut checked = 0;
	for project_path in entries {
		let stem = project_path.file_stem().unwrap().to_string_lossy().to_string();
		let wrl_path = max_dir.join(format!("{stem}.WRL"));
		if !wrl_path.is_file() {
			continue;
		}
		let wrl = read_wrl_file(&wrl_path).unwrap();
		let project = Project::from_wrl(&wrl, &stem);
		assert_eq!((project.width, project.height), (wrl.width, wrl.height), "{stem}: size");

		for y in 0..project.height {
			for x in 0..project.width {
				let composed = project.compose_cell(x, y);
				let i = wrl.bigmap[y as usize * wrl.width as usize + x as usize] as usize;
				let original = &wrl.tiles[i * 4096..(i + 1) * 4096];
				assert!(composed[..] == original[..], "{stem}: cell ({x},{y}) differs from its source tile");
			}
		}
		checked += 1;
	}
	eprintln!("from_wrl lossless on {checked} map(s)");
	assert!(checked > 0, "found a M.A.X. dir but imported nothing");
}
