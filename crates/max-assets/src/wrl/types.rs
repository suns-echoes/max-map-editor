/// Lightweight header view - enough to render a minimap without decoding tiles.
#[derive(Debug, Clone)]
pub struct WrlHeader {
	pub width: u16,
	pub height: u16,
	pub tile_count: u16,
	pub minimap: Vec<u8>,
	pub palette: Vec<u8>,
}

/// Fully-decoded WRL file.
#[derive(Debug, Clone)]
pub struct WrlFile {
	pub header: Vec<u8>, // 5 bytes
	pub width: u16,
	pub height: u16,
	pub minimap: Vec<u8>, // width * height bytes
	pub bigmap: Vec<u16>, // width * height u16s
	pub tile_count: u16,
	pub tiles: Vec<u8>,      // tile_count * 64 * 64 bytes
	pub palette: Vec<u8>,    // 256 * 3 bytes
	pub pass_table: Vec<u8>, // tile_count bytes
}
