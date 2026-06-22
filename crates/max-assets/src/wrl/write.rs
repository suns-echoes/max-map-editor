//! WRL writer - exact inverse of `file::read_wrl_file` (map-editor addition;
//! the upstream re-MAX copy of this crate is read-only).

use std::fs;
use std::path::Path;

use super::consts::TILE_DATA_SIZE;
use super::file::WrlError;
use super::types::WrlFile;

/// Serialize to the on-disk WRL layout, validating field sizes first.
pub fn wrl_to_bytes(wrl: &WrlFile) -> Result<Vec<u8>, WrlError> {
	let cells = wrl.width as usize * wrl.height as usize;
	let tiles_len = wrl.tile_count as usize * TILE_DATA_SIZE;
	if wrl.header.len() != 5 {
		return Err(WrlError::Parse("header must be 5 bytes"));
	}
	if wrl.minimap.len() != cells {
		return Err(WrlError::Parse("minimap length != width * height"));
	}
	if wrl.bigmap.len() != cells {
		return Err(WrlError::Parse("bigmap length != width * height"));
	}
	if wrl.tiles.len() != tiles_len {
		return Err(WrlError::Parse("tiles length != tile_count * 64 * 64"));
	}
	if wrl.palette.len() != 256 * 3 {
		return Err(WrlError::Parse("palette must be 256 * 3 bytes"));
	}
	if wrl.pass_table.len() != wrl.tile_count as usize {
		return Err(WrlError::Parse("pass table length != tile_count"));
	}

	let mut out = Vec::with_capacity(5 + 4 + cells * 3 + 2 + tiles_len + 768 + wrl.pass_table.len());
	out.extend_from_slice(&wrl.header);
	out.extend_from_slice(&wrl.width.to_le_bytes());
	out.extend_from_slice(&wrl.height.to_le_bytes());
	out.extend_from_slice(&wrl.minimap);
	for &cell in &wrl.bigmap {
		out.extend_from_slice(&cell.to_le_bytes());
	}
	out.extend_from_slice(&wrl.tile_count.to_le_bytes());
	out.extend_from_slice(&wrl.tiles);
	out.extend_from_slice(&wrl.palette);
	out.extend_from_slice(&wrl.pass_table);
	Ok(out)
}

pub fn write_wrl_file(wrl: &WrlFile, path: &Path) -> Result<(), WrlError> {
	if let Some(parent) = path.parent() {
		fs::create_dir_all(parent)?;
	}
	fs::write(path, wrl_to_bytes(wrl)?)?;
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::super::consts::INSTALLED_MAP_FILE_NAMES;
	use super::super::file::read_wrl_file;
	use super::*;

	/// Byte-identical read→write round-trip over the stock maps (backlog
	/// TEST-5). Reads the vendored `resources/originals/` (override with the
	/// `MAX_DIR` env var); skips cleanly if that directory is absent.
	#[test]
	fn stock_maps_round_trip_byte_identical() {
		let dir = std::env::var("MAX_DIR").unwrap_or_else(|_| {
			Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/originals").to_string_lossy().into_owned()
		});
		let dir = Path::new(&dir);
		if !dir.is_dir() {
			eprintln!("skipping: no M.A.X. dir at {}", dir.display());
			return;
		}
		let mut checked = 0;
		for name in INSTALLED_MAP_FILE_NAMES {
			let path = dir.join(name);
			if !path.is_file() {
				continue;
			}
			let original = fs::read(&path).unwrap();
			let parsed = read_wrl_file(&path).unwrap();
			let written = wrl_to_bytes(&parsed).unwrap();
			assert!(original == written, "{name}: round-trip differs");
			checked += 1;
		}
		eprintln!("round-tripped {checked} maps byte-identical");
		assert!(checked > 0, "found a M.A.X. dir but no stock maps in it");
	}

	#[test]
	fn size_validation_rejects_inconsistent_files() {
		let wrl = WrlFile {
			header: vec![0; 5],
			width: 2,
			height: 2,
			minimap: vec![0; 4],
			bigmap: vec![0; 3], // wrong: must be 4
			tile_count: 1,
			tiles: vec![0; TILE_DATA_SIZE],
			palette: vec![0; 768],
			pass_table: vec![0; 1],
		};
		assert!(wrl_to_bytes(&wrl).is_err());
	}
}
