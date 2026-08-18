use super::palette::FRAMEPIC_PALETTE_BGRA;
use super::types::{ImageData, MAX_IMAGE_HEIGHT, MAX_IMAGE_WIDTH, MaxType};
use crate::color::indexed_to_color;

/// One decoded frame with palette-indexed pixels retained (index 0 is
/// transparent). Sprite atlases consume this directly; the renderer samples
/// the game's current palette via the shared LUT.
#[derive(Debug, Clone)]
pub struct IndexedFrame {
	pub width: u32,
	pub height: u32,
	/// Signed - MAX sprites may anchor above/left of the sprite rectangle,
	/// which needs a negative value; an unsigned cast of `i16` wraps and
	/// flings the frame off-screen.
	pub hot_spot_x: i32,
	pub hot_spot_y: i32,
	pub pixels: Vec<u8>, // width * height palette indices
}

/// Decodes the first frame of a multi-image.
///
/// Multi-images encode animations or rotations as a series of frames, each
/// with per-row transparency RLE.
pub fn parse_multi_image(data: &[u8]) -> Result<Option<ImageData>, String> {
	if data.len() < 20 {
		return Ok(None);
	}

	let image_count = i16::from_le_bytes(data[0..2].try_into().map_err(|_| "Invalid image count")?);
	let first_offset = i32::from_le_bytes(data[2..6].try_into().map_err(|_| "Invalid frame offset")?);
	let first_frame_offset = first_offset as usize;

	if image_count <= 0 || first_frame_offset != 2 + 4 * image_count as usize {
		return Ok(None);
	}

	Ok(parse_frames(data, first_frame_offset))
}

/// Decodes every frame of a multi-image.
pub fn parse_multi_image_all_frames(data: &[u8]) -> Result<Option<Vec<ImageData>>, String> {
	if data.len() < 20 {
		return Ok(None);
	}

	let image_count = i16::from_le_bytes(data[0..2].try_into().map_err(|_| "Invalid image count")?);
	let mut frames_offsets = vec![i32::from_le_bytes(data[2..6].try_into().map_err(|_| "Invalid frame offset")?)];
	let first_frame_offset = frames_offsets[0] as usize;

	if image_count <= 0 || first_frame_offset != 2 + 4 * image_count as usize {
		return Ok(None);
	}

	for i in 1..image_count as usize {
		let offset = i32::from_le_bytes(
			data.get(2 + i * 4..2 + (i + 1) * 4)
				.ok_or("truncated frame offset table")?
				.try_into()
				.map_err(|_| "Invalid frame offset")?,
		);
		frames_offsets.push(offset);
	}

	let mut all_frames: Vec<ImageData> = Vec::new();
	for frame_offset in &frames_offsets {
		if let Some(frame_data) = parse_frames(data, *frame_offset as usize) {
			all_frames.push(frame_data);
		}
	}

	if all_frames.is_empty() {
		return Ok(None);
	}

	Ok(Some(all_frames))
}

fn parse_frames(data: &[u8], offset: usize) -> Option<ImageData> {
	// `offset` is a raw `i32` from the file, sign-extended by the caller's
	// `as usize`. A negative one becomes ~2^64, so `offset + 8` overflows -
	// which, with the release profile's `overflow-checks`, aborts the process
	// instead of rejecting the frame. Add checked so a bad offset is just None.
	if data.len() < offset.checked_add(8)? {
		return None;
	}

	let width = i16::from_le_bytes(data[offset..offset + 2].try_into().ok()?);
	let height = i16::from_le_bytes(data[offset + 2..offset + 4].try_into().ok()?);
	let hot_spot_x = i16::from_le_bytes(data[offset + 4..offset + 6].try_into().ok()?) as i32;
	let hot_spot_y = i16::from_le_bytes(data[offset + 6..offset + 8].try_into().ok()?) as i32;

	if width <= 0 || height <= 0 || width > MAX_IMAGE_WIDTH || height > MAX_IMAGE_HEIGHT {
		return None;
	}

	let mut row_offsets: Vec<i32> = Vec::new();
	for i in 0..height {
		let start_offset = offset.checked_add(8)?.checked_add(i as usize * 4)?;
		let row_offset = i32::from_le_bytes(data.get(start_offset..start_offset + 4)?.try_into().ok()?);
		row_offsets.push(row_offset);
	}

	if let Some(shadow_image) = decode_shadow_frame(data, &row_offsets, width, height, hot_spot_x, hot_spot_y) {
		return Some(shadow_image);
	}

	decode_image_frame(data, &row_offsets, width, height, hot_spot_x, hot_spot_y)
}

