//! The UI coverage atlas: a single-channel **R8** texture sampled by every UI
//! quad drawn from it. Solid fills sample a reserved opaque-white (coverage 1)
//! texel; glyphs are packed in with a simple shelf allocator and stored as raw
//! coverage. The shader reads the red channel as coverage and takes RGB from the
//! vertex color (`mode == 1`), so one bind group and one pipeline serve solids
//! and text; host RGBA sprites use their own textures through the same pipeline.
//! R8 stores coverage in one byte instead of four — a quarter the memory of an
//! RGBA atlas.

use crate::draw::TexRect;

/// Side of the reserved opaque-white block (kept >1px so its sampled center is
/// safely interior).
const WHITE_BLOCK: u32 = 2;
/// Transparent gap kept between packed entries (avoids neighbor bleed).
const GAP: u32 = 1;

pub struct UiAtlas {
    size: u32,
    /// Bytes per texel: 1 (R8 coverage) or 4 (RGBA color glyphs).
    bpp: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    white: (u32, u32),
    // Shelf allocator: a cursor that fills left-to-right, top-to-bottom.
    cursor_x: u32,
    cursor_y: u32,
    shelf_h: u32,
}

impl UiAtlas {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, size: u32) -> Self {
        Self::with_format(device, queue, size, wgpu::TextureFormat::R8Unorm, 1)
    }

    /// An RGBA (sRGB) atlas — what COLOR glyphs (emoji) pack into; they draw
    /// through the plain-texture sample mode, untinted.
    pub fn new_rgba(device: &wgpu::Device, queue: &wgpu::Queue, size: u32) -> Self {
        Self::with_format(device, queue, size, wgpu::TextureFormat::Rgba8UnormSrgb, 4)
    }

    fn with_format(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: u32,
        format: wgpu::TextureFormat,
        bpp: u32,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("wgpu-ui atlas"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut atlas = Self {
            size,
            bpp,
            texture,
            view,
            white: (0, 0),
            cursor_x: 0,
            cursor_y: 0,
            shelf_h: 0,
        };

        // Reserve and fill the white block at the atlas origin.
        let white = atlas
            .alloc(WHITE_BLOCK, WHITE_BLOCK)
            .expect("atlas fits the white block");
        atlas.white = white;
        // Opaque-white coverage (1.0) so solids that sample this texel pass the
        // vertex color through unchanged.
        let pixels = vec![255u8; (WHITE_BLOCK * WHITE_BLOCK * bpp) as usize];
        atlas.upload(queue, white.0, white.1, WHITE_BLOCK, WHITE_BLOCK, &pixels);
        atlas
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// The current side length (px). Grows when the renderer re-creates a larger
    /// atlas on exhaustion (see `UiKit::grow_atlas`).
    pub fn size(&self) -> u32 {
        self.size
    }

    /// UV of the reserved white texel — what solid fills sample. All four quad
    /// corners use this single point, so the quad reads opaque white and shows
    /// the vertex color.
    pub fn white_uv(&self) -> TexRect {
        let cx = (self.white.0 as f32 + WHITE_BLOCK as f32 * 0.5) / self.size as f32;
        let cy = (self.white.1 as f32 + WHITE_BLOCK as f32 * 0.5) / self.size as f32;
        TexRect::new(cx, cy, cx, cy)
    }

    /// Reserves a `w`x`h` region, returning its top-left texel, or `None` when
    /// the atlas is full.
    pub fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let (aw, ah) = (w + GAP, h + GAP);
        if self.cursor_x + aw > self.size {
            self.cursor_y += self.shelf_h;
            self.cursor_x = 0;
            self.shelf_h = 0;
        }
        if self.cursor_y + ah > self.size {
            return None;
        }
        let pos = (self.cursor_x, self.cursor_y);
        self.cursor_x += aw;
        self.shelf_h = self.shelf_h.max(ah);
        Some(pos)
    }

    /// Uploads tightly packed texels (`bpp` bytes each — coverage or RGBA,
    /// matching the constructor) into a previously
    /// [`alloc`](Self::alloc)ated region.
    pub fn upload(&self, queue: &wgpu::Queue, x: u32, y: u32, w: u32, h: u32, texels: &[u8]) {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            texels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * self.bpp),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Normalized UV rectangle for a packed region.
    pub fn uv(&self, x: u32, y: u32, w: u32, h: u32) -> TexRect {
        let s = self.size as f32;
        TexRect::new(
            x as f32 / s,
            y as f32 / s,
            (x + w) as f32 / s,
            (y + h) as f32 / s,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::UiAtlas;
    use crate::gpu::test_gpu;

    /// Shelf allocation: entries fill a row left-to-right, wrap into a new
    /// shelf below the row's tallest entry (plus the 1px bleed gap), and
    /// exhaustion reports `None` — the renderer's cue to grow the atlas.
    #[test]
    fn shelf_allocator_wraps_and_reports_exhaustion() {
        let Some((device, queue, _serial)) = test_gpu() else {
            eprintln!("no GPU adapter; skipping shelf_allocator_wraps_and_reports_exhaustion");
            return;
        };
        let mut atlas = UiAtlas::new(&device, &queue, 32);
        assert_eq!(atlas.size(), 32);
        // The white block owns the shelf origin: (0,0)..(2,2) plus a 1px gap.
        let a = atlas.alloc(26, 4).expect("fits the first shelf");
        assert_eq!(a, (3, 0), "packs to the right of the white block");
        let b = atlas.alloc(8, 4).expect("wraps to a second shelf");
        assert_eq!(
            b,
            (0, 5),
            "starts below the first shelf's tallest entry (4 + gap)"
        );
        assert_eq!(
            atlas.alloc(8, 30),
            None,
            "too tall for the remaining height: the atlas is full"
        );
    }
}
