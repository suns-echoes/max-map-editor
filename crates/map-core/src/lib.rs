//! Map project: the editable in-memory document. A `.json` project loads
//! directly; a `.WRL` is imported via [`Project::from_wrl`] (a synthetic
//! in-memory pack). Pure logic - no GPU, no windowing, fully headless-
//! testable. Every edit goes through an inverse-storing patch so undo/redo
//! falls out of the design.

mod bake;
mod color;
mod game_palette;
mod grid;
mod image_import;
mod pack;
mod palette;
mod palette_convert;
mod project;
mod selection;
mod shore;
mod template;
mod worldgen;
mod wrl_import;

pub use bake::{MAX_BAKED_TILES, WRL_HEADER, bake};
pub use color::{hsl_to_rgb, rgb_to_hsl};
pub use game_palette::{GAME_PALETTE, apply_game_statics};
pub use image_import::{ConvertOpts, ConvertSession, Coverage, Dedupe, image_to_wrl};
pub use pack::{FamilyProps, MatchRule, TileKind, TilePack, TilePattern, Transformable, family_of};
pub use palette::{parse_palette, set_slot_rgb, slot_rgb, write_palette};
pub use palette_convert::{ConvertOptions, ConvertReport};
pub use project::{
	ANIMATED_SLOTS, DYNAMIC_SLOTS, LAYER_GROUND, LAYER_WATER, MAX_LAYERS, PaletteReimport, Project, Rng, TileRef,
	Transform, UnitNote, UseEntry, WATER_CYCLES, WATER_SLOTS, transform_tile,
};
pub use selection::{Edge, SelectMode, Selection};
pub use shore::{FixSession, FixStrength};
pub use template::{StampOp, Template, clear_selection, clear_selection_layer};
pub use worldgen::{GenParams, GenPattern, GenSession, GenStats};
pub use wrl_import::{ExtrasDest, UnmappedTile, WrlImport};