fn decode_shadow_frame(
	data: &[u8],
	row_offsets: &[i32],
	width: i16,
	height: i16,
	hot_spot_x: i32,
	hot_spot_y: i32,
) -> Option<ImageData> {
	const SHADOW_COLOR_INDEX: u8 = 20;

	let mut indexed_image_data: Vec<u8> = vec![0; width as usize * height as usize];
	let mut data_offset: usize = row_offsets[0] as usize;
	let mut out_offset: usize = 0;

	for y in 0..height {
		let expected_offset = row_offsets[y as usize] as usize;
		let mut remaining_row_length = width as usize;

		if data_offset != expected_offset {
			return None;
		}

		loop {
			let transparent_count = *data.get(data_offset)? as usize;
			data_offset += 1;

			if transparent_count == 0xff {
				break;
			} else if transparent_count > remaining_row_length {
				return None;
			}

			out_offset += transparent_count;
			remaining_row_length -= transparent_count;

			let shadow_count = *data.get(data_offset)? as usize;
			data_offset += 1;

			if shadow_count == 0 {
				continue;
			} else if shadow_count > remaining_row_length {
				return None;
			}

			let dest_slice = &mut indexed_image_data[out_offset..out_offset + shadow_count];
			dest_slice.fill(SHADOW_COLOR_INDEX);

			out_offset += shadow_count;
			remaining_row_length -= shadow_count;
		}

		if remaining_row_length > 0 {
			out_offset += remaining_row_length;
		}
	}

	let image_data = indexed_to_color(&indexed_image_data, &FRAMEPIC_PALETTE_BGRA);

	Some(ImageData {
		max_type: MaxType::MaxMultiShadow,
		width: width as u32,
		height: height as u32,
		hot_spot_x,
		hot_spot_y,
		data: image_data,
	})
}

fn decode_image_frame(
	data: &[u8],
	row_offsets: &[i32],
	width: i16,
	height: i16,
	hot_spot_x: i32,
	hot_spot_y: i32,
) -> Option<ImageData> {
	let mut indexed_image_data: Vec<u8> = vec![0; width as usize * height as usize];
	let mut data_offset: usize = row_offsets[0] as usize;
	let mut out_offset: usize = 0;

	for y in 0..height {
		let expected_offset = row_offsets[y as usize] as usize;
		let mut remaining_row_length = width as usize;

		if data_offset != expected_offset {
			return None;
		}

		loop {
			let transparent_count = *data.get(data_offset)? as usize;
			data_offset += 1;

			if transparent_count == 0xff {
				break;
			} else if transparent_count > remaining_row_length {
				return None;
			}

			out_offset += transparent_count;
			remaining_row_length -= transparent_count;

			let pixel_count = *data.get(data_offset)? as usize;
			data_offset += 1;

			if pixel_count == 0 {
				continue;
			} else if pixel_count > remaining_row_length {
				return None;
			}

			if data_offset + pixel_count > data.len() {
				break;
			}

			let src_slice = &data[data_offset..data_offset + pixel_count];
			let dest_slice = &mut indexed_image_data[out_offset..out_offset + pixel_count];
			dest_slice.copy_from_slice(src_slice);

			data_offset += pixel_count;
			out_offset += pixel_count;
			remaining_row_length -= pixel_count;
		}

		if remaining_row_length > 0 {
			out_offset += remaining_row_length;
		}
	}

	let image_data = indexed_to_color(&indexed_image_data, &FRAMEPIC_PALETTE_BGRA);

	Some(ImageData {
		max_type: MaxType::MaxMultiImage,
		width: width as u32,
		height: height as u32,
		hot_spot_x,
		hot_spot_y,
		data: image_data,
	})
}

