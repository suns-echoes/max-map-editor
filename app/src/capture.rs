//! Headless capture: render one frame offscreen and write it as a PNG. The
//! caller paints via the same `render_frame` composite as the live window —
//! only the target differs — so a `screenshot` command is always a faithful
//! sample of the current view. (Pattern lifted from world-editor.)

use std::path::Path;

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Create a `width`×`height` offscreen target, let `draw` paint it, read it
/// back, then optionally **crop** (a render-resolution sub-rect) and **resize**
/// (nearest-neighbour, crisp) before writing `path` — so one `screenshot` can
/// frame + magnify a region (e.g. small UI). Crop is applied before resize.
#[allow(clippy::too_many_arguments)]
pub fn render_to_png(
	device: &wgpu::Device,
	queue: &wgpu::Queue,
	width: u32,
	height: u32,
	path: &Path,
	crop: Option<(u32, u32, u32, u32)>,
	resize: Option<(u32, u32)>,
	draw: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::TextureView),
) {
	let texture = device.create_texture(&wgpu::TextureDescriptor {
		label: Some("capture.target"),
		size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format: FORMAT,
		usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
		view_formats: &[],
	});
	let view = texture.create_view(&Default::default());

	let mut encoder = device.create_command_encoder(&Default::default());
	draw(&mut encoder, &view);

	// Readback rows must be 256-byte aligned; unpad while copying out.
	let unpadded = width * 4;
	let padded = unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
	let buffer = device.create_buffer(&wgpu::BufferDescriptor {
		label: Some("capture.readback"),
		size: (padded * height) as u64,
		usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
		mapped_at_creation: false,
	});
	encoder.copy_texture_to_buffer(
		texture.as_image_copy(),
		wgpu::TexelCopyBufferInfo {
			buffer: &buffer,
			layout: wgpu::TexelCopyBufferLayout {
				offset: 0,
				bytes_per_row: Some(padded),
				rows_per_image: Some(height),
			},
		},
		wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
	);
	queue.submit([encoder.finish()]);

	let slice = buffer.slice(..);
	let (tx, rx) = std::sync::mpsc::channel();
	slice.map_async(wgpu::MapMode::Read, move |result| {
		let _ = tx.send(result);
	});
	device.poll(wgpu::PollType::Wait).expect("device poll");
	rx.recv().expect("map channel").expect("buffer map failed");

	let data = slice.get_mapped_range();
	let mut rgba = Vec::with_capacity((unpadded * height) as usize);
	for row in 0..height as usize {
		let offset = row * padded as usize;
		rgba.extend_from_slice(&data[offset..offset + unpadded as usize]);
	}
	drop(data);
	buffer.unmap();

	// Crop (render-res px) then nearest-neighbour resize — both operate on the
	// readback RGBA, so a single screenshot can frame + magnify a region.
	let (mut rgba, mut fw, mut fh) = (rgba, width, height);
	if let Some((cx, cy, cw, ch)) = crop {
		(rgba, fw, fh) = crop_rgba(&rgba, fw, fh, cx, cy, cw, ch);
	}
	if let Some((ow, oh)) = resize.filter(|&(w, h)| w > 0 && h > 0) {
		rgba = resize_nearest(&rgba, fw, fh, ow, oh);
		(fw, fh) = (ow, oh);
	}
	write_png(path, &rgba, fw, fh);
}

/// Extract a sub-rect (RGBA8); `(cx, cy)` is clamped inside `w`×`h` and the
/// size is clamped to what remains, so a too-large crop just hits the edge.
fn crop_rgba(src: &[u8], w: u32, h: u32, cx: u32, cy: u32, cw: u32, ch: u32) -> (Vec<u8>, u32, u32) {
	let cx = cx.min(w.saturating_sub(1));
	let cy = cy.min(h.saturating_sub(1));
	let cw = cw.min(w - cx).max(1);
	let ch = ch.min(h - cy).max(1);
	let mut out = Vec::with_capacity((cw * ch * 4) as usize);
	for y in 0..ch {
		let row = (((cy + y) * w + cx) * 4) as usize;
		out.extend_from_slice(&src[row..row + (cw * 4) as usize]);
	}
	(out, cw, ch)
}

/// Nearest-neighbour resize of `src` (RGBA8, `w`×`h`) to `ow`×`oh` — crisp
/// pixels for inspecting small UI, no blur.
fn resize_nearest(src: &[u8], w: u32, h: u32, ow: u32, oh: u32) -> Vec<u8> {
	let mut out = vec![0u8; (ow * oh * 4) as usize];
	for oy in 0..oh {
		let sy = (oy * h / oh).min(h - 1);
		for ox in 0..ow {
			let sx = (ox * w / ow).min(w - 1);
			let s = ((sy * w + sx) * 4) as usize;
			let d = ((oy * ow + ox) * 4) as usize;
			out[d..d + 4].copy_from_slice(&src[s..s + 4]);
		}
	}
	out
}

fn write_png(path: &Path, rgba: &[u8], width: u32, height: u32) {
	if let Some(parent) = path.parent() {
		let _ = std::fs::create_dir_all(parent);
	}
	let file = std::fs::File::create(path).expect("create png file");
	let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
	encoder.set_color(png::ColorType::Rgba);
	encoder.set_depth(png::BitDepth::Eight);
	let mut writer = encoder.write_header().expect("png header");
	writer.write_image_data(rgba).expect("png data");
	eprintln!("screenshot: wrote {} ({width}×{height})", path.display());
}
