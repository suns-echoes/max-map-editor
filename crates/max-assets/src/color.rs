/// Converts packed RGB triplets to BGRA with opaque alpha. Used to promote
/// the 256-entry palettes embedded in WRL and big-image files into a
/// renderer-friendly format.
pub fn rgb_to_bgra(rgb_pixels: &mut [u8]) -> Vec<u8> {
	let mut bgra_pixels = Vec::with_capacity(rgb_pixels.len() / 3 * 4);
	for chunk in rgb_pixels.chunks_exact(3) {
		bgra_pixels.push(chunk[2]);
		bgra_pixels.push(chunk[1]);
		bgra_pixels.push(chunk[0]);
		bgra_pixels.push(255);
	}
	bgra_pixels
}

/// Expands indexed pixels to 32-bit BGRA using a 4-bytes-per-entry palette.
pub fn indexed_to_color(indexed_pixels: &[u8], palette: &[u8]) -> Vec<u8> {
	let mut color_pixels = Vec::with_capacity(indexed_pixels.len() * 4);
	for &index in indexed_pixels {
		let palette_slice = &palette[index as usize * 4..index as usize * 4 + 4];
		color_pixels.extend_from_slice(palette_slice);
	}
	color_pixels
}

/// Expands indexed pixels to BGRA, honoring a per-image transparent palette
/// index (the magic pixel from MAX's `ImageSimpleHeader::transparent_color`).
/// Pixels matching `transparent_index` come out with alpha=0 — every other
/// pixel is opaque. The portrait/UI atlas's fragment shader then discards
/// the transparent pixels via its `c.a <= 0.01` check.
///
/// MAX's simple-image format stores the transparent index as the first byte
/// of pixel data (which doubles as `pixel[0]`); the chosen index is whatever
/// the artist picked when they exported the image, and in practice varies
/// from sprite to sprite (background magenta, palette index 0, etc.). Always
/// honoring it is the correct port of `WindowManager_DecodeSimpleImage`'s
/// `has_transparency` path (window_manager.cpp:799).
pub fn indexed_to_bgra_with_transparency(indexed_pixels: &[u8], palette: &[u8], transparent_index: u8) -> Vec<u8> {
	let mut color_pixels = Vec::with_capacity(indexed_pixels.len() * 4);
	for &index in indexed_pixels {
		let p = index as usize * 4;
		if index == transparent_index {
			// Zero RGB too so any sampler that doesn't honor alpha gets
			// black instead of leaking the magic-pixel color.
			color_pixels.extend_from_slice(&[0, 0, 0, 0]);
		} else {
			color_pixels.push(palette[p]);
			color_pixels.push(palette[p + 1]);
			color_pixels.push(palette[p + 2]);
			color_pixels.push(255);
		}
	}
	color_pixels
}
