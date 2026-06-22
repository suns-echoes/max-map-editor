//! UI skin assets: decode the brushed-steel sheet (`resources/images/steel.png`)
//! that every chrome element samples (see [`theme`](crate::theme) for the
//! tints + bevels layered over it). Pure CPU decode - the GPU upload lives in
//! [`TextPass`](crate::text::TextPass), beside the font atlases.
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
