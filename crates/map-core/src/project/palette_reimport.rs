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

#[cfg(test)]
mod tests {
	use super::*;

	/// A 1×2 synthetic WRL import (one water-cycle tile, one static-slot tile) -
	/// small enough to drive the session row by row.
	fn wrl_project() -> Project {
		let mut tiles = vec![0u8; 2 * TILE_DATA_SIZE];
		tiles[..TILE_DATA_SIZE].fill(100); // water-cycle slot
		tiles[TILE_DATA_SIZE..].fill(40); // a static slot
		Project::from_wrl(
			&WrlFile {
				header: vec![0; 5],
				width: 1,
				height: 2,
				minimap: vec![100, 40],
				bigmap: vec![0, 1],
				tile_count: 2,
				tiles,
				palette: crate::GAME_PALETTE.to_vec(),
				pass_table: vec![1, 0],
			},
			"STEP",
		)
	}

	/// Budget-limited stepping reports the render phase first (its own stage
	/// label, progress under the raster share), then hands over to the import
	/// pipeline and runs to a clean finish.
	#[test]
	fn budgeted_steps_walk_render_then_the_import_pipeline() {
		let p = wrl_project();
		let mut s = PaletteReimport::new(&p, true, crate::image_import::Dedupe::Strict, 0.0);
		assert_eq!(s.stage(), "Rendering map", "the render phase reports its own stage");
		assert_eq!(s.progress(), 0.0);
		assert!(s.error().is_none() && !s.is_done());
		// Budget 1 renders exactly one cell row, then parks (1 of 2 rows done).
		s.step(&p, 1);
		assert!(!s.is_done());
		let mid = s.progress();
		assert!(mid > 0.0 && mid <= RASTER_WEIGHT, "render progress stays within its share, got {mid}");
		assert_eq!(s.stage(), "Rendering map");
		// Keep stepping: the raster completes and the pipeline takes over.
		let mut guard = 0;
		while !s.is_done() {
			s.step(&p, 4096);
			assert!(s.error().is_none(), "a clean conversion never errors");
			guard += 1;
			assert!(guard < 10_000, "must terminate");
		}
		assert!(s.progress() >= RASTER_WEIGHT, "pipeline progress rides above the render share");
		let wrl = s.finish().expect("clean finish");
		assert_eq!((wrl.width, wrl.height), (1, 2));
		assert_eq!(&wrl.tiles[..TILE_DATA_SIZE], &[100u8; TILE_DATA_SIZE][..], "pinned water kept its slot");
	}

	/// A dimension change under a live session is caught as an error: the
	/// session is done, reports it, ignores further steps, and `finish`
	/// surfaces it instead of a WRL.
	#[test]
	fn document_change_under_the_session_errors_out() {
		let mut p = wrl_project();
		let mut s = PaletteReimport::new(&p, false, crate::image_import::Dedupe::Strict, 0.0);
		p.resize(2, 2, 0, 0).unwrap();
		s.step(&p, usize::MAX);
		assert!(s.is_done(), "the mismatch ends the session");
		assert!(s.error().unwrap().contains("changed under"), "{:?}", s.error());
		s.step(&p, usize::MAX); // a done session ignores further steps
		assert!(s.error().is_some());
		let err = s.finish().unwrap_err();
		assert!(err.contains("changed under"), "{err}");
	}
}
