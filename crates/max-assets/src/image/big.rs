use super::multi::IndexedFrame;
use super::palette::FRAMEPIC_PALETTE_BGRA;
use super::types::{ImageData, MAX_IMAGE_HEIGHT, MAX_IMAGE_WIDTH, MaxType};
use crate::color::rgb_to_bgra;

/// Decodes a "big image" - header + embedded 256-entry palette + RLE-compressed raster.
pub fn parse_big_image(data: &[u8]) -> Option<ImageData> {
	if data.len() < 8 {
		return None;
	}

	let hot_spot_x = i16::from_le_bytes(data[0..2].try_into().ok()?) as i32;
	let hot_spot_y = i16::from_le_bytes(data[2..4].try_into().ok()?) as i32;
	let width = i16::from_le_bytes(data[4..6].try_into().ok()?);
	let height = i16::from_le_bytes(data[6..8].try_into().ok()?);

	let palette_size = 256 * 3;

	if width <= 0 || height <= 0 || width > MAX_IMAGE_WIDTH || height > MAX_IMAGE_HEIGHT {
		return None;
	}

	let palette = rgb_to_bgra(&mut data.get(8..8 + palette_size)?.to_owned());

	let indexed_image_data = image_rle_decode(&data[8 + palette_size..])
		.map_err(|e| {
			log::error!("Failed to decode RLE data: {}", e);
		})
		.ok()?;

	let mut image_data = vec![0; (width as i32 * height as i32 * 4) as usize];
	let mut image_data_index = 0;
	for &palette_color_index in indexed_image_data.iter() {
		let palette_slice = &palette[palette_color_index as usize * 4..palette_color_index as usize * 4 + 4];
		image_data[image_data_index..image_data_index + 4].copy_from_slice(palette_slice);
		image_data_index += 4;
	}

	Some(ImageData {
		max_type: MaxType::MaxBigImage,
		width: width as u32,
		height: height as u32,
		hot_spot_x,
		hot_spot_y,
		data: image_data,
	})
}

/// Decodes a big image into an `IndexedFrame` referenced against the
/// canonical FRAMEPIC palette. The big-image format embeds its own
/// 256-entry palette (portraits are authored for that palette), so we
/// remap each pixel to the nearest FRAMEPIC index via squared RGB
/// distance. Keeps the portrait in the single-atlas pipeline without
/// needing per-portrait palette storage or a dedicated shader.
pub fn parse_big_image_indexed(data: &[u8]) -> Option<IndexedFrame> {
	if data.len() < 8 {
		return None;
	}

	let hot_spot_x = i16::from_le_bytes(data[0..2].try_into().ok()?) as i32;
	let hot_spot_y = i16::from_le_bytes(data[2..4].try_into().ok()?) as i32;
	let width = i16::from_le_bytes(data[4..6].try_into().ok()?);
	let height = i16::from_le_bytes(data[6..8].try_into().ok()?);

	if width <= 0 || height <= 0 || width > MAX_IMAGE_WIDTH || height > MAX_IMAGE_HEIGHT {
		return None;
	}

	let palette_size = 256 * 3;
	if data.len() < 8 + palette_size {
		return None;
	}
	// Embedded palette stored as RGB triples - clone into a local buffer
	// so we can compute remaps without mutating the caller's data.
	let embedded_rgb = &data[8..8 + palette_size];

	let indices = image_rle_decode(&data[8 + palette_size..]).ok()?;
	let expected = (width as usize) * (height as usize);
	if indices.len() < expected {
		return None;
	}

	// Build a 256-entry LUT: embedded-index → nearest FRAMEPIC index. Done
	// once per portrait (the portrait's pixels usually touch every palette
	// slot anyway, so eagerly filling the LUT isn't wasteful).
	let mut remap = [0u8; 256];
	for i in 0..256 {
		let r = embedded_rgb[i * 3];
		let g = embedded_rgb[i * 3 + 1];
		let b = embedded_rgb[i * 3 + 2];
		remap[i] = nearest_framepic_index(r, g, b);
	}

	let mut pixels = Vec::with_capacity(expected);
	for &idx in indices.iter().take(expected) {
		pixels.push(remap[idx as usize]);
	}

	Some(IndexedFrame { width: width as u32, height: height as u32, hot_spot_x, hot_spot_y, pixels })
}

