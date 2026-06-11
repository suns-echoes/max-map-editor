//! Image decoders for M.A.X. sprite formats.
//!
//! MAX ships three flavors:
//! - **Simple image**: fixed 8-byte header + palette-indexed raster (UI framebits).
//! - **Big image**: embeds its own 256-entry palette, RLE-compressed (portraits, intro art).
//! - **Multi-image / shadow**: N frames, each row transparent-run encoded (units, buildings).
//!
//! All decoders return `ImageData` in BGRA8 so the renderer can consume them directly.

mod big;
mod multi;
mod palette;
mod simple;
mod types;

pub use big::{image_rle_decode, parse_big_image, parse_big_image_indexed};
pub use multi::{
	IndexedFrame, decode_multi_image_indexed, decode_multi_image_shadow_indexed, parse_multi_image,
	parse_multi_image_all_frames,
};
pub use palette::FRAMEPIC_PALETTE_BGRA;
pub use simple::{
	SimpleImageTransparency, parse_simple_image, parse_simple_image_indexed, parse_simple_image_indexed_with,
	parse_simple_image_opaque, parse_simple_image_with,
};
pub use types::{ImageData, MAX_IMAGE_HEIGHT, MAX_IMAGE_WIDTH, MaxType};
