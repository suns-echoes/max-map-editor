//! MAX.RES archive format.
//!
//! Header = 4-byte id + `i32` index offset + `i32` index size.
//! Index table = array of 8-byte tag + `i32` data offset + `i32` data size.
//! Entries are raw blobs - the caller decides how to decode each one based on
//! the `A_` / `D_` / `F_` / `I_` / `P_` / `S_` / `V_` tag prefix.

pub mod manifest;
pub use manifest::{ResManifest, SimpleImageEntry, load_res_manifest, parse_res_manifest};

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct ResEntry {
	pub tag: String,
	pub data_offset: i32,
	pub data_size: i32,
}

#[derive(Debug)]
pub struct ResArchive {
	pub entries: Vec<ResEntry>,
}

/// Reads the index of a RES archive without extracting any entry data.
pub fn read_res_index(source_file: &Path) -> io::Result<ResArchive> {
	let res_file = std::fs::File::open(source_file)?;
	let mut reader = std::io::BufReader::new(res_file);

	let mut id = [0u8; 4];
	reader.read_exact(&mut id)?;

	let mut offset_bytes = [0u8; 4];
	reader.read_exact(&mut offset_bytes)?;
	let offset = i32::from_le_bytes(offset_bytes);

	let mut size_bytes = [0u8; 4];
	reader.read_exact(&mut size_bytes)?;
	let size = i32::from_le_bytes(size_bytes);

	// Per-entry offsets/sizes are checked below; the *index*'s own pair was not,
	// so a negative one sign-extended into an absurd `u64` seek. The seek fails
	// and surfaces as an error either way - reject it here so the two checks
	// agree and the fields can be trusted from this point on.
	if offset < 0 || size < 0 {
		return Err(io::Error::new(io::ErrorKind::InvalidData, "RES header has a negative index offset or size"));
	}

	reader.seek(SeekFrom::Start(offset as u64))?;

	let entry_size = 8 + 4 + 4;
	let mut entries = Vec::new();
	for _ in 0..size / entry_size {
		let mut tag = [0u8; 8];
		reader.read_exact(&mut tag)?;

		let mut data_offset_bytes = [0u8; 4];
		reader.read_exact(&mut data_offset_bytes)?;
		let data_offset = i32::from_le_bytes(data_offset_bytes);

		let mut data_size_bytes = [0u8; 4];
		reader.read_exact(&mut data_size_bytes)?;
		let data_size = i32::from_le_bytes(data_size_bytes);

		// A negative offset/size in a malformed archive would become a huge
		// `usize` at the `vec![0u8; data_size as usize]` reads below - reject
		// it here so consumers can trust these fields.
		if data_offset < 0 || data_size < 0 {
			return Err(io::Error::new(io::ErrorKind::InvalidData, "RES entry has a negative offset or size"));
		}

		let tag_str = String::from_utf8_lossy(&tag).trim_end_matches(char::from(0)).to_string();

		entries.push(ResEntry { tag: tag_str, data_offset, data_size });
	}

	Ok(ResArchive { entries })
}

/// Extracts every entry into `out_dir`, one file per tag.
pub fn extract_res_file(source_file: &Path, out_dir: &Path) -> io::Result<()> {
	std::fs::create_dir_all(out_dir)?;

	let archive = read_res_index(source_file)?;

	let res_file = std::fs::File::open(source_file)?;
	let mut reader = std::io::BufReader::new(res_file);

	for entry in &archive.entries {
		let resource_file_path = out_dir.join(&entry.tag);
		let mut resource_file = std::fs::File::create(&resource_file_path)?;

		reader.seek(SeekFrom::Start(entry.data_offset as u64))?;
		let mut resource_data = vec![0u8; entry.data_size as usize];
		reader.read_exact(&mut resource_data).map_err(|e| {
			log::error!(
				"Failed to read resource data for {} (offset {}, size {}): {}",
				entry.tag,
				entry.data_offset,
				entry.data_size,
				e
			);
			e
		})?;
		resource_file.write_all(&resource_data)?;
	}

	Ok(())
}