/// Squared RGB distance over all 256 FRAMEPIC slots - linear but only
/// runs 256×256 times total for a portrait load (once per LUT entry),
/// so it's not worth a kd-tree or a perceptual color space.
fn nearest_framepic_index(r: u8, g: u8, b: u8) -> u8 {
	let (mut best_idx, mut best_dist) = (0u8, u32::MAX);
	for i in 0..256usize {
		// FRAMEPIC is stored BGRA, so byte order is B, G, R, A.
		let fb = FRAMEPIC_PALETTE_BGRA[i * 4] as i32;
		let fg = FRAMEPIC_PALETTE_BGRA[i * 4 + 1] as i32;
		let fr = FRAMEPIC_PALETTE_BGRA[i * 4 + 2] as i32;
		let dr = fr - r as i32;
		let dg = fg - g as i32;
		let db = fb - b as i32;
		let dist = (dr * dr + dg * dg + db * db) as u32;
		if dist < best_dist {
			best_dist = dist;
			best_idx = i as u8;
		}
	}
	best_idx
}

/// RLE decode: a signed `i16` count - positive = copy N literal bytes,
/// negative = repeat the next byte N times.
pub fn image_rle_decode(data: &[u8]) -> Result<Vec<u8>, String> {
	if data.is_empty() {
		return Ok(Vec::new());
	}

	let mut decoded_data = Vec::new();
	let mut i = 0;

	while i < data.len() {
		let option: i16 = i16::from_le_bytes(
			data.get(i..i + 2).ok_or("truncated RLE opcode")?.try_into().map_err(|_| "Invalid RLE data")?,
		);
		i += 2;

		if option > 0 {
			let count = option as usize;
			decoded_data.extend_from_slice(data.get(i..i + count).ok_or("truncated RLE literal run")?);
			i += count;
		} else {
			let count = (-(option as i32)) as usize;
			let value = *data.get(i).ok_or("truncated RLE repeat run")?;
			i += 1;
			decoded_data.extend(std::iter::repeat_n(value, count));
		}
	}

	Ok(decoded_data)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rle_truncated_opcode_is_err() {
		assert!(image_rle_decode(&[0x05]).is_err()); // 1 byte: no full opcode
	}

	#[test]
	fn rle_truncated_literal_run_is_err() {
		// opcode +4 (copy 4) but only one payload byte follows
		let mut d = 4i16.to_le_bytes().to_vec();
		d.push(0xaa);
		assert!(image_rle_decode(&d).is_err());
	}

	#[test]
	fn rle_truncated_repeat_run_is_err() {
		// opcode -3 (repeat) with no value byte
		let d = (-3i16).to_le_bytes().to_vec();
		assert!(image_rle_decode(&d).is_err());
	}

	#[test]
	fn rle_min_count_does_not_overflow() {
		// i16::MIN would overflow `-option` computed as i16; ensure it's handled
		let mut d = i16::MIN.to_le_bytes().to_vec();
		d.push(0xcd); // value byte
		assert_eq!(image_rle_decode(&d).unwrap().len(), 32768);
	}

	#[test]
	fn rle_roundtrip_literal_then_repeat() {
		let mut d = Vec::new();
		d.extend_from_slice(&2i16.to_le_bytes()); // copy 2
		d.extend_from_slice(&[1, 2]);
		d.extend_from_slice(&(-3i16).to_le_bytes()); // repeat 3
		d.push(9);
		assert_eq!(image_rle_decode(&d).unwrap(), vec![1, 2, 9, 9, 9]);
	}

	fn big_blob() -> Vec<u8> {
		let mut d = Vec::new();
		d.extend_from_slice(&0i16.to_le_bytes()); // hot_spot_x
		d.extend_from_slice(&0i16.to_le_bytes()); // hot_spot_y
		d.extend_from_slice(&2i16.to_le_bytes()); // width
		d.extend_from_slice(&2i16.to_le_bytes()); // height
		d.extend_from_slice(&[1u8; 256 * 3]); // embedded palette
		d.extend_from_slice(&4i16.to_le_bytes()); // RLE: copy 4 indices
		d.extend_from_slice(&[0, 1, 2, 3]);
		d
	}

	#[test]
	fn big_image_decodes_valid_blob() {
		assert!(parse_big_image(&big_blob()).is_some());
	}

	#[test]
	fn big_image_truncated_palette_is_none() {
		let d = big_blob();
		assert!(parse_big_image(&d[..100]).is_none()); // header ok, palette cut short
	}

	#[test]
	fn big_image_truncating_never_panics() {
		let d = big_blob();
		for len in 0..=d.len() {
			let _ = parse_big_image(&d[..len]);
			let _ = parse_big_image_indexed(&d[..len]);
		}
	}
}
