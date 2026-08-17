use super::multi::IndexedFrame;
use super::palette::FRAMEPIC_PALETTE_BGRA;
use super::types::{ImageData, MAX_IMAGE_HEIGHT, MAX_IMAGE_WIDTH, MaxType};
use crate::color::{indexed_to_bgra_with_transparency, indexed_to_color};

/// How a simple-image's transparency should be resolved at decode time.
/// MAX has multiple rendering paths with different transparency rules; the
/// caller picks the right one per tag (see `apps/game/assets/config/res.ini`'s
/// `[simple_image]` section, sourced from `sandbox/resources/analysis.xlsx`).
///
/// - `Index(N)` - discard pixels whose palette index equals `N`. Most UI
///   buttons render through `Button` → `trans_buf_to_buf` (grbuf.c:243),
///   which globally treats palette index 0 as the discard key. A handful
///   of direct-blit sprites (ZOOMPTR thumb, ARROW_*, FUEL/GOLD/RAW marks,
///   …) use a different index; the analysis spreadsheet records each.
/// - `Opaque` - paint every pixel solid. Mirrors MAX's
///   `WindowManager_LoadSimpleImage(.., has_transparency=false)` path used
///   for backgrounds whose top-left pixel happens to be a real foreground
///   color (`ZOOMPNL1` slider track historically, instrument panels via
///   `buf_to_buf` in `GameManager_DrawDisplayPanel`).
/// - `FromHeader` - fall back to MAX's C-struct convention: the byte at
///   offset 8 (= `pixel[0,0]` = `image->transparent_color`) is the discard
///   key. Useful for callers that don't have manifest metadata yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimpleImageTransparency {
	Index(u8),
	Opaque,
	FromHeader,
}

/// Decodes a simple image into BGRA bytes. Transparency policy is supplied
/// by the caller via `mode`; see [`SimpleImageTransparency`] for the
/// available choices.
pub fn parse_simple_image_with(data: &[u8], mode: SimpleImageTransparency) -> Option<ImageData> {
	let (w, h, hot_x, hot_y, indexed, header_trans) = parse_simple_image_raw(data)?;
	let bgra = match mode {
		SimpleImageTransparency::Opaque => indexed_to_color(indexed, &FRAMEPIC_PALETTE_BGRA),
		SimpleImageTransparency::Index(idx) => indexed_to_bgra_with_transparency(indexed, &FRAMEPIC_PALETTE_BGRA, idx),
		SimpleImageTransparency::FromHeader => {
			indexed_to_bgra_with_transparency(indexed, &FRAMEPIC_PALETTE_BGRA, header_trans)
		}
	};
	Some(ImageData {
		max_type: MaxType::MaxSimpleImage,
		width: w as u32,
		height: h as u32,
		hot_spot_x: hot_x,
		hot_spot_y: hot_y,
		data: bgra,
	})
}

/// Legacy entry point - uses the C-struct convention (`pixel[0,0]` is the
/// discard key). Prefer `parse_simple_image_with` once you have manifest
/// metadata for the tag; this exists for callers that don't yet plumb it
/// through (asset extractor tooling, sandbox tests).
pub fn parse_simple_image(data: &[u8]) -> Option<ImageData> {
	parse_simple_image_with(data, SimpleImageTransparency::FromHeader)
}

/// Opaque-only convenience wrapper. Equivalent to
/// `parse_simple_image_with(data, SimpleImageTransparency::Opaque)`.
pub fn parse_simple_image_opaque(data: &[u8]) -> Option<ImageData> {
	parse_simple_image_with(data, SimpleImageTransparency::Opaque)
}

/// Indexed variant - keeps the palette indices intact so the frame can be
/// uploaded into the R8Uint sprite atlas and recolored at sample time via
/// the shared palette LUT.
///
/// The shared sprite shader treats `idx == 0` as the discard sentinel
/// (a single global rule, not per-instance). MAX's transparent index is
/// per-image and varies (ZOOMPTR=16, ARROW_*=1, FUEL/GOLD marks=255, …),
/// so we **swap** the chosen transparent index with 0:
///
///   - pixels matching `transparent_index` → 0 (shader discards)
///   - pixels originally at index 0        → `transparent_index`
///     (still visible - colored as palette[transparent_index])
///   - all others pass through
///
/// `Opaque` skips the swap entirely (no pixels become transparent at the
/// shader level - useful for full-bleed backgrounds). `FromHeader` keeps
/// the legacy "pixel[0,0] is the key" behavior.
pub fn parse_simple_image_indexed_with(data: &[u8], mode: SimpleImageTransparency) -> Option<IndexedFrame> {
	let (w, h, hot_x, hot_y, indexed, header_trans) = parse_simple_image_raw(data)?;
	let pixels = match mode {
		SimpleImageTransparency::Opaque => indexed.to_vec(),
		SimpleImageTransparency::Index(idx) => swap_with_zero(indexed, idx),
		SimpleImageTransparency::FromHeader => swap_with_zero(indexed, header_trans),
	};
	Some(IndexedFrame { width: w as u32, height: h as u32, hot_spot_x: hot_x, hot_spot_y: hot_y, pixels })
}

