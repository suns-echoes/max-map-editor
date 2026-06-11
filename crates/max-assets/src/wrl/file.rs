use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use super::consts::TILE_DATA_SIZE;
use super::types::{WrlFile, WrlHeader};

#[derive(Debug, thiserror::Error)]
pub enum WrlError {
	#[error("I/O error: {0}")]
	Io(#[from] io::Error),
	#[error("{0}")]
	Parse(&'static str),
}

fn read_u16_le<R: Read>(r: &mut R) -> io::Result<u16> {
	let mut buf = [0u8; 2];
	r.read_exact(&mut buf)?;
	Ok(u16::from_le_bytes(buf))
}

fn read_u16_vec_le<R: Read>(r: &mut R, count: usize) -> io::Result<Vec<u16>> {
	let mut bytes = vec![0u8; count * 2];
	r.read_exact(&mut bytes)?;
	Ok(bytes.chunks_exact(2).map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]])).collect())
}

/// Largest map this loader will allocate for. The 24 original maps are
/// ≤112×112; the editor caps new/imported maps at 1024×1024. Without this
/// bound a crafted 13-byte header (`width = height = 65535`) forces a
/// multi-gigabyte allocation *before* any body is read → OOM / abort.
pub const MAX_WRL_DIM: u16 = 1024;

pub fn read_wrl_file(file_path: &Path) -> Result<WrlFile, WrlError> {
	let mut file = File::open(file_path)?;

	let mut header = vec![0; 5];
	file.read_exact(&mut header)?;

	let width = read_u16_le(&mut file)?;
	let height = read_u16_le(&mut file)?;
	if width == 0 || height == 0 || width > MAX_WRL_DIM || height > MAX_WRL_DIM {
		return Err(WrlError::Parse("map dimensions out of range (1..=1024)"));
	}

	let minimap_size = width as usize * height as usize;
	let mut minimap = vec![0; minimap_size];
	file.read_exact(&mut minimap)?;

	let bigmap_size = width as usize * height as usize;
	let bigmap = read_u16_vec_le(&mut file, bigmap_size)?;

	let tile_count = read_u16_le(&mut file)?;

	let tiles_size = tile_count as usize * TILE_DATA_SIZE;
	let mut tiles = vec![0; tiles_size];
	file.read_exact(&mut tiles)?;

	// Every bigmap entry indexes the tile table; an out-of-range index would
	// panic later when a cell is composed (`tile_pixels` slices the table).
	// Reject it at the trust boundary instead.
	if bigmap.iter().any(|&t| t >= tile_count) {
		return Err(WrlError::Parse("bigmap references a tile beyond the tile table"));
	}

	let palette_size = 256 * 3;
	let mut palette = vec![0; palette_size];
	file.read_exact(&mut palette)?;

	let mut pass_table = vec![0; tile_count as usize];
	file.read_exact(&mut pass_table)?;

	Ok(WrlFile { header, width, height, minimap, bigmap, tile_count, tiles, palette, pass_table })
}

/// Reads only the fields needed to render a minimap preview.
pub fn read_wrl_header(file_path: &Path) -> Result<WrlHeader, WrlError> {
	let mut file = File::open(file_path)?;

	file.seek(SeekFrom::Start(5))?;

	let width = read_u16_le(&mut file)?;
	let height = read_u16_le(&mut file)?;
	if width == 0 || height == 0 || width > MAX_WRL_DIM || height > MAX_WRL_DIM {
		return Err(WrlError::Parse("map dimensions out of range (1..=1024)"));
	}

	let mut minimap = vec![0; width as usize * height as usize];
	file.read_exact(&mut minimap)?;

	file.seek(SeekFrom::Current((width as i64) * (height as i64) * 2))?;

	let tile_count = read_u16_le(&mut file)?;

	file.seek(SeekFrom::Current((tile_count as i64) * TILE_DATA_SIZE as i64))?;

	let mut palette = vec![0; 256 * 3];
	file.read_exact(&mut palette)?;

	Ok(WrlHeader { width, height, tile_count, minimap, palette })
}

#[cfg(test)]
mod tests {
	use super::super::types::WrlFile;
	use super::super::write::wrl_to_bytes;
	use super::*;

	/// A scratch file path under the system temp dir, unique to this run.
	fn scratch(tag: &str) -> std::path::PathBuf {
		let pid = std::process::id();
		std::env::temp_dir().join(format!("mme-wrl-test-{pid}-{tag}.WRL"))
	}

	fn valid_wrl() -> WrlFile {
		WrlFile {
			header: vec![0; 5],
			width: 2,
			height: 1,
			minimap: vec![0; 2],
			bigmap: vec![0, 0],
			tile_count: 1,
			tiles: vec![0u8; TILE_DATA_SIZE],
			palette: vec![0u8; 768],
			pass_table: vec![0u8; 1],
		}
	}

	#[test]
	fn round_trips_a_valid_file() {
		let path = scratch("ok");
		std::fs::write(&path, wrl_to_bytes(&valid_wrl()).unwrap()).unwrap();
		let parsed = read_wrl_file(&path).unwrap();
		assert_eq!((parsed.width, parsed.height, parsed.tile_count), (2, 1, 1));
		let _ = std::fs::remove_file(&path);
	}

	#[test]
	fn rejects_bigmap_index_beyond_the_tile_table() {
		// SEV-3 regression: a bigmap entry >= tile_count would later panic in
		// `tile_pixels` when the cell is composed. It must be rejected at load.
		let mut wrl = valid_wrl();
		wrl.bigmap = vec![0, 9999]; // tile_count is 1
		let path = scratch("bigmap");
		std::fs::write(&path, wrl_to_bytes(&wrl).unwrap()).unwrap();
		let err = read_wrl_file(&path).unwrap_err();
		assert!(matches!(err, WrlError::Parse(m) if m.contains("bigmap")), "{err:?}");
		let _ = std::fs::remove_file(&path);
	}

	#[test]
	fn rejects_oversized_dimensions_before_allocating() {
		// SEV-3 regression: a 9-byte header claiming 65535x65535 must error
		// out before the multi-gigabyte minimap/bigmap allocations.
		let mut bytes = vec![0u8; 5]; // header
		bytes.extend_from_slice(&u16::MAX.to_le_bytes()); // width
		bytes.extend_from_slice(&u16::MAX.to_le_bytes()); // height
		let path = scratch("huge");
		std::fs::write(&path, &bytes).unwrap();
		let err = read_wrl_file(&path).unwrap_err();
		assert!(matches!(err, WrlError::Parse(m) if m.contains("dimensions")), "{err:?}");
		// The header reader guards the same way (minimap preview path).
		assert!(read_wrl_header(&path).is_err());
		let _ = std::fs::remove_file(&path);
	}

	#[test]
	fn rejects_zero_dimensions() {
		let mut bytes = vec![0u8; 5];
		bytes.extend_from_slice(&0u16.to_le_bytes());
		bytes.extend_from_slice(&8u16.to_le_bytes());
		let path = scratch("zero");
		std::fs::write(&path, &bytes).unwrap();
		assert!(read_wrl_file(&path).is_err());
		let _ = std::fs::remove_file(&path);
	}
}
