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
	if data.len() < offset + 8 {
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
		let start_offset = offset + 8 + i as usize * 4;
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
	if data.len() < offset + 8 {
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
		let s = offset + 8 + i as usize * 4;
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
}