/// Reads a single entry's raw bytes.
pub fn read_res_entry(source_file: &Path, tag: &str) -> io::Result<Option<Vec<u8>>> {
	let archive = read_res_index(source_file)?;
	let Some(entry) = archive.entries.iter().find(|e| e.tag == tag) else {
		return Ok(None);
	};

	let res_file = std::fs::File::open(source_file)?;
	let mut reader = std::io::BufReader::new(res_file);
	reader.seek(SeekFrom::Start(entry.data_offset as u64))?;
	let mut buf = vec![0u8; entry.data_size as usize];
	reader.read_exact(&mut buf)?;
	Ok(Some(buf))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn scratch(tag: &str) -> std::path::PathBuf {
		std::env::temp_dir().join(format!("mme-res-test-{}-{tag}.RES", std::process::id()))
	}

	/// Build a minimal RES archive: 4-byte id, i32 index offset, i32 index
	/// size, then one index entry (8-byte tag + i32 data offset + i32 size).
	fn build(data_offset: i32, data_size: i32) -> Vec<u8> {
		let mut out = Vec::new();
		out.extend_from_slice(b"RES0"); // id (4) + offset (4) + size (4) = 12-byte header
		out.extend_from_slice(&12i32.to_le_bytes()); // index offset (right after the header)
		out.extend_from_slice(&16i32.to_le_bytes()); // index size = one 16-byte entry
		let mut tag = *b"TESTTAG\0";
		tag[7] = 0;
		out.extend_from_slice(&tag);
		out.extend_from_slice(&data_offset.to_le_bytes());
		out.extend_from_slice(&data_size.to_le_bytes());
		out
	}

	#[test]
	fn reads_a_well_formed_index() {
		let path = scratch("ok");
		std::fs::write(&path, build(0, 0)).unwrap();
		let archive = read_res_index(&path).unwrap();
		assert_eq!(archive.entries.len(), 1);
		assert_eq!(archive.entries[0].tag, "TESTTAG");
		let _ = std::fs::remove_file(&path);
	}

	#[test]
	fn rejects_negative_entry_size() {
		// SEV-4 regression: a negative i32 size would become a ~18-exabyte
		// `usize` at the `vec![0u8; size]` allocation. Reject at index read.
		let path = scratch("negsize");
		std::fs::write(&path, build(0, -1)).unwrap();
		let err = read_res_index(&path).unwrap_err();
		assert_eq!(err.kind(), io::ErrorKind::InvalidData);
		let _ = std::fs::remove_file(&path);
	}

	#[test]
	fn rejects_negative_entry_offset() {
		let path = scratch("negoff");
		std::fs::write(&path, build(-5, 8)).unwrap();
		assert!(read_res_index(&path).is_err());
		let _ = std::fs::remove_file(&path);
	}

	/// The index's own offset/size pair gets the same check the entries get -
	/// a negative one sign-extends into an absurd `u64` seek.
	#[test]
	fn rejects_a_negative_index_offset_or_size() {
		for (offset, size) in [(-1i32, 16i32), (12, -1), (-1, -1)] {
			let path = scratch(&format!("negidx{offset}_{size}"));
			let mut out = Vec::new();
			out.extend_from_slice(b"RES0");
			out.extend_from_slice(&offset.to_le_bytes());
			out.extend_from_slice(&size.to_le_bytes());
			out.extend_from_slice(&[0u8; 16]);
			std::fs::write(&path, out).unwrap();
			let err = read_res_index(&path).unwrap_err();
			assert_eq!(err.kind(), io::ErrorKind::InvalidData, "offset {offset}, size {size}");
			let _ = std::fs::remove_file(&path);
		}
	}

	/// A scratch directory path unique to this run.
	fn scratch_dir(tag: &str) -> std::path::PathBuf {
		std::env::temp_dir().join(format!("mme-res-test-{}-{tag}", std::process::id()))
	}

	/// Archive with real payloads: 12-byte header, the data blobs back to
	/// back, then the index table at the end (as MAX.RES lays it out).
	fn build_with_data(entries: &[(&str, &[u8])]) -> Vec<u8> {
		let mut data = Vec::new();
		let mut index = Vec::new();
		let mut offset = 12i32;
		for (tag, blob) in entries {
			let mut tag8 = [0u8; 8];
			tag8[..tag.len()].copy_from_slice(tag.as_bytes());
			index.extend_from_slice(&tag8);
			index.extend_from_slice(&offset.to_le_bytes());
			index.extend_from_slice(&(blob.len() as i32).to_le_bytes());
			data.extend_from_slice(blob);
			offset += blob.len() as i32;
		}
		let mut out = Vec::new();
		out.extend_from_slice(b"RES0");
		out.extend_from_slice(&offset.to_le_bytes()); // index sits right after the data
		out.extend_from_slice(&(index.len() as i32).to_le_bytes());
		out.extend_from_slice(&data);
		out.extend_from_slice(&index);
		out
	}

	/// `read_res_entry` returns the exact blob for a present tag and
	/// `Ok(None)` - not an error - for an absent one.
	#[test]
	fn read_res_entry_finds_a_tag_or_reports_none() {
		let path = scratch("entry");
		std::fs::write(&path, build_with_data(&[("FOO", b"HELLO"), ("BAR", b"XY")])).unwrap();
		assert_eq!(read_res_entry(&path, "FOO").unwrap().as_deref(), Some(&b"HELLO"[..]), "first entry's bytes");
		assert_eq!(read_res_entry(&path, "BAR").unwrap().as_deref(), Some(&b"XY"[..]), "second entry's bytes");
		assert!(read_res_entry(&path, "MISSING").unwrap().is_none(), "an unknown tag is None, not an error");
		let _ = std::fs::remove_file(&path);
	}

	/// `extract_res_file` creates the output directory and writes one file
	/// per tag, byte-identical to the archived blob.
	#[test]
	fn extract_res_file_writes_one_file_per_tag() {
		let path = scratch("extract");
		let out_root = scratch_dir("extract-out");
		let out_dir = out_root.join("nested"); // exercises create_dir_all
		std::fs::write(&path, build_with_data(&[("FOO", b"HELLO"), ("BAR", b"XY")])).unwrap();
		extract_res_file(&path, &out_dir).unwrap();
		assert_eq!(std::fs::read(out_dir.join("FOO")).unwrap(), b"HELLO", "FOO extracted verbatim");
		assert_eq!(std::fs::read(out_dir.join("BAR")).unwrap(), b"XY", "BAR extracted verbatim");
		let _ = std::fs::remove_file(&path);
		let _ = std::fs::remove_dir_all(&out_root);
	}

	/// An entry whose claimed size runs past EOF fails extraction with an
	/// error instead of silently writing a short file.
	#[test]
	fn extract_res_file_errors_when_an_entry_runs_past_eof() {
		let path = scratch("eof");
		let out_dir = scratch_dir("eof-out");
		std::fs::write(&path, build(12, 9999)).unwrap(); // 9999-byte entry in a 28-byte file
		assert!(extract_res_file(&path, &out_dir).is_err(), "the truncated read must surface as an error");
		let _ = std::fs::remove_file(&path);
		let _ = std::fs::remove_dir_all(&out_dir);
	}
}
