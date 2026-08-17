//! The tilepack-selection model shared by the New Map and Import WRL modals:
//! scan the installed packs (with their display metadata from `info.json` /
//! `palette.json`), track which are selected + which owns the palette + which
//! water pack fills the bottom layer, and produce the ordered name list
//! `Project::new` expects (the chosen water pack first, the palette owner
//! next, then the rest in scan order).

use std::path::{Path, PathBuf};

/// One installed tilepack as the picker sees it.
pub struct PackEntry {
	pub name: String,
	/// Display title: `info.json`'s `name` with a trailing "tile pack"
	/// trimmed ("Green tile pack" → "Green"); falls back to the dir name.
	pub title: String,
	pub selected: bool,
	pub has_palette: bool,
	/// The palette's display name: `palette.json`'s `name` with a trailing
	/// "palette" trimmed ("Green Palette" → "Green"); falls back to `title`.
	/// `None` on palette-less packs.
	pub palette_name: Option<String>,
	/// Water packs fill the bottom layer; exactly one is chosen at a time
	/// (WATER by default). Classified by the dir-name convention (`WATER…`).
	pub water: bool,
}

/// The `name` field of a small JSON metadata file (`None` when the file is
/// absent, unreadable, or carries none - lenient like the pack loader).
pub(crate) fn json_name(path: &Path) -> Option<String> {
	let text = std::fs::read_to_string(path).ok()?;
	let v = json::parse(&text).ok()?;
	v.get("name").and_then(|n| n.as_str().map(str::to_string))
}

/// Trims a trailing `suffix` word group (ASCII case-insensitive) plus the
/// space before it: `trim_suffix("Green tile pack", "tile pack")` → "Green".
pub(crate) fn trim_suffix(s: &str, suffix: &str) -> String {
	let t = s.trim();
	if t.len() > suffix.len() && t[t.len() - suffix.len()..].eq_ignore_ascii_case(suffix) {
		t[..t.len() - suffix.len()].trim_end().to_string()
	} else {
		t.to_string()
	}
}

/// Scan `assets_root` for installed packs (dirs with `tiles-data.bin`). The
/// WATER pack is the default water choice; GREEN is selected by default,
/// matching the original new-map UI.
pub fn scan(assets_root: &Path) -> Vec<PackEntry> {
	let mut packs = Vec::new();
	if let Ok(entries) = std::fs::read_dir(assets_root) {
		let mut names: Vec<PathBuf> =
			entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("tiles-data.bin").is_file()).collect();
		names.sort();
		for path in names {
			let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
			let water = name.to_ascii_uppercase().starts_with("WATER");
			let title = json_name(&path.join("info.json"))
				.map(|n| trim_suffix(&n, "tile pack"))
				.filter(|t| !t.is_empty())
				.unwrap_or_else(|| name.clone());
			let has_palette = path.join("palette.json").is_file();
			let palette_name = if has_palette {
				json_name(&path.join("palette.json"))
					.map(|n| trim_suffix(&n, "palette"))
					.filter(|t| !t.is_empty())
					.or_else(|| Some(title.clone()))
			} else {
				None
			};
			packs.push(PackEntry {
				selected: (water && name == "WATER") || name == "GREEN",
				has_palette,
				palette_name,
				water,
				title,
				name,
			});
		}
		// Ensure a default water choice even without a pack named WATER.
		if !packs.iter().any(|p| p.water && p.selected) {
			if let Some(p) = packs.iter_mut().find(|p| p.water) {
				p.selected = true;
			}
		}
	}
	packs
}

/// The pack that will own the palette: the radio choice when it's a selected,
/// palette-capable land pack, else the first such pack in scan order.
pub fn effective_owner(packs: &[PackEntry], chosen: &Option<String>) -> Option<String> {
	if let Some(name) = chosen {
		if packs.iter().any(|p| &p.name == name && p.selected && p.has_palette && !p.water) {
			return Some(name.clone());
		}
	}
	packs.iter().find(|p| p.selected && p.has_palette && !p.water).map(|p| p.name.clone())
}

/// The selected pack names in the order `Project::new` wants: the chosen
/// water pack (bottom layer) first, then the palette owner, then the rest in
/// scan order.
pub fn selected(packs: &[PackEntry], chosen: &Option<String>) -> Vec<String> {
	let mut out: Vec<String> = packs.iter().filter(|p| p.selected && p.water).map(|p| p.name.clone()).collect();
	let owner = effective_owner(packs, chosen);
	if let Some(o) = &owner {
		out.push(o.clone());
	}
	out.extend(
		packs.iter().filter(|p| p.selected && !p.water && Some(&p.name) != owner.as_ref()).map(|p| p.name.clone()),
	);
	out
}

