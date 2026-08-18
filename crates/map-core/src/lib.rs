//! Map project: the editable in-memory document. A `.json` project loads
//! directly; a `.WRL` is imported via [`Project::from_wrl`] (a synthetic
//! in-memory pack). Pure logic - no GPU, no windowing, fully headless-
//! testable. Every edit goes through an inverse-storing patch so undo/redo
//! falls out of the design.

mod bake;
mod color;
mod combos;
mod deanimate;
mod game_palette;
mod grid;
mod image_import;
mod pack;
mod palette;
mod palette_convert;
mod project;
mod scenery;
mod selection;
mod shore;
mod template;
mod worldgen;
mod wrl_import;

pub use bake::{MAX_BAKED_TILES, WRL_HEADER, bake};
pub use color::{hsl_to_rgb, rgb_to_hsl};
pub use combos::match_combos_map;
pub use deanimate::{animated_slot, deanimate_pixels, deanimate_remap, deanimate_with};
pub use game_palette::{GAME_PALETTE, apply_game_statics};
pub use image_import::{ConvertOpts, ConvertSession, Coverage, Dedupe, image_to_wrl};
pub use pack::{FamilyProps, MatchRule, TileKind, TilePack, TilePattern, Transformable, family_of, replace_id_token};
pub use palette::{parse_palette, set_slot_rgb, slot_rgb, write_palette};
pub use palette_convert::{ConvertOptions, ConvertReport};
pub use project::{
	ANIMATED_SLOTS, DYNAMIC_SLOTS, LAYER_GROUND, LAYER_WATER, MAX_LAYERS, MapObject, ObjectProps, PaletteReimport,
	Project, RenderDirty, Rng, SynthesisSummary, SynthesizeSaveOptions, TileRef, Transform, UnexportedEdits, UseEntry,
	WATER_CYCLES, WATER_SLOTS, check_name_component, scenery_root, transform_tile, user_scenery_root,
};
pub use scenery::{
	BLEND_BAND, CutOpts, GroundInk, HGT_EXT, HGT_MAGIC, HeightOpts, ImageBand, PASS_EMPTY, RasterOpts, SCENERY_DIR,
	SCENERY_VERSION, SCN_EXT, SCN_MAGIC, SHADOW_ALPHA, SHADOW_BAND, SceneryBlend, SceneryPack, SceneryPiece,
	ScenerySpot, ShadeTable, ShadowFit, ShadowInk, Sprite, band_of, blend_keeps, brightness_table, cut, cut_image,
	decode_plane, default_peak, edge_distance, encode_plane, family_is_pyramid, family_is_sunken, family_peak,
	family_stands_low, height_field, height_from_grey, height_to_grey, ink_ranks, piece_family, rasterize, read_hgt,
	read_scn, rim_interior, write_hgt, write_scn,
};
pub use selection::{Edge, SelectMode, Selection};
pub use shore::{FixSession, FixStrength};
pub use template::{StampOp, Template, clear_selection, clear_selection_layer};
pub use worldgen::{AccessibilityMode, GenParams, GenSession, GenStats, Generator, Range, ShoreMethod, Span, Symmetry};
pub use wrl_import::{ExtrasDest, UnmappedTile, WrlImport};
