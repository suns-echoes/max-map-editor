//! Reader for the M.A.X. saved-game header (`SAVE#.DTA` and stock-mission
//! `.CAM/.SCE/.TRA/.MPS/.DMO` files).
//!
//! This decodes the header + game-options block only — enough to identify a
//! save and resolve the world it references. The surface/cargo maps and the
//! five unit lists that follow are decoded elsewhere (see `SAVE-FORMAT.md`).
//!
//! Layout mirrors M.A.X. Port's `SaveLoad_LoadFormatV70` / `...V71`
//! (`saveload.cpp`) and `SmartFileReader` (`smartfile.cpp`).

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use super::types::{SaveCategory, SaveFormat, SaveHeader, SaveOptions, TEAM_COUNT, world_file_name};

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
	#[error("I/O error: {0}")]
	Io(#[from] io::Error),
	#[error("unsupported save format version {0} (expected 70 or 71)")]
	UnsupportedVersion(u16),
	#[error("invalid save-file category {0}")]
	InvalidCategory(u32),
	/// Ran off the end of the buffer while decoding the body (offset, needed bytes).
	#[error("unexpected end of save data at offset {offset} (needed {needed} more bytes)")]
	UnexpectedEof { offset: usize, needed: usize },
	/// Full-body decode is only implemented for `V70` so far.
	#[error("save body decode not yet supported for format version {0}")]
	UnsupportedBody(u16),
	/// The object graph named a class index outside the 1..=6 registry.
	#[error("unknown object type index {0} in save graph at offset {offset}", offset = .1)]
	UnknownObjectType(u32, usize),
	/// An object reference resolved to a different class than expected.
	#[error("object #{index} is a {actual}, expected {expected}")]
	ObjectTypeMismatch { index: usize, expected: &'static str, actual: &'static str },
	/// An object reference was neither a back-reference into the already-read
	/// graph nor the one legal "next new object" index (`count + 1`).
	#[error("object reference {index} out of range at offset {offset} ({count} objects read so far)")]
	ObjectIndexOutOfRange { index: u32, count: u32, offset: usize },
	/// Inline object bodies nested past the decoder's depth limit — a corrupt or
	/// hostile file. Guards the recursive descent against a stack overflow.
	#[error("save object graph nested deeper than {limit} levels at offset {offset}")]
	ObjectGraphTooDeep { offset: usize, limit: u32 },
}

fn read_u8<R: Read>(r: &mut R) -> io::Result<u8> {
	let mut b = [0u8; 1];
	r.read_exact(&mut b)?;
	Ok(b[0])
}

fn read_u16<R: Read>(r: &mut R) -> io::Result<u16> {
	let mut b = [0u8; 2];
	r.read_exact(&mut b)?;
	Ok(u16::from_le_bytes(b))
}

fn read_u32<R: Read>(r: &mut R) -> io::Result<u32> {
	let mut b = [0u8; 4];
	r.read_exact(&mut b)?;
	Ok(u32::from_le_bytes(b))
}

fn read_i32<R: Read>(r: &mut R) -> io::Result<i32> {
	Ok(read_u32(r)? as i32)
}

/// Reads a fixed-width, NUL-terminated string field (`char[len]`, as used by
/// the `V70` header). Bytes past the first NUL are padding. Decoded lossily —
/// the game's text is CP437, but save/team names are effectively ASCII.
fn read_fixed_string<R: Read>(r: &mut R, len: usize) -> io::Result<String> {
	let mut buf = vec![0u8; len];
	r.read_exact(&mut buf)?;
	let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
	Ok(String::from_utf8_lossy(&buf[..end]).into_owned())
}

/// Reads a `V71` length-prefixed byte block (`u32` length, then that many bytes).
///
/// The length is untrusted - a raw `u32` straight out of the file - so the block
/// grows in bounded chunks as bytes actually arrive instead of being reserved up
/// front. A corrupt header claiming ~4 GiB then reports the short read it really
/// is, rather than aborting the process on a failed allocation.
fn read_v71_bytes<R: Read>(r: &mut R) -> io::Result<Vec<u8>> {
	const CHUNK: usize = 64 * 1024;
	let len = read_u32(r)? as usize;
	let mut buf = Vec::new();
	while buf.len() < len {
		let start = buf.len();
		buf.resize(start + (len - start).min(CHUNK), 0);
		r.read_exact(&mut buf[start..])?;
	}
	Ok(buf)
}

/// Reads a `V71` length-prefixed string (`u32` length, then that many bytes).
fn read_v71_string<R: Read>(r: &mut R) -> io::Result<String> {
	Ok(String::from_utf8_lossy(&read_v71_bytes(r)?).into_owned())
}

