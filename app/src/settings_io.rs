//! Settings-file IO: persist the machine-owned `[Workspace]` section into the
//! user override INI (`resources/user/config/mme.ini`, or `--settings PATH`),
//! re-reading first so any hand-edited sections in that file (key bindings,
//! `MaxPath`) survive. The shipped defaults (`resources/config/mme.ini`) are
//! never written - they're layered under this at load time.
//!
//! Pure (path + section in, `Result` out; no editor state), so the
//! `save-settings` handler stays thin and this can be tested against a temp dir.

use ini::{INI, INISection};
use std::path::Path;

/// Merge `workspace` into the `[Workspace]` section of the INI at `path`,
/// re-reading the file first so other (hand-edited) sections survive, and
/// creating the parent dir. The writer re-emits the whole file sorted, so
/// comments are not preserved (documented in MANUAL.md).
pub fn save_workspace(path: &Path, workspace: INISection) -> Result<(), String> {
	let mut ini = INI::from_file(path).unwrap_or_else(|_| INI::new());
	ini.insert_section("Workspace".to_string(), workspace);
	if let Some(parent) = path.parent() {
		if !parent.as_os_str().is_empty() {
			std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
		}
	}
	ini.to_file(path).map_err(|e| format!("cannot write {}: {e}", path.display()))
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
		save_workspace(&path, workspace_section()).unwrap();
		let back = INI::from_file(&path).unwrap();
		assert!(back.get_section("Workspace").is_some(), "[Workspace] written");
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

		save_workspace(&path, workspace_section()).unwrap();

		let back = INI::from_file(&path).unwrap();
		assert!(back.get_section("Keys").is_some(), "[Keys] survived the workspace save");
		assert!(back.get_section("Workspace").is_some(), "[Workspace] added");
		let _ = std::fs::remove_dir_all(&dir);
	}
}
