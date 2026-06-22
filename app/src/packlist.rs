//! The tilepack-selection model shared by the New Map and Import WRL modals:
//! scan the installed packs, track which are selected + which owns the palette,
//! and produce the ordered name list `Project::new` expects (WATER first, the
//! palette owner next, then the rest in scan order).

use std::path::{Path, PathBuf};

/// One installed tilepack as the picker sees it.
pub struct PackEntry {
	pub name: String,
	pub selected: bool,
	pub has_palette: bool,
	/// WATER fills the bottom layer - always on (the `new` command implies it).
	pub locked: bool,
}

/// Scan `assets_root` for installed packs (dirs with `tiles-data.bin`). WATER
/// is locked-on; GREEN is selected by default, matching the original new-map UI.
pub fn scan(assets_root: &Path) -> Vec<PackEntry> {
	let mut packs = Vec::new();
	if let Ok(entries) = std::fs::read_dir(assets_root) {
		let mut names: Vec<PathBuf> =
			entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("tiles-data.bin").is_file()).collect();
		names.sort();
		for path in names {
			let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
			let locked = name == "WATER";
			packs.push(PackEntry {
				selected: locked || name == "GREEN",
				has_palette: path.join("palette.json").is_file(),
				locked,
				name,
			});
		}
	}
	packs
}

/// The pack that will own the palette: the radio choice when it's a selected,
/// palette-capable, non-WATER pack, else the first such pack in scan order.
pub fn effective_owner(packs: &[PackEntry], chosen: &Option<String>) -> Option<String> {
	if let Some(name) = chosen {
		if packs.iter().any(|p| &p.name == name && p.selected && p.has_palette && !p.locked) {
			return Some(name.clone());
		}
	}
	packs.iter().find(|p| p.selected && p.has_palette && !p.locked).map(|p| p.name.clone())
}

/// The selected pack names in the order `Project::new` wants: WATER (locked)
/// first, then the palette owner, then the rest in scan order.
pub fn selected(packs: &[PackEntry], chosen: &Option<String>) -> Vec<String> {
	let mut out: Vec<String> = packs.iter().filter(|p| p.selected && p.locked).map(|p| p.name.clone()).collect();
	let owner = effective_owner(packs, chosen);
	if let Some(o) = &owner {
		out.push(o.clone());
	}
	out.extend(
		packs.iter().filter(|p| p.selected && !p.locked && Some(&p.name) != owner.as_ref()).map(|p| p.name.clone()),
	);
	out
}

/// Whether any selected pack can own the palette (the minimum to build a map).
pub fn has_palette_owner(packs: &[PackEntry]) -> bool {
	packs.iter().any(|p| p.selected && p.has_palette)
}
