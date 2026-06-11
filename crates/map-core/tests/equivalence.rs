//! The 24-map equivalence proof:
//! every cell of every converted project in `resources/templates/`, composed via
//! `Project::compose_cell`, must be pixel-identical to the corresponding
//! tile of the original `.WRL`. This pins the transform convention, the
//! layer fall-through rule, and the pack data in one sweep.
//!
//! Reads the original WRLs from the gitignored `testdata/originals/` (they
//! are copyrighted game data and not in the repo — run
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

/// Palette indices the game color-cycles (water shimmer + effects) — see
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

#[test]
fn projects_compose_identical_to_original_wrls() {
	let max_dir = wrl_dir();
	if !max_dir.is_dir() {
		eprintln!("SKIPPED: original-map proof — no fixtures at {}", max_dir.display());
		eprintln!("         run tools/fetch-testdata.sh (or set MAX_DIR) to restore this coverage");
		return;
	}

	let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources");
	let assets = resources.join("assets");
	let maps = resources.join("templates");

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
			eprintln!("{stem}: no original WRL — skipped");
			continue;
		}

		let project =
			Project::load(&project_path, &assets).unwrap_or_else(|e| panic!("{stem}: project load failed: {e}"));
		let wrl = read_wrl_file(&wrl_path).unwrap();
		assert_eq!((project.width, project.height), (wrl.width, wrl.height), "{stem}: size");

		let mut bad_cells = 0;
		let mut phase_cells = 0;
		let mut first_bad = None;
		for y in 0..project.height {
			for x in 0..project.width {
				let composed = project.compose_cell(x, y);
				let tile_index = wrl.bigmap[y as usize * wrl.width as usize + x as usize] as usize;
				let original = &wrl.tiles[tile_index * 4096..(tile_index + 1) * 4096];
				if composed == *original {
					continue;
				}
				let acceptable = phase_equal(&composed, original);
				if acceptable {
					phase_cells += 1;
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
		if bad_cells > 0 {
			eprintln!("{stem}: {bad_cells}/{total} cells differ — {}", first_bad.unwrap());
			failed_maps.push(stem.clone());
		} else if phase_cells > 0 {
			eprintln!("{stem}: {total}/{total} ok ({phase_cells} animated-phase cells)");
		} else {
			eprintln!("{stem}: {total}/{total} cells identical");
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

/// Bake: projects bake to valid WRLs — dedup at least as tight as
/// the originals, byte round-trip through the writer/reader, and pass
/// values matching the original passtabs per cell (majority-rule pass data
/// deviates on ~0.1% of cells where Interplay assigned the same tile
/// different values across maps).
#[test]
fn projects_bake_to_valid_wrls() {
	let max_dir = wrl_dir();
	if !max_dir.is_dir() {
		eprintln!("SKIPPED: original-map proof — no fixtures at {}", max_dir.display());
		eprintln!("         run tools/fetch-testdata.sh (or set MAX_DIR) to restore this coverage");
		return;
	}
	let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources");
	let assets = resources.join("assets");

	let mut entries: Vec<_> = std::fs::read_dir(resources.join("templates"))
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
				// Equal modulo animated classes — the bake canonicalizes
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

/// `Project::from_wrl` (the document-model convergence: a `.WRL` opens as a
/// `Project`) is lossless — importing a WRL and composing every cell
/// reproduces the original tile **byte-for-byte**. No phase tolerance: the
/// import copies pixels verbatim, unlike the pack conversion above.
#[test]
fn from_wrl_imports_losslessly() {
	let max_dir = wrl_dir();
	if !max_dir.is_dir() {
		eprintln!("SKIPPED: original-map proof — no fixtures at {}", max_dir.display());
		eprintln!("         run tools/fetch-testdata.sh (or set MAX_DIR) to restore this coverage");
		return;
	}

	let maps = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/templates");
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