fn read_options<R: Read>(r: &mut R) -> io::Result<SaveOptions> {
	Ok(SaveOptions {
		world: read_i32(r)?,
		timer: read_i32(r)?,
		endturn: read_i32(r)?,
		start_gold: read_i32(r)?,
		play_mode: read_i32(r)?,
		victory_type: read_i32(r)?,
		victory_limit: read_i32(r)?,
		opponent: read_i32(r)?,
		raw_resource: read_i32(r)?,
		fuel_resource: read_i32(r)?,
		gold_resource: read_i32(r)?,
		alien_derelicts: read_i32(r)?,
	})
}

/// Reads the header + options block of a M.A.X. save file.
///
/// The format (`V70` vs `V71`) is detected from the leading version field, so
/// this transparently handles both the stock DOS saves in a M.A.X. install and
/// saves written by M.A.X. Port.
pub fn read_save_header(path: &Path) -> Result<SaveHeader, SaveError> {
	let mut file = File::open(path)?;
	read_header_from(&mut file)
}

/// Reads the version word, header, and options block from a stream positioned
/// at the start of a save file, leaving the cursor at the first byte of the
/// surface map. Shared by [`read_save_header`] and the full-body decoder
/// (`decode::read_save`), which needs the post-options position to continue.
pub(crate) fn read_header_from<R: Read>(r: &mut R) -> Result<SaveHeader, SaveError> {
	// The format is identified by the first 16 bits of the version field (which
	// is `u16` in V70 and `u32` in V71 — its low half is the version number and
	// the high half is zero, so a `u16` peek disambiguates either way).
	let version = read_u16(r)?;
	match version {
		70 => read_header_v70(r),
		71 => {
			// Consume the high half of V71's `u32` version field.
			let _ = read_u16(r)?;
			read_header_v71(r)
		}
		other => Err(SaveError::UnsupportedVersion(other)),
	}
}

fn read_header_v70<R: Read>(r: &mut R) -> Result<SaveHeader, SaveError> {
	let game_type = read_u8(r)?;
	let category = SaveCategory::from_game_type(game_type).ok_or(SaveError::InvalidCategory(game_type as u32))?;

	let save_name = read_fixed_string(r, 30)?;
	let world_index = read_u8(r)?;
	let mission_index = read_u16(r)?;

	// team_names[4][30] — the alien slot (index 4) has no name in V70.
	let mut team_names = std::array::from_fn::<String, TEAM_COUNT, _>(|_| String::new());
	for name in team_names.iter_mut().take(4) {
		*name = read_fixed_string(r, 30)?;
	}

	// team_type[5] / team_clan[5] — one byte each in V70, widened to u32.
	let mut team_type = [0u32; TEAM_COUNT];
	for t in team_type.iter_mut() {
		*t = read_u8(r)? as u32;
	}
	let mut team_clan = [0u32; TEAM_COUNT];
	for c in team_clan.iter_mut() {
		*c = read_u8(r)? as u32;
	}

	let rng_seed = read_u32(r)?;

	// Pre-options block (opponent:i8, turn_timer:u16, endturn:u16, play_mode:i8).
	// These are superseded by the options block that follows; consume and drop.
	let _ = read_u8(r)?; // opponent
	let _ = read_u16(r)?; // turn_timer_time
	let _ = read_u16(r)?; // endturn_time
	let _ = read_u8(r)?; // play_mode

	let options = read_options(r)?;

	Ok(SaveHeader {
		format: SaveFormat::V70,
		category,
		save_name,
		world_index: Some(world_index),
		world_file: world_file_name(world_index),
		world_hash: None,
		mission_index,
		script: Vec::new(),
		team_names,
		team_type,
		team_clan,
		rng_seed,
		options,
	})
}

fn read_header_v71<R: Read>(r: &mut R) -> Result<SaveHeader, SaveError> {
	let category_index = read_u32(r)?;
	let category =
		SaveCategory::from_mission_category(category_index).ok_or(SaveError::InvalidCategory(category_index))?;

	let script = read_v71_bytes(r)?;
	let save_name = read_v71_string(r)?;
	let world_hash = read_v71_string(r)?;
	let world_index = world_index_from_hash(&world_hash);

	let mut team_names = std::array::from_fn::<String, TEAM_COUNT, _>(|_| String::new());
	for name in team_names.iter_mut() {
		*name = read_v71_string(r)?;
	}
	let mut team_type = [0u32; TEAM_COUNT];
	for t in team_type.iter_mut() {
		*t = read_u32(r)?;
	}
	let mut team_clan = [0u32; TEAM_COUNT];
	for c in team_clan.iter_mut() {
		*c = read_u32(r)?;
	}
	// team difficulty levels — consumed, not surfaced here.
	for _ in 0..TEAM_COUNT {
		let _ = read_u32(r)?;
	}

	let rng_seed = read_u32(r)?;

	// Pre-options block (turn_timer, endturn, play_mode — all u32 in V71).
	let _ = read_u32(r)?;
	let _ = read_u32(r)?;
	let _ = read_u32(r)?;

	let options = read_options(r)?;

	Ok(SaveHeader {
		format: SaveFormat::V71,
		category,
		save_name,
		world_index,
		world_file: world_index.and_then(world_file_name),
		world_hash: Some(world_hash),
		mission_index: 0,
		script,
		team_names,
		team_type,
		team_clan,
		rng_seed,
		options,
	})
}

