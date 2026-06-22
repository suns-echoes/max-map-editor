//! The 24-map equivalence proof:
//! every cell of every converted project in `resources/templates/`, composed via
//! `Project::compose_cell`, must be pixel-identical to the corresponding
//! tile of the original `.WRL`. This pins the transform convention, the
//! layer fall-through rule, and the pack data in one sweep.
//!
//! Two principled exceptions are tolerated: phase-free animated palette classes
//! (sea/effects sparkle, see [`animated_class`]) and tiles the shipped packs
//! have deliberately re-authored away from the 1996 art (see [`REAUTHORED`]).
//! Every other cell must match, so accidental regressions are still caught.
//!
//! Reads the original WRLs from the gitignored `testdata/originals/` (they
//! are copyrighted game data and not in the repo - run
//! `tools/fetch-testdata.sh` or set the `MAX_DIR` env var); skips **loudly**
//! if that directory is absent.

use std::path::{Path, PathBuf};

use map_core::Project;
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

#[test]
fn projects_compose_identical_to_original_wrls() {
	let max_dir = wrl_dir();
	if !max_dir.is_dir() {
		eprintln!("SKIPPED: original-map proof - no fixtures at {}", max_dir.display());
		eprintln!("         run tools/fetch-testdata.sh (or set MAX_DIR) to restore this coverage");
		return;
	}

	let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources");
	let assets = resources.join("assets/tilepacks");
	let maps = resources.join("assets/maps");

	let mut checked_maps = 0;
	let mut failed_maps = Vec::new();

	let mut entries: Vec<_> = std::fs::read_dir(&maps)
		.expect("read resources/templates")
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
				} else {
					bad_cells += 1;
					if first_bad.is_none() {
						let stack = project.cell(x, y).unwrap();
						first_bad = Some(format!("first mismatch at ({x},{y}), stack {stack:?}"));
					}
				}
			}
		}

		let total = project.width as u32 * project.height as u32;
		let extra = match (phase_cells, edited_cells) {
			(0, 0) => String::new(),
			(p, e) => format!(" ({p} animated-phase, {e} re-authored cells)"),
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
/// values matching the original passtabs per cell (majority-rule pass data
/// deviates on ~0.1% of cells where Interplay assigned the same tile
/// different values across maps).
#[test]
fn projects_bake_to_valid_wrls() {
	let max_dir = wrl_dir();
	if !max_dir.is_dir() {
		eprintln!("SKIPPED: original-map proof - no fixtures at {}", max_dir.display());
		eprintln!("         run tools/fetch-testdata.sh (or set MAX_DIR) to restore this coverage");
		return;
	}
	let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources");
	let assets = resources.join("assets/tilepacks");

	let mut entries: Vec<_> = std::fs::read_dir(resources.join("assets/maps"))
		.expect("read resources/templates")
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
	assert!(total_pass_deviation <= 300, "pass deviation {total_pass_deviation} exceeds the known majority-rule bound",);
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
		eprintln!("SKIPPED: original-map proof - no fixtures at {}", max_dir.display());
		eprintln!("         run tools/fetch-testdata.sh (or set MAX_DIR) to restore this coverage");
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
		eprintln!("SKIPPED: original-map proof - no fixtures at {}", max_dir.display());
		eprintln!("         run tools/fetch-testdata.sh (or set MAX_DIR) to restore this coverage");
		return;
	}

	let maps = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/assets/maps");
	let mut entries: Vec<_> = std::fs::read_dir(&maps)
		.expect("read resources/templates")
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