/// Decodes every frame of a multi-image and keeps pixels in palette-index
/// form. Use this when the consumer is going to sample against the game's
/// own palette (unit sprites, tileset overlays) - no color-space conversion
/// happens here, so palette cycling "just works" downstream.
pub fn decode_multi_image_indexed(data: &[u8]) -> Result<Vec<IndexedFrame>, String> {
	if data.len() < 20 {
		return Err("input too short for multi-image".to_string());
	}

	let image_count = i16::from_le_bytes(data[0..2].try_into().map_err(|_| "invalid image count")?);
	let first_offset = i32::from_le_bytes(data[2..6].try_into().map_err(|_| "invalid frame offset")?) as usize;

	if image_count <= 0 || first_offset != 2 + 4 * image_count as usize {
		return Err("not a multi-image (header mismatch)".to_string());
	}

	let mut offsets = Vec::with_capacity(image_count as usize);
	offsets.push(first_offset as i32);
	for i in 1..image_count as usize {
		let start = 2 + i * 4;
		let off = i32::from_le_bytes(
			data.get(start..start + 4)
				.ok_or("truncated frame offset table")?
				.try_into()
				.map_err(|_| "invalid frame offset")?,
		);
		offsets.push(off);
	}

	let mut frames = Vec::with_capacity(offsets.len());
	for &off in &offsets {
		if let Some(f) = decode_frame_indexed(data, off as usize) {
			frames.push(f);
		}
	}

	if frames.is_empty() {
		return Err("no frames decoded".to_string());
	}

	Ok(frames)
}

/// Decodes a single frame as palette-indexed pixels. Unlike `parse_frames`,
/// this never falls back to the shadow-color path - shadows are a separate
/// visual layer handled by their own decode pass.
/// Parse a multi-image frame header at `offset`: the 8-byte
/// `(width, height, hot_spot_x, hot_spot_y)` block (dims validated `> 0` and
/// within the max), then the `height`-entry row-offset table. `None` on a
/// short / out-of-range / truncated header. Shared by the indexed body and
/// shadow decoders, whose row-RLE bodies differ but whose headers are identical.
fn read_multi_header(data: &[u8], offset: usize) -> Option<(i16, i16, i32, i32, Vec<i32>)> {
	// Checked for the same reason as `parse_frames` - a negative frame offset
	// out of the file sign-extends, and the bare `+ 8` would abort the process.
	if data.len() < offset.checked_add(8)? {
		return None;
	}
	let width = i16::from_le_bytes(data[offset..offset + 2].try_into().ok()?);
	let height = i16::from_le_bytes(data[offset + 2..offset + 4].try_into().ok()?);
	let hot_spot_x = i16::from_le_bytes(data[offset + 4..offset + 6].try_into().ok()?) as i32;
	let hot_spot_y = i16::from_le_bytes(data[offset + 6..offset + 8].try_into().ok()?) as i32;
	if width <= 0 || height <= 0 || width > MAX_IMAGE_WIDTH || height > MAX_IMAGE_HEIGHT {
		return None;
	}
	let mut row_offsets: Vec<i32> = Vec::with_capacity(height as usize);
	for i in 0..height {
		let s = offset.checked_add(8)?.checked_add(i as usize * 4)?;
		row_offsets.push(i32::from_le_bytes(data.get(s..s + 4)?.try_into().ok()?));
	}
	Some((width, height, hot_spot_x, hot_spot_y, row_offsets))
}

fn decode_frame_indexed(data: &[u8], offset: usize) -> Option<IndexedFrame> {
	let (width, height, hot_spot_x, hot_spot_y, row_offsets) = read_multi_header(data, offset)?;
	let mut pixels: Vec<u8> = vec![0; width as usize * height as usize];
	let mut data_offset: usize = row_offsets[0] as usize;
	let mut out_offset: usize = 0;

	for y in 0..height {
		let expected = row_offsets[y as usize] as usize;
		let mut remaining = width as usize;
		if data_offset != expected {
			return None;
		}

		loop {
			let transparent = *data.get(data_offset)? as usize;
			data_offset += 1;

			if transparent == 0xff {
				break;
			} else if transparent > remaining {
				return None;
			}

			out_offset += transparent;
			remaining -= transparent;

			let pixel_count = *data.get(data_offset)? as usize;
			data_offset += 1;

			if pixel_count == 0 {
				continue;
			} else if pixel_count > remaining {
				return None;
			}
			if data_offset + pixel_count > data.len() {
				break;
			}

			let src = &data[data_offset..data_offset + pixel_count];
			pixels[out_offset..out_offset + pixel_count].copy_from_slice(src);

			data_offset += pixel_count;
			out_offset += pixel_count;
			remaining -= pixel_count;
		}

		if remaining > 0 {
			out_offset += remaining;
		}
	}

	Some(IndexedFrame { width: width as u32, height: height as u32, hot_spot_x, hot_spot_y, pixels })
}

