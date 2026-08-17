//! UI skin assets: decode the brushed-steel sheet (`resources/images/steel.png`)
//! that every chrome element samples (see [`theme`](crate::theme) for the
//! tints + bevels layered over it). Pure CPU decode - the GPU upload lives in
//! [`MenuChrome`](crate::uikit_menu::MenuChrome), beside the one `Fonts`.
//!
//! Loading is best-effort: a missing/garbled PNG falls back to a flat neutral
//! tile so headless renders, CI, and the screenshot path never panic over a
//! cosmetic asset.

use std::path::Path;

/// A decoded RGBA8 image: tightly packed `w*h*4` bytes.
pub struct Image {
	pub rgba: Vec<u8>,
	pub size: (u32, u32),
}

impl Image {
	/// A 2×2 mid-gray fallback - keeps the steel pass valid when the sheet is
	/// absent (the tints/bevels still read as flat gunmetal panels).
	fn flat() -> Self {
		Self { rgba: [128u8, 130, 134, 255].repeat(4), size: (2, 2) }
	}
}

/// Decode `resources/images/steel.png` to RGBA8, or fall back to flat gray.
pub fn load_steel(resources_dir: &Path) -> Image {
	decode(&resources_dir.join("images/steel.png")).unwrap_or_else(Image::flat)
}

/// Decode an 8-bit PNG (RGB or RGBA) to packed RGBA8; `None` on any error or
/// an unsupported color type.
fn decode(path: &Path) -> Option<Image> {
	let file = std::fs::File::open(path).ok()?;
	let mut reader = png::Decoder::new(std::io::BufReader::new(file)).read_info().ok()?;
	let mut buf = vec![0; reader.output_buffer_size()?];
	let info = reader.next_frame(&mut buf).ok()?;
	if info.bit_depth != png::BitDepth::Eight {
		return None;
	}
	let src = &buf[..info.buffer_size()];
	let rgba = match info.color_type {
		png::ColorType::Rgba => src.to_vec(),
		png::ColorType::Rgb => {
			let mut out = Vec::with_capacity(src.len() / 3 * 4);
			for px in src.chunks_exact(3) {
				out.extend_from_slice(&[px[0], px[1], px[2], 255]);
			}
			out
		}
		_ => return None,
	};
	Some(Image { rgba, size: (info.width, info.height) })
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Write a minimal PNG of the given colour type / bit depth (the decode
	/// fixtures - `data` is the raw image data for that encoding).
	fn write_test_png(path: &Path, color: png::ColorType, depth: png::BitDepth, size: (u32, u32), data: &[u8]) {
		let file = std::fs::File::create(path).expect("create test png");
		let mut enc = png::Encoder::new(std::io::BufWriter::new(file), size.0, size.1);
		enc.set_color(color);
		enc.set_depth(depth);
		enc.write_header().expect("png header").write_image_data(data).expect("png data");
	}

	/// A missing steel sheet falls back to the flat 2×2 neutral tile, so
	/// headless renders / CI / screenshots never panic over a cosmetic asset.
	#[test]
	fn load_steel_falls_back_to_flat_gray() {
		let img = load_steel(Path::new("/nonexistent/resources"));
		assert_eq!(img.size, (2, 2), "the fallback is the 2x2 tile");
		assert_eq!(img.rgba, [128u8, 130, 134, 255].repeat(4), "mid-gray, opaque");
	}

	/// `decode` passes 8-bit RGBA through byte-exact, and rejects (`None`) the
	/// encodings the steel pass can't take: 16-bit depth and grayscale.
	#[test]
	fn decode_accepts_rgba8_and_rejects_unsupported() {
		let dir = std::env::temp_dir().join(format!("max-map-editor-skin-{}", std::process::id()));
		std::fs::create_dir_all(&dir).expect("temp dir");

		let rgba8 = dir.join("rgba8.png");
		let pixels = [1u8, 2, 3, 4, 5, 6, 7, 8];
		write_test_png(&rgba8, png::ColorType::Rgba, png::BitDepth::Eight, (2, 1), &pixels);
		let img = decode(&rgba8).expect("8-bit RGBA decodes");
		assert_eq!(img.size, (2, 1));
		assert_eq!(img.rgba, pixels, "RGBA passes through byte-exact");

		let deep = dir.join("rgb16.png");
		write_test_png(&deep, png::ColorType::Rgb, png::BitDepth::Sixteen, (1, 1), &[0u8; 6]);
		assert!(decode(&deep).is_none(), "16-bit depth is rejected");

		let gray = dir.join("gray8.png");
		write_test_png(&gray, png::ColorType::Grayscale, png::BitDepth::Eight, (2, 2), &[9u8; 4]);
		assert!(decode(&gray).is_none(), "grayscale colour type is rejected");

		let _ = std::fs::remove_dir_all(&dir);
	}
}