/// Whether any selected pack can own the palette (the minimum to build a map).
pub fn has_palette_owner(packs: &[PackEntry]) -> bool {
	packs.iter().any(|p| p.selected && p.has_palette)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn entry(name: &str, selected: bool, has_palette: bool, water: bool) -> PackEntry {
		PackEntry {
			name: name.to_string(),
			title: name.to_string(),
			selected,
			has_palette,
			palette_name: has_palette.then(|| name.to_string()),
			water,
		}
	}

	#[test]
	fn trim_suffix_strips_the_metadata_boilerplate() {
		assert_eq!(trim_suffix("Green tile pack", "tile pack"), "Green");
		assert_eq!(trim_suffix("Green Palette", "palette"), "Green");
		assert_eq!(trim_suffix("Green", "tile pack"), "Green", "no suffix, unchanged");
		assert_eq!(trim_suffix("tile pack", "tile pack"), "tile pack", "never trims to empty");
	}

	#[test]
	fn selected_orders_water_then_owner_then_rest() {
		let packs = vec![
			entry("CRATER", true, true, false),
			entry("GREEN", true, true, false),
			entry("WATER", true, false, true),
		];
		// The explicit owner leads the land packs; water always leads them all.
		let order = selected(&packs, &Some("GREEN".to_string()));
		assert_eq!(order, ["WATER", "GREEN", "CRATER"]);
		// No choice: the first selected palette-capable land pack owns.
		let order = selected(&packs, &None);
		assert_eq!(order, ["WATER", "CRATER", "GREEN"]);
	}

	#[test]
	fn water_packs_never_own_the_palette() {
		// A hypothetical palette-carrying water pack still may not own.
		let packs = vec![entry("WATER_DEEP", true, true, true), entry("GREEN", true, true, false)];
		assert_eq!(effective_owner(&packs, &Some("WATER_DEEP".to_string())).as_deref(), Some("GREEN"));
	}

	#[test]
	fn scan_reads_titles_and_palette_names() {
		let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources/assets/tilepacks");
		let packs = scan(&root);
		assert!(!packs.is_empty(), "stock packs present");
		let green = packs.iter().find(|p| p.name == "GREEN").expect("GREEN installed");
		assert_eq!(green.title, "Green", "info.json name, ' tile pack' trimmed");
		assert_eq!(green.palette_name.as_deref(), Some("Green"), "palette.json name, ' Palette' trimmed");
		let water = packs.iter().find(|p| p.name == "WATER").expect("WATER installed");
		assert!(water.water && water.selected, "WATER is the default water choice");
		assert_eq!(water.title, "Water");
		assert!(water.palette_name.is_none(), "WATER carries no palette");
	}

	#[test]
	fn scan_defaults_a_water_choice_even_without_a_pack_named_water() {
		// A minimal install: two metadata-less packs, water only as "WATER2".
		let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp").join("packlist_waterfb");
		let _ = std::fs::remove_dir_all(&root);
		for name in ["ALPHA", "WATER2"] {
			std::fs::create_dir_all(root.join(name)).unwrap();
			std::fs::write(root.join(name).join("tiles-data.bin"), b"").unwrap();
		}
		let packs = scan(&root);
		assert_eq!(packs.len(), 2, "both packs found, in sorted order");
		let alpha = &packs[0];
		assert_eq!(alpha.title, "ALPHA", "no info.json: the dir name is the title");
		assert!(!alpha.selected && !alpha.water && !alpha.has_palette && alpha.palette_name.is_none());
		let water2 = &packs[1];
		assert!(water2.water, "WATER... dir names classify as water packs");
		assert!(water2.selected, "no pack named WATER: the first water pack becomes the default choice");
		// A missing root scans to nothing (no error).
		assert!(scan(&root.join("absent")).is_empty(), "an absent assets root yields no packs");
		let _ = std::fs::remove_dir_all(&root);
	}

	#[test]
	fn a_map_needs_a_selected_palette_owner() {
		let mut packs = vec![entry("GREEN", false, true, false), entry("WATER", true, false, true)];
		assert!(!has_palette_owner(&packs), "an unselected palette pack doesn't count; nor a palette-less pack");
		packs[0].selected = true;
		assert!(has_palette_owner(&packs), "selecting the palette pack satisfies the minimum to build a map");
	}
}