/// Palette index used to mark "shadow pixel" in decoded shadow frames.
/// Chosen to match the original RES-extractor convention - any non-zero
/// value works downstream since the shadow pipeline only tests for opacity.
const SHADOW_MARKER: u8 = 20;

/// Decodes a shadow multi-image (`S_*` RES tag). Shadow frames use a
/// different row RLE than body frames: each pair is `(transparent, shadow)`
/// with no pixel payload between them. Output pixels are `0` for transparent
/// and `SHADOW_MARKER` for shadow.
pub fn decode_multi_image_shadow_indexed(data: &[u8]) -> Result<Vec<IndexedFrame>, String> {
	if data.len() < 20 {
		return Err("input too short for multi-image".to_string());
	}

	let image_count = i16::from_le_bytes(data[0..2].try_into().map_err(|_| "invalid image count")?);
	let first_offset = i32::from_le_bytes(data[2..6].try_into().map_err(|_| "invalid frame offset")?) as usize;

	if image_count <= 0 || first_offset != 2 + 4 * image_count as usize {
		return Err("not a multi-image (header mismatch)".to_string());
	}

	let mut offsets = Vec::with_capacity(image_count as usize);
	offsets.push(first_offset as i32);
	for i in 1..image_count as usize {
		let start = 2 + i * 4;
		let off = i32::from_le_bytes(
			data.get(start..start + 4)
				.ok_or("truncated frame offset table")?
				.try_into()
				.map_err(|_| "invalid frame offset")?,
		);
		offsets.push(off);
	}

	let mut frames = Vec::with_capacity(offsets.len());
	for &off in &offsets {
		if let Some(f) = decode_frame_shadow_indexed(data, off as usize) {
			frames.push(f);
		}
	}

	if frames.is_empty() {
		return Err("no frames decoded".to_string());
	}

	Ok(frames)
}