/// Legacy indexed entry point - `FromHeader` convention. Same backward-
/// compatibility caveat as `parse_simple_image`.
pub fn parse_simple_image_indexed(data: &[u8]) -> Option<IndexedFrame> {
	parse_simple_image_indexed_with(data, SimpleImageTransparency::FromHeader)
}

/// Pixels matching `trans_idx` get value 0; pixels originally at 0 get
/// `trans_idx` (preserves visibility in case the artwork uses index 0
/// for actual content). Identity transform when `trans_idx == 0`.
fn swap_with_zero(indexed: &[u8], trans_idx: u8) -> Vec<u8> {
	if trans_idx == 0 {
		indexed.to_vec()
	} else {
		indexed
			.iter()
			.map(|&p| {
				if p == trans_idx {
					0
				} else if p == 0 {
					trans_idx
				} else {
					p
				}
			})
			.collect()
	}
}

/// Decoded fields of a simple image: width, height, hot-spot x/y, the pixel
/// slice, and the header transparent index (`pixels[0]`).
type SimpleImageRaw<'a> = (i16, i16, i32, i32, &'a [u8], u8);

/// Returns `(width, height, hot_spot_x, hot_spot_y, &pixels, header_trans)`
/// for a valid simple image, or `None` if the header is malformed or the
/// payload size doesn't match. `header_trans` is `pixels[0]` -
/// MAX's `ImageSimpleHeader::transparent_color` aliases the first raster
/// byte. Most UI tags don't actually use this value (it's a packing
/// artifact); res.ini's per-tag override carries the right discard index
/// for those.
fn parse_simple_image_raw(data: &[u8]) -> Option<SimpleImageRaw<'_>> {
	if data.len() < 9 {
		return None;
	}

	let width = i16::from_le_bytes(data[0..2].try_into().ok()?);
	let height = i16::from_le_bytes(data[2..4].try_into().ok()?);
	let hot_spot_x = i16::from_le_bytes(data[4..6].try_into().ok()?) as i32;
	let hot_spot_y = i16::from_le_bytes(data[6..8].try_into().ok()?) as i32;

	if width <= 0
		|| height <= 0
		|| width > MAX_IMAGE_WIDTH
		|| height > MAX_IMAGE_HEIGHT
		|| data.len() - 8 != width as usize * height as usize
	{
		return None;
	}

	let pixels = &data[8..];
	let header_trans = pixels[0];

	Some((width, height, hot_spot_x, hot_spot_y, pixels, header_trans))
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The packed on-disk simple-image layout: 8-byte header + raw pixels.
	fn blob(width: i16, height: i16, hot_x: i16, hot_y: i16, pixels: &[u8]) -> Vec<u8> {
		let mut d = Vec::new();
		d.extend_from_slice(&width.to_le_bytes());
		d.extend_from_slice(&height.to_le_bytes());
		d.extend_from_slice(&hot_x.to_le_bytes());
		d.extend_from_slice(&hot_y.to_le_bytes());
		d.extend_from_slice(pixels);
		d
	}

	/// The FRAMEPIC BGRA quad for palette index `idx` (all entries are opaque,
	/// so this doubles as the expected output of the transparency decoders).
	fn pal(idx: u8) -> &'static [u8] {
		&FRAMEPIC_PALETTE_BGRA[idx as usize * 4..idx as usize * 4 + 4]
	}

	/// `FromHeader` mode discards pixels matching `pixel[0,0]` (here 5) and
	/// carries the header fields - including negative hot-spots - through.
	#[test]
	fn from_header_uses_the_first_pixel_as_the_discard_key() {
		let img = parse_simple_image(&blob(2, 2, -3, -7, &[5, 0, 7, 5])).expect("a valid 2x2 image must decode");
		assert_eq!(img.max_type, MaxType::MaxSimpleImage);
		assert_eq!((img.width, img.height), (2, 2), "dimensions come from the header");
		assert_eq!((img.hot_spot_x, img.hot_spot_y), (-3, -7), "hot-spots are sign-extended i16s");
		let mut want = Vec::new();
		want.extend_from_slice(&[0, 0, 0, 0]); // index 5 == header key -> discarded
		want.extend_from_slice(pal(0));
		want.extend_from_slice(pal(7));
		want.extend_from_slice(&[0, 0, 0, 0]);
		assert_eq!(img.data, want, "only pixels matching the header key become transparent");
	}

	/// `Index(N)` overrides the header key: index 7 is discarded while the
	/// header's own key (5) stays visible.
	#[test]
	fn explicit_index_mode_overrides_the_header_key() {
		let img = parse_simple_image_with(&blob(2, 2, 0, 0, &[5, 0, 7, 5]), SimpleImageTransparency::Index(7))
			.expect("a valid 2x2 image must decode");
		let mut want = Vec::new();
		want.extend_from_slice(pal(5));
		want.extend_from_slice(pal(0));
		want.extend_from_slice(&[0, 0, 0, 0]); // index 7 == chosen key -> discarded
		want.extend_from_slice(pal(5));
		assert_eq!(img.data, want, "only the caller-chosen index becomes transparent");
	}

	/// `Opaque` paints every pixel solid, ignoring the header key entirely.
	#[test]
	fn opaque_mode_keeps_every_pixel_solid() {
		let img = parse_simple_image_opaque(&blob(2, 2, 0, 0, &[5, 0, 7, 5])).expect("a valid 2x2 image must decode");
		let mut want = Vec::new();
		want.extend_from_slice(pal(5));
		want.extend_from_slice(pal(0));
		want.extend_from_slice(pal(7));
		want.extend_from_slice(pal(5));
		assert_eq!(img.data, want, "no pixel is discarded in opaque mode");
	}

	/// The indexed decoder swaps the transparent index with 0 so the shader's
	/// global "0 discards" rule applies: 5 -> 0, 0 -> 5, others pass through.
	#[test]
	fn indexed_from_header_swaps_the_transparent_index_with_zero() {
		let f = parse_simple_image_indexed(&blob(2, 2, -3, -7, &[5, 0, 7, 5])).expect("a valid 2x2 image must decode");
		assert_eq!((f.width, f.height), (2, 2));
		assert_eq!((f.hot_spot_x, f.hot_spot_y), (-3, -7), "hot-spots survive the indexed path too");
		assert_eq!(f.pixels, vec![0, 5, 7, 0], "5<->0 swapped, 7 untouched");
	}

	/// `Index(0)` is the identity transform - the swap would be a no-op, so
	/// the pixels come back verbatim.
	#[test]
	fn indexed_index_zero_is_the_identity_transform() {
		let f = parse_simple_image_indexed_with(&blob(2, 2, 0, 0, &[5, 0, 7, 5]), SimpleImageTransparency::Index(0))
			.expect("a valid 2x2 image must decode");
		assert_eq!(f.pixels, vec![5, 0, 7, 5], "transparent index 0 needs no swap");
	}

	/// Indexed `Opaque` skips the swap entirely, keeping raw indices - even
	/// zeros - intact for full-bleed backgrounds.
	#[test]
	fn indexed_opaque_keeps_raw_indices() {
		let f = parse_simple_image_indexed_with(&blob(2, 2, 0, 0, &[5, 0, 7, 5]), SimpleImageTransparency::Opaque)
			.expect("a valid 2x2 image must decode");
		assert_eq!(f.pixels, vec![5, 0, 7, 5], "opaque mode must not remap any index");
	}

	/// Every malformed-header shape is rejected by both the BGRA and the
	/// indexed entry points: short input, non-positive or oversized
	/// dimensions, and a payload that disagrees with width * height.
	#[test]
	fn malformed_headers_are_rejected() {
		let cases: Vec<(&str, Vec<u8>)> = vec![
			("too short for the header + key pixel", blob(1, 1, 0, 0, &[])),
			("zero width", blob(0, 1, 0, 0, &[9])),
			("zero height", blob(1, 0, 0, 0, &[9])),
			("negative width", blob(-1, 1, 0, 0, &[9])),
			("width over the cap", blob(MAX_IMAGE_WIDTH + 1, 1, 0, 0, &vec![9; MAX_IMAGE_WIDTH as usize + 1])),
			("height over the cap", blob(1, MAX_IMAGE_HEIGHT + 1, 0, 0, &vec![9; MAX_IMAGE_HEIGHT as usize + 1])),
			("payload shorter than width * height", blob(2, 2, 0, 0, &[9, 9, 9])),
			("payload longer than width * height", blob(2, 2, 0, 0, &[9, 9, 9, 9, 9])),
		];
		for (why, data) in &cases {
			assert!(parse_simple_image(data).is_none(), "BGRA decode must reject: {why}");
			assert!(parse_simple_image_indexed(data).is_none(), "indexed decode must reject: {why}");
		}
	}
}
