//! MAX asset decoders.
//!
//! Pure decoders for M.A.X. file formats - RES archives, WRL maps, indexed
//! images (simple / big / multi), and base-unit-data files. No game logic,
//! no rendering - just bytes in, typed values out.
//!
//! Kept deliberately free of `wgpu` / `winit` dependencies so the binary
//! asset extractor and headless tests can link against it cheaply.

pub mod color;
pub mod image;
pub mod res;
pub mod units;
pub mod wrl;

pub use color::{indexed_to_color, rgb_to_bgra};
