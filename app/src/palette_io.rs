//! Palette file IO for the palette manager (`user/palettes/*.json`): the
//! read / write / rename / delete that the `palette-*` commands drive.
//!
//! These are pure (paths and data in, `Result` out; no editor state or
//! console), so the command handlers stay thin and the IO can be tested
//! directly against a temp dir.

use std::path::{Path, PathBuf};

/// Write `palette` (768 RGB bytes) to `path` as a named palette JSON, creating
/// the parent directory first.
pub fn save(path: &Path, palette: &[u8], name: &str) -> Result<(), String> {
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
	}
	std::fs::write(path, map_core::write_palette(palette, name)).map_err(|e| format!("{}: {e}", path.display()))
}

/// Rename a saved palette file (`from` -> `to`).
pub fn rename(from: &Path, to: &Path) -> Result<(), String> {
	std::fs::rename(from, to).map_err(|e| format!("{}: {e}", to.display()))
}

/// Delete a saved palette file.
pub fn delete(path: &Path) -> Result<(), String> {
	std::fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Validate that `src` is a palette JSON and copy it into `dir` as `<stem>.json`
/// (creating `dir`). Returns the destination path.
pub fn import(src: &Path, dir: &Path) -> Result<PathBuf, String> {
	let text = std::fs::read_to_string(src).map_err(|e| format!("{}: {e}", src.display()))?;
	map_core::parse_palette(&text).map_err(|e| format!("not a palette ({e})"))?;
	std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
	let stem = src.file_stem().map_or_else(|| "imported".into(), |s| s.to_string_lossy().into_owned());
	let dest = dir.join(format!("{stem}.json"));
	std::fs::write(&dest, text).map_err(|e| format!("{}: {e}", dest.display()))?;
	Ok(dest)
}

/// Read + parse a palette file into its 768 RGB bytes.
pub fn load(path: &Path) -> Result<Vec<u8>, String> {
	let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
	map_core::parse_palette(&text)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A fresh empty scratch dir under the project `temp/`, unique per `tag` so
	/// tests don't collide.
	fn scratch(tag: &str) -> PathBuf {
		let d =
			PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp").join(format!("palette_io_{tag}"));
		let _ = std::fs::remove_dir_all(&d);
		std::fs::create_dir_all(&d).unwrap();
		d
	}

	/// A valid 768-byte palette (a per-channel ramp).
	fn ramp() -> Vec<u8> {
		(0..768).map(|i| (i % 256) as u8).collect()
	}

	#[test]
	fn save_then_load_round_trips() {
		let dir = scratch("roundtrip");
		let path = dir.join("p.json");
		save(&path, &ramp(), "my pal").unwrap();
		assert!(path.is_file());
		assert_eq!(load(&path).unwrap(), ramp());
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn save_creates_missing_parent_dirs() {
		let dir = scratch("mkparent");
		let path = dir.join("nested/deeper/p.json");
		save(&path, &ramp(), "x").unwrap();
		assert!(path.is_file(), "parent dirs created");
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn rename_then_delete() {
		let dir = scratch("renamedelete");
		let (a, b) = (dir.join("a.json"), dir.join("b.json"));
		save(&a, &ramp(), "a").unwrap();
		rename(&a, &b).unwrap();
		assert!(!a.is_file() && b.is_file(), "moved a -> b");
		delete(&b).unwrap();
		assert!(!b.is_file(), "deleted");
		assert!(delete(&b).is_err(), "deleting a missing file errors");
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn import_validates_and_copies_under_stem() {
		let dir = scratch("import");
		let user = dir.join("user");
		let src = dir.join("Cool Ramp.json");
		std::fs::write(&src, map_core::write_palette(&ramp(), "Cool Ramp")).unwrap();
		let dest = import(&src, &user).unwrap();
		assert_eq!(dest, user.join("Cool Ramp.json"), "copied to <stem>.json");
		assert_eq!(load(&dest).unwrap(), ramp());
		// A non-palette source is rejected.
		let bad = dir.join("notpal.json");
		std::fs::write(&bad, "{\"nope\": true}").unwrap();
		assert!(import(&bad, &user).is_err(), "non-palette rejected");
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn load_missing_file_errors() {
		assert!(load(Path::new("temp/__palette_io_absent__.json")).is_err());
	}
}
