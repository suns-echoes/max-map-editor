//! Auto-shore vs the original game maps (the auto-shore ground truth): the 24
//! converted projects in `resources/templates/` carry hand-crafted (game-true)
//! coastlines — a correct auto-shore must leave them alone. A
//! direction-mapping bug in the transform math lights up thousands of
//! cells here (the pass-based shore classification did exactly that).
//!
//! One known exception, a genuine boundary violation shipped in the
//! originals (the kind a shore-bug finder will list): CRATER_1 (17,98) `CSl000:S` —
//! a shore tile whose landward edge faces open water. (DESERT_4 (83,80),
//! flagged by the first cut of the law, is actually legal: its sea-only
//! edge presses against another band tile, the double-thick-band pattern
//! the originals use freely.)

use std::path::Path;

use map_core::Project;

#[test]
fn auto_shore_leaves_original_maps_alone() {
	let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources");
	let maps = resources.join("templates");
	let assets = resources.join("assets");

	let mut entries: Vec<_> = std::fs::read_dir(&maps)
		.expect("resources/templates")
		.filter_map(|e| e.ok())
		.map(|e| e.path())
		.filter(|p| p.extension().is_some_and(|x| x == "json"))
		.collect();
	entries.sort();
	assert_eq!(entries.len(), 24, "expected the 24 converted maps");

	let mut bad = Vec::new();
	for path in entries {
		let name = path.file_name().unwrap().to_string_lossy().to_string();
		let mut project = Project::load(&path, &assets).expect("load");
		let (changed, _) = project.auto_shore(None);
		let expected = match name.as_str() {
			"CRATER_1.json" => 1, // the known original bug at (17,98)
			_ => 0,
		};
		if changed != expected {
			bad.push(format!("{name}: {changed} cells (expected {expected})"));
		}
		// And the fix settles: a second pass finds nothing.
		let (again, _) = project.auto_shore(None);
		if again != 0 {
			bad.push(format!("{name}: not idempotent ({again} cells on second run)"));
		}
	}
	assert!(bad.is_empty(), "auto-shore disagrees with original maps:\n{}", bad.join("\n"));
}

/// The loop-walk variant (`shore alt`) must respect the same ground truth:
/// pristine coastlines have no targets and no fringe to promote, so the
/// walk leaves every original map alone (CRATER_1's shipped bug aside) and
/// settles on a second pass.
#[test]
fn auto_shore_alt_leaves_original_maps_alone() {
	let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources");
	let maps = resources.join("templates");
	let assets = resources.join("assets");

	let mut entries: Vec<_> = std::fs::read_dir(&maps)
		.expect("resources/templates")
		.filter_map(|e| e.ok())
		.map(|e| e.path())
		.filter(|p| p.extension().is_some_and(|x| x == "json"))
		.collect();
	entries.sort();

	let mut bad = Vec::new();
	for path in entries {
		let name = path.file_name().unwrap().to_string_lossy().to_string();
		let mut project = Project::load(&path, &assets).expect("load");
		let (changed, _) = project.auto_shore_alt(None);
		let expected = match name.as_str() {
			"CRATER_1.json" => 1, // the known original bug at (17,98)
			_ => 0,
		};
		if changed != expected {
			bad.push(format!("{name}: {changed} cells (expected {expected})"));
		}
		let (again, _) = project.auto_shore_alt(None);
		if again != 0 {
			bad.push(format!("{name}: not idempotent ({again} cells on second run)"));
		}
	}
	assert!(bad.is_empty(), "auto-shore alt disagrees with original maps:\n{}", bad.join("\n"));
}
