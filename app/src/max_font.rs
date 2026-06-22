//! The MAX UI **label** charset: printable ASCII, `0x20..=0x7e`. A glyph's
//! index is `code - FIRST`. The glyphs themselves are rasterized from the
//! TrueType outlines of `assets/max_square.ttf` at runtime (see [`crate::font`]
//! / [`crate::ttf`]) - this module is just the character range they cover.

/// First atlas character (space); a glyph's index is `code - FIRST`.
pub const FIRST: u8 = 32;
/// Number of glyphs (printable ASCII, `0x20..=0x7e`).
pub const COUNT: u32 = 95;