fn decode_frame_shadow_indexed(data: &[u8], offset: usize) -> Option<IndexedFrame> {
	let (width, height, hot_spot_x, hot_spot_y, row_offsets) = read_multi_header(data, offset)?;
	let mut pixels: Vec<u8> = vec![0; width as usize * height as usize];
	let mut data_offset: usize = row_offsets[0] as usize;
	let mut out_offset: usize = 0;

	for y in 0..height {
		let expected = row_offsets[y as usize] as usize;
		let mut remaining = width as usize;
		if data_offset != expected {
			return None;
		}

		loop {
			let transparent = *data.get(data_offset)? as usize;
			data_offset += 1;

			if transparent == 0xff {
				break;
			} else if transparent > remaining {
				return None;
			}

			out_offset += transparent;
			remaining -= transparent;

			let shadow_count = *data.get(data_offset)? as usize;
			data_offset += 1;

			if shadow_count == 0 {
				continue;
			} else if shadow_count > remaining {
				return None;
			}

			pixels[out_offset..out_offset + shadow_count].fill(SHADOW_MARKER);
			out_offset += shadow_count;
			remaining -= shadow_count;
		}

		if remaining > 0 {
			out_offset += remaining;
		}
	}

	Some(IndexedFrame { width: width as u32, height: height as u32, hot_spot_x, hot_spot_y, pixels })
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A minimal but structurally valid single-frame multi-image:
	/// image_count = 1, one 2x1 body row encoded as (0 transparent, 2 pixels, end).
	fn valid_single_frame_blob() -> Vec<u8> {
		let mut d = Vec::new();
		d.extend_from_slice(&1i16.to_le_bytes()); // image_count
		d.extend_from_slice(&6i32.to_le_bytes()); // first frame offset (= 2 + 4*1)
		d.extend_from_slice(&2i16.to_le_bytes()); // width
		d.extend_from_slice(&1i16.to_le_bytes()); // height
		d.extend_from_slice(&0i16.to_le_bytes()); // hot_spot_x
		d.extend_from_slice(&0i16.to_le_bytes()); // hot_spot_y
		let rle_pos = d.len() + 4; // one row-offset entry precedes the RLE
		d.extend_from_slice(&(rle_pos as i32).to_le_bytes()); // row 0 offset
		d.push(0); // transparent run = 0
		d.push(2); // literal run = 2 pixels
		d.push(10);
		d.push(11);
		d.push(0xff); // end of row
		d
	}

	#[test]
	fn decodes_valid_blob() {
		assert!(parse_multi_image(&valid_single_frame_blob()).unwrap().is_some());
	}

	#[test]
	fn truncating_at_every_length_never_panics() {
		let d = valid_single_frame_blob();
		for len in 0..=d.len() {
			let s = &d[..len];
			let _ = parse_multi_image(s);
			let _ = parse_multi_image_all_frames(s);
			let _ = decode_multi_image_indexed(s);
			let _ = decode_multi_image_shadow_indexed(s);
		}
	}

	#[test]
	fn oversized_image_count_does_not_over_read() {
		// Header passes (first_offset == 2 + 4*image_count) but the claimed
		// 1000-entry offset table runs far past the 40-byte buffer.
		let mut d = Vec::new();
		d.extend_from_slice(&1000i16.to_le_bytes());
		d.extend_from_slice(&4002i32.to_le_bytes());
		d.resize(40, 0);
		let _ = parse_multi_image(&d);
		let _ = parse_multi_image_all_frames(&d);
		let _ = decode_multi_image_indexed(&d);
		let _ = decode_multi_image_shadow_indexed(&d);
	}

	/// One-frame multi-image blob: header (image_count = 1, first offset = 6),
	/// a `width`x`height` frame with zero hot-spots, and each row's RLE bytes
	/// laid out back-to-back behind a consistent row-offset table.
	fn one_frame_blob(width: i16, height: i16, rows: &[&[u8]]) -> Vec<u8> {
		let mut d = Vec::new();
		d.extend_from_slice(&1i16.to_le_bytes());
		d.extend_from_slice(&6i32.to_le_bytes());
		d.extend_from_slice(&width.to_le_bytes());
		d.extend_from_slice(&height.to_le_bytes());
		d.extend_from_slice(&0i16.to_le_bytes()); // hot_spot_x
		d.extend_from_slice(&0i16.to_le_bytes()); // hot_spot_y
		let mut row_start = 6 + 8 + 4 * rows.len();
		for row in rows {
			d.extend_from_slice(&(row_start as i32).to_le_bytes());
			row_start += row.len();
		}
		for row in rows {
			d.extend_from_slice(row);
		}
		d
	}

	/// The FRAMEPIC BGRA quad for palette index `idx`.
	fn pal(idx: u8) -> &'static [u8] {
		&FRAMEPIC_PALETTE_BGRA[idx as usize * 4..idx as usize * 4 + 4]
	}

	/// A buffer long enough to be a multi-image but whose count/offset pair
	/// doesn't match is "not a multi-image": the lenient decoders answer
	/// `Ok(None)`, the strict indexed ones a header-mismatch error.
	#[test]
	fn header_mismatch_is_none_for_lenient_and_err_for_strict_decoders() {
		let zero_count = vec![0u8; 20]; // image_count = 0
		let mut bad_offset = valid_single_frame_blob(); // 23 bytes, clears the length floor
		bad_offset[2..6].copy_from_slice(&7i32.to_le_bytes()); // first offset must be 6 for count 1
		for d in [&zero_count, &bad_offset] {
			assert!(parse_multi_image(d).unwrap().is_none(), "lenient single-frame decode says None");
			assert!(parse_multi_image_all_frames(d).unwrap().is_none(), "lenient all-frames decode says None");
			let err = decode_multi_image_indexed(d).unwrap_err();
			assert!(err.contains("header mismatch"), "strict indexed decode names the mismatch: {err}");
			let err = decode_multi_image_shadow_indexed(d).unwrap_err();
			assert!(err.contains("header mismatch"), "strict shadow decode names the mismatch: {err}");
		}
	}

	/// A frame whose header claims zero width is rejected by every decoder -
	/// the lenient ones as `Ok(None)`, the strict ones as "no frames decoded".
	#[test]
	fn zero_width_frame_yields_no_frames() {
		let mut d = one_frame_blob(0, 1, &[&[0xff]]);
		while d.len() < 20 {
			d.push(0); // pad past the 20-byte floor so the frame header itself is what fails
		}
		assert!(parse_multi_image(&d).unwrap().is_none(), "BGRA decode rejects a zero-width frame");
		assert!(parse_multi_image_all_frames(&d).unwrap().is_none(), "all-frames decode rejects it too");
		assert!(decode_multi_image_indexed(&d).unwrap_err().contains("no frames"), "indexed decode has no frames");
		assert!(decode_multi_image_shadow_indexed(&d).unwrap_err().contains("no frames"), "shadow decode likewise");
	}

	/// Frame offsets pointing past (or into nonsense inside) the buffer leave
	/// zero decodable frames instead of panicking or over-reading.
	#[test]
	fn frame_offsets_past_the_end_yield_no_frames() {
		let mut d = Vec::new();
		d.extend_from_slice(&4i16.to_le_bytes()); // four frames
		d.extend_from_slice(&18i32.to_le_bytes()); // first offset = 2 + 4*4, right at the buffer's edge
		d.resize(20, 0); // frames 2-4 get offset 0, aliasing the outer header
		assert!(parse_multi_image(&d).unwrap().is_none(), "no room for a frame header at offset 18");
		assert!(parse_multi_image_all_frames(&d).unwrap().is_none(), "no frame in the set decodes");
		assert!(decode_multi_image_indexed(&d).unwrap_err().contains("no frames"), "indexed decode has no frames");
		assert!(decode_multi_image_shadow_indexed(&d).unwrap_err().contains("no frames"), "shadow decode likewise");
	}

	/// A shadow-encoded frame (rows of `(transparent, shadow)` pairs with no
	/// pixel payload) decodes through the shadow path: `parse_multi_image`
	/// tags it `MaxMultiShadow` and the indexed decoder emits marker pixels.
	/// The row exercises a zero-length shadow run, a real run, the 0xff
	/// terminator and an implicit transparent tail.
	#[test]
	fn shadow_frame_decodes_with_marker_pixels() {
		// width 3: skip 1 (shadow run 0), then 1 shadow pixel, end; tail stays transparent.
		let d = one_frame_blob(3, 1, &[&[1, 0, 0, 1, 0xff]]);

		let img = parse_multi_image(&d).unwrap().expect("a valid shadow frame must decode");
		assert_eq!(img.max_type, MaxType::MaxMultiShadow, "shadow RLE wins over the body decoder");
		assert_eq!((img.width, img.height), (3, 1));
		let mut want = Vec::new();
		want.extend_from_slice(pal(0));
		want.extend_from_slice(pal(20)); // SHADOW_COLOR_INDEX
		want.extend_from_slice(pal(0));
		assert_eq!(img.data, want, "one shadow pixel between two transparent ones");

		let frames = decode_multi_image_shadow_indexed(&d).expect("indexed shadow decode succeeds");
		assert_eq!(frames.len(), 1, "single-frame blob");
		assert_eq!(frames[0].pixels, vec![0, SHADOW_MARKER, 0], "marker where the shadow run landed");
		assert_eq!((frames[0].width, frames[0].height), (3, 1));
	}

	/// A row-offset table that disagrees with where the RLE cursor actually
	/// lands rejects the frame in all four decoders (the offsets are the
	/// format's integrity check).
	#[test]
	fn row_offset_mismatch_rejects_the_frame() {
		// Two empty rows (bare 0xff terminators), then the second row's offset is bent.
		let mut d = one_frame_blob(1, 2, &[&[0xff], &[0xff]]);
		d[18..22].copy_from_slice(&99i32.to_le_bytes()); // row 1's table entry
		assert!(parse_multi_image(&d).unwrap().is_none(), "BGRA decode rejects the bent offset");
		assert!(decode_multi_image_indexed(&d).unwrap_err().contains("no frames"), "indexed decode rejects it");
		assert!(decode_multi_image_shadow_indexed(&d).unwrap_err().contains("no frames"), "shadow decode rejects it");
	}

	/// A transparent run longer than the row is malformed - every decoder
	/// refuses the frame rather than writing past the row.
	#[test]
	fn transparent_run_longer_than_the_row_is_rejected() {
		let d = one_frame_blob(2, 1, &[&[5, 0, 0xff]]); // skip 5 in a 2-wide row
		assert!(parse_multi_image(&d).unwrap().is_none(), "BGRA decode rejects the oversized skip");
		assert!(decode_multi_image_indexed(&d).unwrap_err().contains("no frames"), "indexed decode rejects it");
		assert!(decode_multi_image_shadow_indexed(&d).unwrap_err().contains("no frames"), "shadow decode rejects it");
	}

	/// A pixel/shadow run longer than the row's remaining width is likewise
	/// refused by all decoders.
	#[test]
	fn pixel_run_longer_than_the_row_is_rejected() {
		let d = one_frame_blob(2, 1, &[&[0, 3, 0xff]]); // 3 payload pixels in a 2-wide row
		assert!(parse_multi_image(&d).unwrap().is_none(), "BGRA decode rejects the oversized run");
		assert!(decode_multi_image_indexed(&d).unwrap_err().contains("no frames"), "indexed decode rejects it");
		assert!(decode_multi_image_shadow_indexed(&d).unwrap_err().contains("no frames"), "shadow decode rejects it");
	}

	/// A zero-length pixel run is a legal no-op pair, not a terminator: the
	/// body decoders skip it and keep reading the same row.
	#[test]
	fn zero_length_pixel_run_is_skipped_not_terminal() {
		// width 2: (skip 1, 0 pixels) then (skip 0, 1 pixel = 0x2a), end.
		let d = one_frame_blob(2, 1, &[&[1, 0, 0, 1, 0x2a, 0xff]]);

		let img = parse_multi_image(&d).unwrap().expect("a valid body frame must decode");
		assert_eq!(img.max_type, MaxType::MaxMultiImage, "payload bytes disqualify the shadow decoder");
		let mut want = Vec::new();
		want.extend_from_slice(pal(0));
		want.extend_from_slice(pal(0x2a));
		assert_eq!(img.data, want, "the literal pixel lands after the skipped column");

		let frames = decode_multi_image_indexed(&d).expect("indexed decode succeeds");
		assert_eq!(frames[0].pixels, vec![0, 0x2a], "raw palette indices preserved");
	}

	/// Security regression: only frame-offset entry 0 is checked against the
	/// header, so entries 1.. arrive raw. A negative one sign-extends through
	/// `as usize` to ~2^64, and the header readers' `offset + 8` then overflows,
	/// which aborts the process (the release profile sets `overflow-checks` and
	/// `panic = "abort"`) rather than rejecting the frame. A crafted `MAX.RES`
	/// reaches this through the unit-sprite loader, whose `else { continue }`
	/// cannot catch a panic. Every decoder must simply decline the frame.
	#[test]
	fn a_negative_frame_offset_is_declined_not_fatal() {
		let count: i16 = 2;
		let mut d = Vec::new();
		d.extend_from_slice(&count.to_le_bytes());
		d.extend_from_slice(&(2 + 4 * count as i32).to_le_bytes()); // entry 0: valid
		d.extend_from_slice(&(-1i32).to_le_bytes()); // entry 1: unchecked
		d.resize(64, 0);

		assert!(parse_multi_image(&d).unwrap().is_none(), "first-frame decode declines");
		assert!(parse_multi_image_all_frames(&d).unwrap().is_none(), "all-frames decode declines");
		assert!(decode_multi_image_indexed(&d).is_err(), "indexed decode declines");
		assert!(decode_multi_image_shadow_indexed(&d).is_err(), "shadow decode declines");

		// i32::MIN and i32::MAX are the other two ends of the same hazard.
		for bad in [i32::MIN, i32::MAX] {
			let mut d = d.clone();
			d[6..10].copy_from_slice(&bad.to_le_bytes());
			assert!(parse_multi_image_all_frames(&d).unwrap().is_none(), "offset {bad} declines");
			assert!(decode_multi_image_indexed(&d).is_err(), "offset {bad} declines (indexed)");
		}
	}
}
