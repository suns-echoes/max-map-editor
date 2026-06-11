//! Shared render constants + the map uniforms. The map itself is drawn by
//! `project_render::ProjectRenderer` (composes cells from the project's packs).

pub const TILE_PX: u32 = 64;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
	pub screen_size: [f32; 2],
	/// World px at the viewport's top-left corner.
	pub pan: [f32; 2],
	/// Map dimensions in tiles.
	pub map_size: [f32; 2],
	/// Screen px per world px.
	pub zoom: f32,
	/// Unused by the project renderer (it composes from packs); kept 0.
	pub tiles_per_row: u32,
}
