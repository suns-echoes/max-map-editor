//! The resumable rasterize-and-reimport palette-conversion session
//! (`PaletteReimport`), split out of the document model in `super`.

use super::*;
use crate::palette::slot_rgb;

/// The resumable rasterize-and-reimport palette conversion: render the map
/// through its internal palette (the "Rendering map" phase), then run the
/// raster through the New-from-Image [`ConvertSession`](crate::ConvertSession)
/// pipeline. The shell drives it per frame - `step` does a bounded slice and
/// reports `progress`/`stage` - so the modal stays live (progress bar, ETA,
/// Abort). [`Project::convert_palette_by_reimport`] is the run-to-completion
/// convenience over this (scripts/headless).
///
/// The session borrows nothing: `step` re-takes the project each call, so it
/// parks in the modal between frames. The project must not change under a
/// live session (the modal blocks input; a dimension change is caught and
/// reported as an error rather than composing out of bounds).
pub struct PaletteReimport {
	preserve_water: bool,
	width: u16,
	height: u16,
	internal: Vec<u8>,
	/// Target raster (filled during the render phase, then moved into `inner`).
	rgba: Vec<u8>,
	pins: Vec<u8>,
	/// Next cell row to rasterize.
	row: usize,
	dedupe: crate::image_import::Dedupe,
	threshold: f32,
	inner: Option<crate::image_import::ConvertSession>,
	error: Option<String>,
}

/// The render phase's share of the progress bar (the re-import pipeline's
/// own phases fill the rest).
const RASTER_WEIGHT: f32 = 0.15;

impl PaletteReimport {
	pub fn new(project: &Project, preserve_water: bool, dedupe: crate::image_import::Dedupe, threshold: f32) -> Self {
		let (tw, th) = (project.width as usize * TILE_SIZE, project.height as usize * TILE_SIZE);
		Self {
			preserve_water,
			width: project.width,
			height: project.height,
			internal: project.internal_palette(),
			rgba: vec![0u8; tw * th * 4],
			pins: vec![0u8; tw * th],
			row: 0,
			dedupe,
			threshold,
			inner: None,
			error: None,
		}
	}

	pub fn is_done(&self) -> bool {
		self.error.is_some() || self.inner.as_ref().is_some_and(|s| s.is_done())
	}

	pub fn error(&self) -> Option<&str> {
		self.error.as_deref().or_else(|| self.inner.as_ref().and_then(|s| s.error()))
	}

	/// 0..1 overall progress (render phase first, then the import pipeline).
	pub fn progress(&self) -> f32 {
		match &self.inner {
			Some(s) => RASTER_WEIGHT + (1.0 - RASTER_WEIGHT) * s.progress(),
			None => RASTER_WEIGHT * self.row as f32 / self.height.max(1) as f32,
		}
	}

	pub fn stage(&self) -> &'static str {
		match &self.inner {
			Some(s) => s.stage(),
			None => "Rendering map",
		}
	}

	/// Do bounded work - render cell rows, then hand the raster to the
	/// re-import pipeline and step it. `budget` is in pixel-equivalent units
	/// (one cell = 4096).
	pub fn step(&mut self, project: &Project, budget: usize) {
		if self.is_done() {
			return;
		}
		if (project.width, project.height) != (self.width, self.height) {
			self.error = Some("the document changed under the conversion".into());
			return;
		}
		let (w, h) = (self.width as usize, self.height as usize);
		let tw = w * TILE_SIZE;
		let mut done = 0usize;
		while self.row < h && done < budget {
			let cy = self.row;
			for cx in 0..w {
				let tile = project.compose_cell(cx as u16, cy as u16);
				for py in 0..TILE_SIZE {
					let row = (cy * TILE_SIZE + py) * tw + cx * TILE_SIZE;
					for px in 0..TILE_SIZE {
						let idx = tile[py * TILE_SIZE + px];
						let at = (row + px) * 4;
						self.rgba[at..at + 3].copy_from_slice(&slot_rgb(&self.internal, idx));
						self.rgba[at + 3] = 255;
						if self.preserve_water && WATER_SLOTS.contains(&idx) {
							self.pins[row + px] = idx;
						}
					}
				}
			}
			self.row += 1;
			done += w * TILE_DATA_SIZE / 16; // a composed cell is cheaper than a dithered one
		}
		if self.row < h {
			return;
		}
		if self.inner.is_none() {
			// Raster complete - build the import session (moves the buffers).
			let th = h * TILE_SIZE;
			let opts = crate::image_import::ConvertOpts {
				dedupe: self.dedupe,
				threshold: self.threshold,
				..crate::image_import::ConvertOpts::fit_source(tw as u32, th as u32)
			};
			let rgba = std::mem::take(&mut self.rgba);
			let pins = std::mem::take(&mut self.pins);
			match crate::image_import::ConvertSession::new(rgba, tw as u32, th as u32, opts) {
				Ok(mut session) => {
					if self.preserve_water {
						let water: Vec<(u8, [u8; 3])> = WATER_SLOTS.map(|s| (s, slot_rgb(&self.internal, s))).collect();
						if let Err(e) = session.pin(pins, &water) {
							self.error = Some(e);
							return;
						}
					}
					self.inner = Some(session);
				}
				Err(e) => {
					self.error = Some(e);
					return;
				}
			}
		}
		if let Some(session) = self.inner.as_mut() {
			session.step(budget.saturating_sub(done).max(1));
		}
	}

	/// Consume the finished session into a `WrlFile` (call once `is_done`; an
	/// errored session returns its error here).
	pub fn finish(mut self) -> Result<WrlFile, String> {
		if let Some(e) = self.error.take() {
			return Err(e);
		}
		self.inner.take().ok_or("conversion not finished")?.finish()
	}
}
