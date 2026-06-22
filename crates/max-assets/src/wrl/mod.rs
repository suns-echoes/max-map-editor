//! MAX WRL map file format.
//!
//! A WRL is a tile-indexed map plus tileset:
//! - 5-byte header, then `(u16 width, u16 height)` tile dimensions.
//! - `width * height` bytes minimap (one byte per tile slot).
//! - `width * height * u16` bigmap - each entry is an index into the tile table.
//! - `u16 tile_count`, then `tile_count * 64 * 64` bytes of raw tile rasters.
//! - 256 * 3 byte palette.
//! - `tile_count` byte pass-table (terrain passability per tile).
//!
//! Dimensions are in *tile cells* (64×64 pixels each). A standard MAX map is
//! up to 112×112 tiles; the engine here targets 1024×1024 cells.

mod consts;
mod file;
mod types;
mod write;

pub use consts::{INSTALLED_MAP_FILE_NAMES, TILE_DATA_SIZE, TILE_SIZE};
pub use file::{WrlError, read_wrl_file, read_wrl_header};
pub use types::{WrlFile, WrlHeader};
pub use write::{write_wrl_file, wrl_to_bytes};