/// SHA-256 content hashes of the 24 stock worlds, in `world_index` order
/// (`SaveLoad_TranslateWorldIndexToHashKey`). Used to resolve a `V71` save's
/// stored world hash back to a stock index.
const WORLD_HASHES: [&str; 24] = [
	"c3f5c757d02197efab583091287a046b4b8991f2f0634281a7dd300d5fb365b0",
	"e90592f415271319fa53e8ec3605b4ff6abfc55915e938a91eedfea3d3ea9172",
	"a625d409ca1d5cd24cdeed165813e5b79c538b123491791016372492f5106801",
	"a6367393db48e1621d5ac247c4c4b2aac07885317f3ca40d0f12b81a28596f2e",
	"11a9255e65af53b6438803d0aef4ca96ed9d57575da3cf13779838ccda25a6e9",
	"d86feaba2d0af91065030a7aad2e206c3e2d26f780074ad436a84a5f7de226ee",
	"1c7fd2dbf663bd59e44b805d4eefe01679e25ffea8eb68337adfca5ec6eba7c6",
	"f18045f9dd315bb2c91dce05efb196924f9fcdd64a873482b997c432af1a9b12",
	"91b3ed80b33560e0e14c61120a6bb9ef7d2dfa44a159c130ea78f944cb5b9c64",
	"3749ceabc5b53aa3e7126d8751479725786d93fc800cf4b3e59b32d22dd301bd",
	"2646e4c177bccf19e88a78365f18fda48e06c0f8e0aa62ebd442b6ab2de646e1",
	"f2a2b010c280a8a73f0a7e6af549eb318b7b981463c366a7511a4725a138d393",
	"fad64e38a8c405afd9dfe8a905e2b4d167c671d02f4422500f300175b839c321",
	"4663efd15a19949ffd6fb0f2b7a23475494d54c51c670f500ec5c0b5c0a95645",
	"fa3da4a51d99e9a69e28de3aadb984789a8f79016a1c286971fc2e8340e0b6b0",
	"dbcfc4495334640776e6a0b1776651f904583aa87f193a43436e6b1f04635241",
	"315a11f6a4cc9cf6066a9cba9f84fb2dde7df9ad3b3025779d06268f8938dcc8",
	"f1fcedbc7be8571ffad83133711a8d267ffa07d05b207dc8e0e9353e05e34204",
	"2a1e65b4e9d95c6edc3f6da12d85c1d801df860fac562bc60f612c6fc2087ad8",
	"ef341cff54196289740ea3156423897b0a4135741d8214f8c1104cba3ba7f37a",
	"5877fc3c584558330683909b0ec153712e25f3337e975b590a0cacca3d8b4c7a",
	"5a6ac1733af876ed603d076f03106ac0a46977d43dc29d8c2885684ab3269b30",
	"33771f1cd7b3b321dca8667976f3e1fcadbf9c2c978e509fda1819291f0cf80c",
	"ddcba1014d11165e219d17396457a640376b22ca099525264fc7f3a509a138e9",
];

/// Resolves a `V71` world content hash to its stock `world_index`, or `None`
/// for a custom (non-stock) world.
pub fn world_index_from_hash(hash: &str) -> Option<u8> {
	WORLD_HASHES.iter().position(|&h| h == hash).map(|i| i as u8)
}

