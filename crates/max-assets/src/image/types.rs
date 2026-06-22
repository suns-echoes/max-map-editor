/// Tag for the source format of a decoded image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaxType {
	MaxSimpleImage,
	MaxBigImage,
	MaxMultiImage,
	MaxMultiShadow,
}

pub const MAX_IMAGE_WIDTH: i16 = 640;
pub const MAX_IMAGE_HEIGHT: i16 = 480;

/// BGRA8 raster plus provenance metadata.
#[derive(Debug, Clone)]
pub struct ImageData {
	pub max_type: MaxType,
	pub width: u32,
	pub height: u32,
	/// Hot-spot (anchor) offset. Signed because MAX sprites freely place the
	/// anchor outside the sprite rectangle - in particular above or left of
	/// the top-left corner (e.g. AWAC's overhead radar dish).
	pub hot_spot_x: i32,
	pub hot_spot_y: i32,
	pub data: Vec<u8>,
}
