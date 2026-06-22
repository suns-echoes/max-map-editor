//! Shared render constants + the map uniforms. The map itself is drawn by
//! `project_render::ProjectRenderer` (composes cells from the project's packs).

pub const TILE_PX: u32 = 64;

/// Begin a render pass that **loads** (preserves) the target's existing
/// contents and stores the result - the single-color-attachment "draw over
/// what's already there" pass every overlay/UI renderer uses.
pub fn load_pass<'a>(
	encoder: &'a mut wgpu::CommandEncoder,
	target: &'a wgpu::TextureView,
	label: &str,
) -> wgpu::RenderPass<'a> {
	encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
		label: Some(label),
		color_attachments: &[Some(wgpu::RenderPassColorAttachment {
			view: target,
			resolve_target: None,
			ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
			depth_slice: None,
		})],
		depth_stencil_attachment: None,
		timestamp_writes: None,
		occlusion_query_set: None,
	})
}

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