/// The stock world hash for a `world_index` (0..=23) — what a `V71` save
/// stores in its header for that slot. The inverse of
/// [`world_index_from_hash`]; save synthesis writes it so the engine resolves
/// the world by slot (the swapped-`.WRL` workflow puts the actual map there).
pub fn stock_world_hash(world_index: u8) -> Option<&'static str> {
	WORLD_HASHES.get(world_index as usize).copied()
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::Cursor;

	fn push_fixed(buf: &mut Vec<u8>, text: &str, len: usize) {
		let mut bytes = text.as_bytes().to_vec();
		bytes.resize(len, 0);
		buf.extend_from_slice(&bytes);
	}

	/// A synthetic V70 header body (everything after the 2-byte version field),
	/// exercising the exact layout verified against the real `~/MAX` saves.
	#[test]
	fn parses_a_synthetic_v70_header() {
		let mut buf = Vec::new();
		buf.push(1); // game_type = training
		push_fixed(&mut buf, "Test Save", 30); // save_name
		buf.push(16); // world_index -> GREEN_5
		buf.extend_from_slice(&1u16.to_le_bytes()); // mission_index
		push_fixed(&mut buf, "Red", 30);
		push_fixed(&mut buf, "Green", 30);
		push_fixed(&mut buf, "", 30);
		push_fixed(&mut buf, "", 30);
		buf.extend_from_slice(&[1, 2, 0, 0, 0]); // team_type
		buf.extend_from_slice(&[3, 7, 0, 0, 0]); // team_clan
		buf.extend_from_slice(&0xDEAD_BEEFu32.to_le_bytes()); // rng_seed
		buf.push(3); // opponent (i8)
		buf.extend_from_slice(&180u16.to_le_bytes()); // turn_timer
		buf.extend_from_slice(&40u16.to_le_bytes()); // endturn
		buf.push(1); // play_mode (i8)
		for v in [16i32, 180, 40, 200, 1, 0, 30, 3, 1, 1, 1, 0] {
			buf.extend_from_slice(&v.to_le_bytes());
		}

		let mut cursor = Cursor::new(buf);
		let h = read_header_v70(&mut cursor).unwrap();

		assert_eq!(h.format, SaveFormat::V70);
		assert_eq!(h.category, SaveCategory::Training);
		assert_eq!(h.save_name, "Test Save");
		assert_eq!(h.world_index, Some(16));
		assert_eq!(h.world_file, Some("GREEN_5.WRL"));
		assert_eq!(h.team_names[0], "Red");
		assert_eq!(h.team_names[1], "Green");
		assert_eq!(h.team_type, [1, 2, 0, 0, 0]);
		assert_eq!(h.team_clan, [3, 7, 0, 0, 0]);
		assert_eq!(h.rng_seed, 0xDEAD_BEEF);
		assert_eq!(h.options.start_gold, 200);
		assert_eq!(h.options.victory_limit, 30);
		// The full body was consumed exactly, with nothing left over.
		assert_eq!(cursor.position() as usize, cursor.get_ref().len());
	}

	/// Guards the SNOW/CRATER/GREEN/DESERT `world_index` order. The editor's
	/// display list (`INSTALLED_MAP_FILE_NAMES`) swaps GREEN and DESERT, which
	/// would mis-resolve a save's world — verified live against SAVE1.CAM
	/// (index 16 = GREEN_5) and SAVE1.MPS (index 23 = DESERT_6).
	#[test]
	fn world_index_order_is_snow_crater_green_desert() {
		assert_eq!(world_file_name(0), Some("SNOW_1.WRL"));
		assert_eq!(world_file_name(6), Some("CRATER_1.WRL"));
		assert_eq!(world_file_name(16), Some("GREEN_5.WRL"));
		assert_eq!(world_file_name(23), Some("DESERT_6.WRL"));
		assert_eq!(world_file_name(24), None);
	}

	#[test]
	fn rejects_an_unknown_version() {
		let path = std::env::temp_dir().join(format!("mme-save-badver-{}.dta", std::process::id()));
		std::fs::write(&path, 999u16.to_le_bytes()).unwrap();
		let err = read_save_header(&path).unwrap_err();
		assert!(matches!(err, SaveError::UnsupportedVersion(999)), "{err:?}");
		let _ = std::fs::remove_file(&path);
	}

	/// S0.6 acceptance: the hand-rolled SHA-256 reproduces the embedded
	/// stock-world hashes. Hashing each pristine `.WRL` under
	/// `testdata/originals/` (the exact whole-file hash `World::ComputeHash`
	/// computes) must equal `WORLD_HASHES[index]`. Gated on the bundled maps.
	#[test]
	fn world_hashes_match_pristine_wrls() {
		let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/originals");
		if !dir.is_dir() {
			crate::testutil::skip_fixture("world_hashes_match_pristine_wrls: testdata/originals not found");
			return;
		}
		let mut checked = 0;
		for (index, expected) in WORLD_HASHES.iter().enumerate() {
			let file = crate::save::WORLD_FILE_NAMES[index];
			let path = dir.join(file);
			if !path.is_file() {
				continue;
			}
			let got = crate::sha256::sha256_file(&path).unwrap();
			assert_eq!(&got, expected, "SHA-256 of {file} must match WORLD_HASHES[{index}]");
			checked += 1;
		}
		assert!(checked > 0, "no pristine stock .WRL files found under testdata/originals to check");
	}
}
