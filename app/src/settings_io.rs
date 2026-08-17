//! Settings-file IO: persist the machine-owned `[Workspace*]` layout sections
//! (one per mode's dock layout) into the
//! user override INI (`resources/user/config/mme.ini`, or `--settings PATH`),
//! re-reading first so any hand-edited sections in that file (key bindings,
//! `MaxPath`) survive. The shipped defaults (`resources/config/mme.ini`) are
//! never written - they're layered under this at load time.
//!
//! Pure (path + section in, `Result` out; no editor state), so the
//! `save-settings` handler stays thin and this can be tested against a temp dir.

use ini::{INI, INISection};
use std::path::{Path, PathBuf};

/// Merge several named sections into the INI at `path` in one read-modify-write:
/// re-read the file first so other (hand-edited) sections survive, replace each
/// named section, create the parent dir, and write. The writer re-emits the
/// whole file sorted, so comments are not preserved (documented in MANUAL.md).
pub fn save_sections(path: &Path, sections: Vec<(&str, INISection)>) -> Result<(), String> {
	let mut ini = INI::from_file(path).unwrap_or_else(|_| INI::new());
	for (name, section) in sections {
		ini.insert_section(name.to_string(), section);
	}
	if let Some(parent) = path.parent() {
		if !parent.as_os_str().is_empty() {
			std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
		}
	}
	ini.to_file(path).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Merge a single `section` into the `[name]` section of the INI at `path` (see
/// [`save_sections`]).
pub fn save_section(path: &Path, name: &str, section: INISection) -> Result<(), String> {
	save_sections(path, vec![(name, section)])
}

/// Persist the `[Preferences]` section: small user options (the New Map
/// palette-preview toggle). Written immediately on change, like QuickLoad.
pub fn save_preferences(path: &Path, prefs: INISection) -> Result<(), String> {
	save_section(path, "Preferences", prefs)
}

/// Persist the recent-maps list as the `[QuickLoad]` section: keys `0..n`,
/// most-recent first (the File ▸ Quick Load order). Written immediately as
/// maps open, so the history survives even an unclean exit.
pub fn save_quickload(path: &Path, recent: &[PathBuf]) -> Result<(), String> {
	let mut section = INISection::new();
	for (i, p) in recent.iter().enumerate() {
		let _ = section.set_entry(i.to_string(), p.display().to_string());
	}
	save_section(path, "QuickLoad", section)
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::PathBuf;

	fn scratch(tag: &str) -> PathBuf {
		let d =
			PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp").join(format!("settings_io_{tag}"));
		let _ = std::fs::remove_dir_all(&d);
		std::fs::create_dir_all(&d).unwrap();
		d
	}

	fn workspace_section() -> INISection {
		let mut s = INISection::new();
		let _ = s.set_entry("dock_left".to_string(), "tiles".to_string());
		s
	}

	#[test]
	fn writes_workspace_and_creates_parent_dirs() {
		let dir = scratch("write");
		let path = dir.join("nested/mme.ini");
		save_section(&path, "Workspace", workspace_section()).unwrap();
		let back = INI::from_file(&path).unwrap();
		assert!(back.get_section("Workspace").is_some(), "[Workspace] written");
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn quickload_writes_integer_keys_and_keeps_other_sections() {
		let dir = scratch("quickload");
		let path = dir.join("mme.ini");
		// A pre-existing [Workspace] section must survive the QuickLoad write.
		let mut existing = INI::new();
		let mut ws = INISection::new();
		let _ = ws.set_entry("Docks".to_string(), "100 100 100 100".to_string());
		existing.insert_section("Workspace".to_string(), ws);
		existing.to_file(&path).unwrap();

		let recent = vec![PathBuf::from("/maps/a.json"), PathBuf::from("/maps/b.json")];
		save_quickload(&path, &recent).unwrap();

		let back = INI::from_file(&path).unwrap();
		let qs = back.get_section("QuickLoad").expect("[QuickLoad] written");
		assert_eq!(qs.get_entry::<String>("0").as_deref(), Some("/maps/a.json"), "key 0 = most recent");
		assert_eq!(qs.get_entry::<String>("1").as_deref(), Some("/maps/b.json"));
		assert!(back.get_section("Workspace").is_some(), "[Workspace] survived the QuickLoad write");
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn preferences_write_their_own_section() {
		let dir = scratch("prefs");
		let path = dir.join("mme.ini");
		let mut prefs = INISection::new();
		let _ = prefs.set_entry("PalettePreview".to_string(), "1".to_string());
		save_preferences(&path, prefs).unwrap();
		let back = INI::from_file(&path).unwrap();
		let ps = back.get_section("Preferences").expect("[Preferences] written");
		// The parser types values on re-read, so the written "1" comes back numeric.
		assert_eq!(ps.get_entry::<i64>("PalettePreview"), Some(1), "the option round-trips");
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn bare_and_parentless_paths_need_no_directory_creation() {
		// A bare file name (an empty parent) writes into the cwd with no mkdir.
		let bare = Path::new("__settings_io_bare_scratch__.ini");
		let _ = std::fs::remove_file(bare);
		save_section(bare, "Workspace", workspace_section()).unwrap();
		assert!(bare.is_file(), "written to the cwd without creating directories");
		std::fs::remove_file(bare).unwrap();
		// `/` has no parent at all; the write error surfaces (no panic, no mkdir).
		let err = save_section(Path::new("/"), "Workspace", workspace_section())
			.expect_err("cannot write the root as a file");
		assert!(err.starts_with("cannot write"), "the writer's error message: {err}");
	}

	#[test]
	fn save_sections_writes_every_section_and_preserves_others() {
		let dir = scratch("sections");
		let path = dir.join("mme.ini");
		// A hand-edited section the editor doesn't own must survive.
		let mut existing = INI::new();
		let mut keys = INISection::new();
		let _ = keys.set_entry("save".to_string(), "Ctrl+S".to_string());
		existing.insert_section("Keys".to_string(), keys);
		existing.to_file(&path).unwrap();

		// The per-mode layout sections are written together in one pass.
		let mut pass = INISection::new();
		let _ = pass.set_entry("Docks".to_string(), "1 2 3 4".to_string());
		let mut save = INISection::new();
		let _ = save.set_entry("Docks".to_string(), "5 6 7 8".to_string());
		save_sections(&path, vec![("Workspace.Pass", pass), ("Workspace.Save", save)]).unwrap();

		let back = INI::from_file(&path).unwrap();
		assert!(back.get_section("Workspace.Pass").is_some(), "[Workspace.Pass] written");
		assert!(back.get_section("Workspace.Save").is_some(), "[Workspace.Save] written");
		assert!(back.get_section("Keys").is_some(), "hand-edited [Keys] survived");
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn preserves_other_hand_edited_sections() {
		let dir = scratch("preserve");
		let path = dir.join("mme.ini");
		// A user's hand-edited file with a [Keys] section the editor doesn't own.
		let mut existing = INI::new();
		let mut keys = INISection::new();
		let _ = keys.set_entry("save".to_string(), "Ctrl+S".to_string());
		existing.insert_section("Keys".to_string(), keys);
		existing.to_file(&path).unwrap();

		save_section(&path, "Workspace", workspace_section()).unwrap();

		let back = INI::from_file(&path).unwrap();
		assert!(back.get_section("Keys").is_some(), "[Keys] survived the workspace save");
		assert!(back.get_section("Workspace").is_some(), "[Workspace] added");
		let _ = std::fs::remove_dir_all(&dir);
	}
}
