//! GPU bring-up: windowed (surface + swapchain) and headless (device only,
//! used by `--screenshot`). Trimmed from re-max's `GpuContext` - no offscreen
//! scene buffers yet, the tile pass renders straight to the target.

use std::sync::Arc;

use winit::window::Window;

pub struct WindowGpu {
	pub device: wgpu::Device,
	pub queue: wgpu::Queue,
	pub surface: wgpu::Surface<'static>,
	pub config: wgpu::SurfaceConfiguration,
}

impl WindowGpu {
	pub async fn new(window: Arc<Window>) -> Self {
		let size = window.inner_size();

		let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
			backends: wgpu::Backends::all(),
			..wgpu::InstanceDescriptor::new_without_display_handle()
		});

		let surface = instance.create_surface(window).expect("create surface");

		let adapter = pick_adapter(&instance, Some(&surface)).await;
		let (device, queue) = request_device(&adapter).await;

		let caps = surface.get_capabilities(&adapter);
		let format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0]);

		let config = wgpu::SurfaceConfiguration {
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			format,
			// `Auto` reproduces wgpu's historical behaviour: sRGB for the sRGB
			// format picked above, which is what the shaders' linear output wants.
			color_space: wgpu::SurfaceColorSpace::Auto,
			width: size.width.max(1),
			height: size.height.max(1),
			present_mode: wgpu::PresentMode::Fifo,
			alpha_mode: wgpu::CompositeAlphaMode::Auto,
			view_formats: vec![],
			desired_maximum_frame_latency: 2,
		};
		surface.configure(&device, &config);

		Self { device, queue, surface, config }
	}

	pub fn resize(&mut self, width: u32, height: u32) {
		if width == 0 || height == 0 {
			return;
		}
		self.config.width = width;
		self.config.height = height;
		self.surface.configure(&self.device, &self.config);
	}
}

/// Device + queue without a window - the `--screenshot` path.
pub async fn headless() -> (wgpu::Device, wgpu::Queue) {
	let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
		backends: wgpu::Backends::all(),
		..wgpu::InstanceDescriptor::new_without_display_handle()
	});
	let adapter = pick_adapter(&instance, None).await;
	request_device(&adapter).await
}

async fn pick_adapter(instance: &wgpu::Instance, surface: Option<&wgpu::Surface<'_>>) -> wgpu::Adapter {
	let options = |force_fallback_adapter| wgpu::RequestAdapterOptions {
		power_preference: wgpu::PowerPreference::HighPerformance,
		compatible_surface: surface,
		force_fallback_adapter,
		// Bucketed limits exist to stop untrusted content fingerprinting the
		// adapter; this is a local tool, and the tile atlas wants the real
		// `max_texture_dimension_2d` (see `request_device` below).
		apply_limit_buckets: false,
	};
	// Prefer a real GPU, but fall back to a software rasterizer (lavapipe / WARP)
	// when there's no hardware adapter - so the headless path (`--headless` /
	// `--screenshot`) still renders on a GPU-less CI box.
	match instance.request_adapter(&options(false)).await {
		Ok(adapter) => adapter,
		Err(_) => instance
			.request_adapter(&options(true))
			.await
			.expect("no GPU adapter found (hardware or software fallback)"),
	}
}

async fn request_device(adapter: &wgpu::Adapter) -> (wgpu::Device, wgpu::Queue) {
	// Request the adapter's *own* limits (always grantable, since they're what it
	// reports), not wgpu's conservative defaults - so the tile atlas can size its
	// layers to the real `max_texture_dimension_2d` / `max_texture_array_layers`.
	adapter
		.request_device(&wgpu::DeviceDescriptor {
			label: Some("max-map-editor.device"),
			required_limits: adapter.limits(),
			..Default::default()
		})
		.await
		.expect("request GPU device")
}
