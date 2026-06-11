//! Image → map conversion: quantize an imported image into the
//! palette and reblock it into 64×64 tiles, producing a `WrlFile` the editor
//! opens via [`Project::from_wrl`](crate::Project::from_wrl) — so an import
//! rides the same synthetic-pack path as a `.WRL` open.
//!
//! Palette policy (tileset-contract §1): the game-static slots are **preserved**
//! at their in-game values and the **animated** cycle slots (9..=31, 96..=127)
//! are **never emitted**, so an import stays game-legal and doesn't shimmer.
//! Colour binning fills only the dynamic, non-animated slots (64..=95,
//! 128..=159) via k-means over the image's colour histogram; each pixel is then
//! Floyd–Steinberg dithered to the nearest emittable slot.
//!
//! The work is exposed as a resumable [`ConvertSession`] — `step` does a bounded
//! slice and reports `progress`/`stage`, so the editor's New-from-Image modal
//! can drive it per frame (progress bar, ETA, Abort) without freezing the UI.
//! [`image_to_wrl`] is the run-to-completion convenience over that session.

use std::collections::HashMap;

use max_assets::wrl::{TILE_DATA_SIZE, TILE_SIZE, WrlFile};

use crate::game_palette::GAME_PALETTE;

/// The dynamic, non-animated palette slots image colours quantize into.
const K: usize = 64; // 64..=95 (32) + 128..=159 (32)
const KMEANS_ITERS: usize = 10;
/// Max per-pixel RGB Euclidean distance (sqrt(3)·255) — relaxed-dedupe scale.
const MAX_PX_DIST: f32 = 441.672_94;

/// How the source image is mapped onto the chosen map dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coverage {
	/// 1:1 pixels, centred (+offset); pad with index 0 / crop the overflow.
	Crop,
	/// Scale each axis independently to fill the map exactly (distorts aspect).
	Stretch,
	/// Scale uniformly to cover the map (preserves aspect), centred (+offset).
	Fill,
}

/// Tile-collapsing strategy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dedupe {
	/// Only byte-identical tiles collapse.
	Strict,
	/// Tiles within `threshold` average colour distance collapse to one.
	Relaxed,
}

/// Conversion parameters (the New-from-Image modal's settings).
#[derive(Clone, Copy, Debug)]
pub struct ConvertOpts {
	pub width_tiles: u32,
	pub height_tiles: u32,
	pub coverage: Coverage,
	pub off_x: i32,
	pub off_y: i32,
	pub dedupe: Dedupe,
	/// Relaxed similarity threshold as a fraction (0..1) of the max per-pixel
	/// colour distance; unused for [`Dedupe::Strict`].
	pub threshold: f32,
}

impl ConvertOpts {
	/// Defaults that reproduce a 1:1 import of the source: tiles = source / 64,
	/// stretch identity, exact dedupe.
	pub fn fit_source(src_w: u32, src_h: u32) -> Self {
		Self {
			width_tiles: (src_w / TILE_SIZE as u32).max(1),
			height_tiles: (src_h / TILE_SIZE as u32).max(1),
			coverage: Coverage::Stretch,
			off_x: 0,
			off_y: 0,
			dedupe: Dedupe::Strict,
			threshold: 0.0,
		}
	}
}

/// Whether a slot lies in an animated colour-cycle range (never emitted).
fn is_animated(slot: u8) -> bool {
	(9..=31).contains(&slot) || (96..=127).contains(&slot)
}

/// The 64 dynamic, non-animated slots, in fill order.
fn fill_slots() -> impl Iterator<Item = u8> {
	(64u8..=95).chain(128u8..=159)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
	Resample,
	Cluster,
	Dither,
	Reblock,
	Done,
}

/// A resumable image→`WrlFile` conversion. Build with [`ConvertSession::new`],
/// drive with [`step`](Self::step) until [`is_done`](Self::is_done), then take
/// the result with [`finish`](Self::finish).
pub struct ConvertSession {
	opts: ConvertOpts,
	src: Vec<u8>,
	src_w: usize,
	src_h: usize,
	tw: usize, // target raster width  (width_tiles * 64)
	th: usize, // target raster height (height_tiles * 64)
	tiles_x: usize,
	tiles_y: usize,

	target: Vec<u8>, // tw*th*3, filled during Resample
	hist: HashMap<[u8; 3], u32>,
	colors: Vec<([f32; 3], f32)>, // sorted histogram (built entering Cluster)
	centroids: Vec<[f32; 3]>,
	palette: Vec<u8>,
	emit: Vec<u8>,
	emit_rgb: Vec<[f32; 3]>,

	buf: Vec<f32>, // tw*th*3 dither working buffer
	indices: Vec<u8>,

	tiles: Vec<u8>,
	bigmap: Vec<u16>,
	exact: HashMap<[u8; TILE_DATA_SIZE], u16>, // Strict dedupe
	pal_rgb: Vec<[f32; 3]>,                    // 256, built entering Reblock
	groups: Vec<Group>,                        // Relaxed dedupe (group id == tile id)

	cursor: usize, // row (Resample/Dither), iteration (Cluster), or cell (Reblock)
	phase: Phase,
	error: Option<String>,
}

impl ConvertSession {
	pub fn new(rgba: Vec<u8>, src_w: u32, src_h: u32, opts: ConvertOpts) -> Result<Self, String> {
		if src_w == 0 || src_h == 0 {
			return Err("source image is empty".into());
		}
		let want = src_w as usize * src_h as usize * 4;
		if rgba.len() != want {
			return Err(format!("rgba is {} bytes, want {want} for {src_w}×{src_h}", rgba.len()));
		}
		if !(1..=1024).contains(&opts.width_tiles) || !(1..=1024).contains(&opts.height_tiles) {
			return Err(format!("map size {}×{} tiles (1..=1024)", opts.width_tiles, opts.height_tiles));
		}
		let (tiles_x, tiles_y) = (opts.width_tiles as usize, opts.height_tiles as usize);
		let (tw, th) = (tiles_x * TILE_SIZE, tiles_y * TILE_SIZE);

		let mut palette = GAME_PALETTE.to_vec();
		let emit: Vec<u8> = (0u8..=255).filter(|&s| !is_animated(s)).collect();
		// Palette dynamic slots are filled after Cluster; emit_rgb is rebuilt then.
		let emit_rgb = emit
			.iter()
			.map(|&s| {
				let at = s as usize * 3;
				[palette[at] as f32, palette[at + 1] as f32, palette[at + 2] as f32]
			})
			.collect();
		palette.truncate(768);

		Ok(Self {
			opts,
			src: rgba,
			src_w: src_w as usize,
			src_h: src_h as usize,
			tw,
			th,
			tiles_x,
			tiles_y,
			target: vec![0; tw * th * 3],
			hist: HashMap::new(),
			colors: Vec::new(),
			centroids: Vec::new(),
			palette,
			emit,
			emit_rgb,
			buf: Vec::new(),
			indices: vec![0; tw * th],
			tiles: Vec::new(),
			bigmap: Vec::with_capacity(tiles_x * tiles_y),
			exact: HashMap::new(),
			pal_rgb: Vec::new(),
			groups: Vec::new(),
			cursor: 0,
			phase: Phase::Resample,
			error: None,
		})
	}

	pub fn is_done(&self) -> bool {
		self.phase == Phase::Done || self.error.is_some()
	}

	pub fn error(&self) -> Option<&str> {
		self.error.as_deref()
	}

	/// 0..1 overall progress (phase-weighted).
	pub fn progress(&self) -> f32 {
		let frac = |cur: usize, total: usize| if total == 0 { 1.0 } else { (cur as f32 / total as f32).min(1.0) };
		match self.phase {
			Phase::Resample => 0.10 * frac(self.cursor, self.th),
			Phase::Cluster => 0.10 + 0.30 * frac(self.cursor, KMEANS_ITERS),
			Phase::Dither => 0.40 + 0.40 * frac(self.cursor, self.th),
			Phase::Reblock => 0.80 + 0.20 * frac(self.cursor, self.tiles_x * self.tiles_y),
			Phase::Done => 1.0,
		}
	}

	pub fn stage(&self) -> &'static str {
		match self.phase {
			Phase::Resample => "Resampling image",
			Phase::Cluster => "Clustering colours",
			Phase::Dither => "Dithering pixels",
			Phase::Reblock => "Building tiles",
			Phase::Done => "Done",
		}
	}

	/// Do bounded work — at least one chunk, then more until ~`budget` pixel-
	/// equivalent units are processed. Returns when the budget is spent or the
	/// conversion finishes/errors.
	pub fn step(&mut self, budget: usize) {
		let mut done = 0usize;
		while done < budget && !self.is_done() {
			done += self.advance();
		}
	}

	/// Advance one chunk of the current phase; returns its approximate cost.
	fn advance(&mut self) -> usize {
		match self.phase {
			Phase::Resample => self.resample_chunk(),
			Phase::Cluster => self.cluster_chunk(),
			Phase::Dither => self.dither_chunk(),
			Phase::Reblock => self.reblock_chunk(),
			Phase::Done => 0,
		}
	}

	/// Consume the finished session into a `WrlFile` (call once `is_done`; a
	/// conversion that errored returns its error here).
	pub fn finish(mut self) -> Result<WrlFile, String> {
		if let Some(e) = self.error.take() {
			return Err(e);
		}
		let tile_count = (self.tiles.len() / TILE_DATA_SIZE) as u16;
		// Minimap = each cell's (final) representative tile's centre pixel; built
		// here so relaxed-dedupe median updates are reflected.
		let center = (TILE_SIZE / 2) * TILE_SIZE + TILE_SIZE / 2;
		let minimap = self.bigmap.iter().map(|&id| self.tiles[id as usize * TILE_DATA_SIZE + center]).collect();
		Ok(WrlFile {
			header: vec![0; 5],
			width: self.tiles_x as u16,
			height: self.tiles_y as u16,
			minimap,
			bigmap: self.bigmap,
			tile_count,
			tiles: self.tiles,
			palette: self.palette,
			pass_table: vec![0; tile_count as usize],
		})
	}

	// ----- phase 1: resample + histogram ------------------------------------

	fn resample_chunk(&mut self) -> usize {
		let rows = 16.min(self.th - self.cursor);
		for ry in 0..rows {
			let ty = self.cursor + ry;
			for tx in 0..self.tw {
				let [r, g, b, _] = self.sample(tx, ty);
				let at = (ty * self.tw + tx) * 3;
				self.target[at] = r;
				self.target[at + 1] = g;
				self.target[at + 2] = b;
				*self.hist.entry([r, g, b]).or_insert(0) += 1;
			}
		}
		self.cursor += rows;
		if self.cursor >= self.th {
			self.enter_cluster();
		}
		rows * self.tw
	}

	fn src_px(&self, x: usize, y: usize) -> [u8; 4] {
		let at = (y * self.src_w + x) * 4;
		[self.src[at], self.src[at + 1], self.src[at + 2], self.src[at + 3]]
	}

	fn bilinear(&self, fx: f32, fy: f32) -> [u8; 4] {
		let cx = |x: i32| x.clamp(0, self.src_w as i32 - 1) as usize;
		let cy = |y: i32| y.clamp(0, self.src_h as i32 - 1) as usize;
		let (x0, y0) = (fx.floor() as i32, fy.floor() as i32);
		let (dx, dy) = (fx - x0 as f32, fy - y0 as f32);
		let p00 = self.src_px(cx(x0), cy(y0));
		let p10 = self.src_px(cx(x0 + 1), cy(y0));
		let p01 = self.src_px(cx(x0), cy(y0 + 1));
		let p11 = self.src_px(cx(x0 + 1), cy(y0 + 1));
		let mut out = [0u8; 4];
		for c in 0..4 {
			let top = p00[c] as f32 * (1.0 - dx) + p10[c] as f32 * dx;
			let bot = p01[c] as f32 * (1.0 - dx) + p11[c] as f32 * dx;
			out[c] = round_u8(top * (1.0 - dy) + bot * dy);
		}
		out
	}

	/// One target pixel's source colour, per the coverage mode.
	fn sample(&self, tx: usize, ty: usize) -> [u8; 4] {
		match self.opts.coverage {
			Coverage::Crop => {
				let base_x = (self.tw as i32 - self.src_w as i32) / 2;
				let base_y = (self.th as i32 - self.src_h as i32) / 2;
				let sx = tx as i32 - base_x - self.opts.off_x;
				let sy = ty as i32 - base_y - self.opts.off_y;
				if sx < 0 || sy < 0 || sx >= self.src_w as i32 || sy >= self.src_h as i32 {
					[0, 0, 0, 255]
				} else {
					self.src_px(sx as usize, sy as usize)
				}
			}
			Coverage::Stretch => {
				let fx = (tx as f32 + 0.5) * self.src_w as f32 / self.tw as f32 - 0.5;
				let fy = (ty as f32 + 0.5) * self.src_h as f32 / self.th as f32 - 0.5;
				self.bilinear(fx, fy)
			}
			Coverage::Fill => {
				let s = (self.tw as f32 / self.src_w as f32).max(self.th as f32 / self.src_h as f32);
				let ox = (self.tw as f32 - self.src_w as f32 * s) * 0.5 + self.opts.off_x as f32;
				let oy = (self.th as f32 - self.src_h as f32 * s) * 0.5 + self.opts.off_y as f32;
				self.bilinear((tx as f32 - ox) / s, (ty as f32 - oy) / s)
			}
		}
	}

	// ----- phase 2: k-means over the colour histogram -----------------------

	fn enter_cluster(&mut self) {
		let mut colors: Vec<([f32; 3], f32)> =
			self.hist.iter().map(|(&c, &w)| ([c[0] as f32, c[1] as f32, c[2] as f32], w as f32)).collect();
		colors.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| key(a.0).cmp(&key(b.0))));
		self.centroids = colors.iter().take(K).map(|&(c, _)| c).collect();
		while self.centroids.len() < K {
			self.centroids.push(self.centroids.last().copied().unwrap_or([0.0; 3]));
		}
		self.colors = colors;
		self.cursor = 0;
		self.phase = Phase::Cluster;
	}

	fn cluster_chunk(&mut self) -> usize {
		let mut sum = vec![[0.0f64; 3]; K];
		let mut wsum = vec![0.0f64; K];
		for &(c, w) in &self.colors {
			let k = nearest(&c, &self.centroids);
			for j in 0..3 {
				sum[k][j] += (c[j] * w) as f64;
			}
			wsum[k] += w as f64;
		}
		let mut moved = 0.0f32;
		for k in 0..K {
			if wsum[k] > 0.0 {
				let nc = [(sum[k][0] / wsum[k]) as f32, (sum[k][1] / wsum[k]) as f32, (sum[k][2] / wsum[k]) as f32];
				moved += dist_sq(&nc, &self.centroids[k]);
				self.centroids[k] = nc;
			}
		}
		self.cursor += 1;
		if self.cursor >= KMEANS_ITERS || moved < 1.0 {
			self.enter_dither();
		}
		self.colors.len().max(1)
	}

	fn enter_dither(&mut self) {
		for (slot, c) in fill_slots().zip(self.centroids.iter()) {
			let at = slot as usize * 3;
			self.palette[at] = round_u8(c[0]);
			self.palette[at + 1] = round_u8(c[1]);
			self.palette[at + 2] = round_u8(c[2]);
		}
		self.emit_rgb = self
			.emit
			.iter()
			.map(|&s| {
				let at = s as usize * 3;
				[self.palette[at] as f32, self.palette[at + 1] as f32, self.palette[at + 2] as f32]
			})
			.collect();
		self.buf = self.target.iter().map(|&b| b as f32).collect();
		self.cursor = 0;
		self.phase = Phase::Dither;
	}

	// ----- phase 3: Floyd–Steinberg dither ----------------------------------

	fn dither_chunk(&mut self) -> usize {
		let rows = 16.min(self.th - self.cursor);
		let w = self.tw;
		for ry in 0..rows {
			let y = self.cursor + ry;
			for x in 0..w {
				let p = (y * w + x) * 3;
				let old = [self.buf[p], self.buf[p + 1], self.buf[p + 2]];
				let ei = nearest(&old, &self.emit_rgb);
				self.indices[y * w + x] = self.emit[ei];
				let chosen = self.emit_rgb[ei];
				let err = [old[0] - chosen[0], old[1] - chosen[1], old[2] - chosen[2]];
				if x + 1 < w {
					add3(&mut self.buf, (y * w + x + 1) * 3, err, 7.0 / 16.0);
				}
				if y + 1 < self.th {
					if x > 0 {
						add3(&mut self.buf, ((y + 1) * w + x - 1) * 3, err, 3.0 / 16.0);
					}
					add3(&mut self.buf, ((y + 1) * w + x) * 3, err, 5.0 / 16.0);
					if x + 1 < w {
						add3(&mut self.buf, ((y + 1) * w + x + 1) * 3, err, 1.0 / 16.0);
					}
				}
			}
		}
		self.cursor += rows;
		if self.cursor >= self.th {
			self.enter_reblock();
		}
		rows * w
	}

	fn enter_reblock(&mut self) {
		self.buf = Vec::new(); // free the dither buffer
		self.pal_rgb = (0..256)
			.map(|s| [self.palette[s * 3] as f32, self.palette[s * 3 + 1] as f32, self.palette[s * 3 + 2] as f32])
			.collect();
		self.cursor = 0;
		self.phase = Phase::Reblock;
	}

	// ----- phase 4: reblock into 64×64 tiles + dedupe -----------------------

	fn reblock_chunk(&mut self) -> usize {
		let n = TILE_SIZE;
		let total = self.tiles_x * self.tiles_y;
		let cells = match self.opts.dedupe {
			Dedupe::Strict => 64,
			Dedupe::Relaxed => 8, // relaxed compares against every existing group
		}
		.min(total - self.cursor);
		for _ in 0..cells {
			let cell = self.cursor;
			let (tx, ty) = (cell % self.tiles_x, cell / self.tiles_x);
			let mut tile = [0u8; TILE_DATA_SIZE];
			for ry in 0..n {
				let src = (ty * n + ry) * self.tw + tx * n;
				tile[ry * n..ry * n + n].copy_from_slice(&self.indices[src..src + n]);
			}
			match self.add_tile(&tile) {
				Ok(id) => self.bigmap.push(id),
				Err(e) => {
					self.error = Some(e);
					self.phase = Phase::Done;
					return cells * TILE_DATA_SIZE;
				}
			}
			self.cursor += 1;
		}
		if self.cursor >= total {
			self.phase = Phase::Done;
		}
		cells * TILE_DATA_SIZE * self.groups.len().max(1)
	}

	/// Intern a tile, returning its id. Strict dedupe collapses byte-identical
	/// tiles; relaxed merges a tile into an existing group only when it
	/// uses the **same colours** (within a small margin), its **edge/wall ring**
	/// matches the group's (so a shore isn't merged with ground, nor two shores
	/// facing different ways), and the body is within the threshold — and the
	/// group's representative becomes the per-pixel **median (mode)** of its
	/// members.
	fn add_tile(&mut self, tile: &[u8; TILE_DATA_SIZE]) -> Result<u16, String> {
		if self.opts.dedupe == Dedupe::Strict {
			if let Some(&id) = self.exact.get(tile) {
				return Ok(id);
			}
		} else if let Some(id) = self.merge_relaxed(tile) {
			return Ok(id);
		}
		let count = self.tiles.len() / TILE_DATA_SIZE;
		if count > u16::MAX as usize {
			return Err("image too detailed: over 65535 unique tiles".into());
		}
		let id = count as u16;
		self.tiles.extend_from_slice(tile);
		match self.opts.dedupe {
			Dedupe::Strict => {
				self.exact.insert(*tile, id);
			}
			Dedupe::Relaxed => self.groups.push(Group {
				members: vec![*tile],
				colors: distinct_colors(tile),
				mean: tile_mean(tile, &self.pal_rgb),
			}),
		}
		Ok(id)
	}

	/// Try to merge `tile` into an existing relaxed group; on success update that
	/// group's representative to the per-pixel median of its members and return
	/// its id.
	fn merge_relaxed(&mut self, tile: &[u8; TILE_DATA_SIZE]) -> Option<u16> {
		let margin = self.opts.threshold * MAX_PX_DIST;
		let colors = distinct_colors(tile);
		let mean = tile_mean(tile, &self.pal_rgb);
		let budget = margin * TILE_DATA_SIZE as f32;
		for gid in 0..self.groups.len() {
			// Mean-colour prefilter: dist(means) ≤ mean-of-distances (a far mean
			// can never satisfy the body check) — no false skips.
			if dist_sq(&mean, &self.groups[gid].mean).sqrt() > margin {
				continue;
			}
			// Same colours (within the margin) — never mixes distinct palettes.
			if !colors_compatible(&colors, &self.groups[gid].colors, &self.pal_rgb, margin) {
				continue;
			}
			let rep = &self.tiles[gid * TILE_DATA_SIZE..(gid + 1) * TILE_DATA_SIZE];
			// Matching edge/wall ring (orientation) + overall body similarity.
			if edge_ring_dist(tile, rep, &self.pal_rgb) > margin {
				continue;
			}
			if !tile_sum_dist_below(tile, rep, &self.pal_rgb, budget) {
				continue;
			}
			// Merge: recompute the representative as the members' per-pixel mode.
			self.groups[gid].members.push(*tile);
			let rep = per_pixel_mode(&self.groups[gid].members);
			self.tiles[gid * TILE_DATA_SIZE..(gid + 1) * TILE_DATA_SIZE].copy_from_slice(&rep);
			self.groups[gid].colors = distinct_colors(&rep);
			self.groups[gid].mean = tile_mean(&rep, &self.pal_rgb);
			return Some(gid as u16);
		}
		None
	}
}

/// A relaxed-dedupe tile group: its member tiles (for the median) + cached
/// signature of the current representative (`tiles[group id]`).
struct Group {
	members: Vec<[u8; TILE_DATA_SIZE]>,
	colors: Vec<u8>,
	mean: [f32; 3],
}

/// Run-to-completion convenience: import a square-/64-aligned RGBA image with
/// default settings (1:1, exact dedupe). The modal path uses [`ConvertSession`].
pub fn image_to_wrl(rgba: &[u8], width: u32, height: u32) -> Result<WrlFile, String> {
	let n = TILE_SIZE as u32;
	if width == 0 || height == 0 || !width.is_multiple_of(n) || !height.is_multiple_of(n) {
		return Err(format!("image {width}×{height}: each side must be a non-zero multiple of {n}"));
	}
	if width / n > 1024 || height / n > 1024 {
		return Err(format!("image yields a {}×{}-cell map (max 1024×1024)", width / n, height / n));
	}
	let mut s = ConvertSession::new(rgba.to_vec(), width, height, ConvertOpts::fit_source(width, height))?;
	while !s.is_done() {
		s.step(usize::MAX);
	}
	s.finish()
}

fn key(c: [f32; 3]) -> u32 {
	(c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32
}

fn add3(buf: &mut [f32], k: usize, err: [f32; 3], f: f32) {
	buf[k] += err[0] * f;
	buf[k + 1] += err[1] * f;
	buf[k + 2] += err[2] * f;
}

fn dist_sq(a: &[f32; 3], b: &[f32; 3]) -> f32 {
	let (d0, d1, d2) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
	d0 * d0 + d1 * d1 + d2 * d2
}

fn nearest(c: &[f32; 3], centroids: &[[f32; 3]]) -> usize {
	let mut best = 0;
	let mut best_d = f32::MAX;
	for (i, k) in centroids.iter().enumerate() {
		let d = dist_sq(c, k);
		if d < best_d {
			best_d = d;
			best = i;
		}
	}
	best
}

/// Mean resolved RGB of an indexed tile.
fn tile_mean(tile: &[u8], pal_rgb: &[[f32; 3]]) -> [f32; 3] {
	let mut s = [0.0f32; 3];
	for &i in tile {
		let c = pal_rgb[i as usize];
		s[0] += c[0];
		s[1] += c[1];
		s[2] += c[2];
	}
	let n = tile.len() as f32;
	[s[0] / n, s[1] / n, s[2] / n]
}

/// Whether the summed per-pixel colour distance between two indexed tiles stays
/// below `budget` (early-outs as soon as it's exceeded).
fn tile_sum_dist_below(a: &[u8], b: &[u8], pal_rgb: &[[f32; 3]], budget: f32) -> bool {
	let mut sum = 0.0f32;
	for (&ia, &ib) in a.iter().zip(b.iter()) {
		if ia != ib {
			sum += dist_sq(&pal_rgb[ia as usize], &pal_rgb[ib as usize]).sqrt();
			if sum > budget {
				return false;
			}
		}
	}
	true
}

/// Sorted distinct palette indices used by a tile (its colour set).
fn distinct_colors(tile: &[u8]) -> Vec<u8> {
	let mut seen = [false; 256];
	for &i in tile {
		seen[i as usize] = true;
	}
	(0u16..256).filter(|&i| seen[i as usize]).map(|i| i as u8).collect()
}

/// Whether two colour sets are the "same" within `margin`: every colour in each
/// has a counterpart in the other within `margin`. Gates relaxed merges so
/// tiles built from different palettes (shore vs ground) never collapse.
fn colors_compatible(a: &[u8], b: &[u8], pal_rgb: &[[f32; 3]], margin: f32) -> bool {
	let covered = |xs: &[u8], ys: &[u8]| {
		xs.iter().all(|&x| ys.iter().any(|&y| dist_sq(&pal_rgb[x as usize], &pal_rgb[y as usize]).sqrt() <= margin))
	};
	covered(a, b) && covered(b, a)
}

/// Mean colour distance over the tile's outer ring (its edges / "walls"). A
/// shore facing N and one facing E have different rings, so this keeps
/// differently-oriented structural tiles from merging.
fn edge_ring_dist(a: &[u8], b: &[u8], pal_rgb: &[[f32; 3]]) -> f32 {
	let n = TILE_SIZE;
	let mut sum = 0.0f32;
	let mut at = |i: usize| {
		sum += dist_sq(&pal_rgb[a[i] as usize], &pal_rgb[b[i] as usize]).sqrt();
	};
	for k in 0..n {
		at(k); // top row
		at((n - 1) * n + k); // bottom row
		at(k * n); // left column
		at(k * n + n - 1); // right column
	}
	sum / (4 * n) as f32
}

/// Per-pixel median (mode) across a group's member tiles — the relaxed-dedupe
/// representative. Ties break to the lowest index (deterministic).
fn per_pixel_mode(members: &[[u8; TILE_DATA_SIZE]]) -> [u8; TILE_DATA_SIZE] {
	if members.len() == 1 {
		return members[0];
	}
	let mut out = [0u8; TILE_DATA_SIZE];
	let mut counts = [0u16; 256];
	for (p, slot) in out.iter_mut().enumerate() {
		for m in members {
			counts[m[p] as usize] += 1;
		}
		let (mut best, mut best_c) = (members[0][p], 0u16);
		for m in members {
			let i = m[p];
			let c = counts[i as usize];
			if c > best_c || (c == best_c && i < best) {
				best = i;
				best_c = c;
			}
		}
		for m in members {
			counts[m[p] as usize] = 0; // reset only touched entries
		}
		*slot = best;
	}
	out
}

fn round_u8(v: f32) -> u8 {
	v.round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
	use super::*;

	fn solid_blocks(tiles_x: u32, tiles_y: u32, block: impl Fn(u32, u32) -> [u8; 3]) -> (Vec<u8>, u32, u32) {
		let (w, h) = (tiles_x * 64, tiles_y * 64);
		let mut rgba = vec![0u8; (w * h) as usize * 4];
		for y in 0..h {
			for x in 0..w {
				let c = block(x / 64, y / 64);
				let i = (y * w + x) as usize * 4;
				rgba[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
			}
		}
		(rgba, w, h)
	}

	fn run(rgba: &[u8], w: u32, h: u32, opts: ConvertOpts) -> WrlFile {
		let mut s = ConvertSession::new(rgba.to_vec(), w, h, opts).unwrap();
		while !s.is_done() {
			s.step(50_000);
		}
		s.finish().unwrap()
	}

	#[test]
	fn rejects_non_multiple_of_64() {
		let rgba = vec![0u8; 100 * 64 * 4];
		assert!(image_to_wrl(&rgba, 100, 64).is_err());
	}

	#[test]
	fn dedups_identical_tiles() {
		let (rgba, w, h) = solid_blocks(2, 1, |_, _| [10, 200, 40]);
		let wrl = image_to_wrl(&rgba, w, h).unwrap();
		assert_eq!((wrl.width, wrl.height), (2, 1));
		assert_eq!(wrl.tile_count, 1);
		assert_eq!(wrl.bigmap, vec![0, 0]);
	}

	#[test]
	fn distinct_blocks_make_distinct_tiles() {
		let (rgba, w, h) = solid_blocks(2, 1, |tx, _| if tx == 0 { [200, 30, 30] } else { [30, 30, 200] });
		let wrl = image_to_wrl(&rgba, w, h).unwrap();
		assert_eq!(wrl.tile_count, 2);
		assert_eq!(wrl.bigmap, vec![0, 1]);
	}

	#[test]
	fn preserves_statics_and_never_emits_animated() {
		let (rgba, w, h) = solid_blocks(2, 2, |tx, ty| [(tx * 120) as u8 + 30, (ty * 90) as u8 + 40, 100]);
		let wrl = image_to_wrl(&rgba, w, h).unwrap();
		assert_eq!(&wrl.palette[0..3], &GAME_PALETTE[0..3]);
		assert_eq!(&wrl.palette[200 * 3..200 * 3 + 3], &GAME_PALETTE[200 * 3..200 * 3 + 3]);
		assert!(wrl.tiles.iter().all(|&i| !is_animated(i)), "animated slots must never be emitted");
	}

	#[test]
	fn resamples_to_chosen_dimensions() {
		// 2×2 source blocks → request a 4×4-tile map (upscaled by Stretch).
		let (rgba, w, h) = solid_blocks(2, 2, |tx, ty| [(tx * 150) as u8 + 20, (ty * 150) as u8 + 20, 90]);
		let opts = ConvertOpts { width_tiles: 4, height_tiles: 4, ..ConvertOpts::fit_source(w, h) };
		let wrl = run(&rgba, w, h, opts);
		assert_eq!((wrl.width, wrl.height), (4, 4));
		assert_eq!(wrl.bigmap.len(), 16);
	}

	#[test]
	fn relaxed_dedupe_collapses_near_tiles() {
		// Two blocks that differ by one unit per channel: strict keeps both,
		// a relaxed threshold collapses them (same colours, matching edges).
		let (rgba, w, h) = solid_blocks(2, 1, |tx, _| if tx == 0 { [100, 100, 100] } else { [102, 101, 100] });
		let strict = run(&rgba, w, h, ConvertOpts::fit_source(w, h));
		assert_eq!(strict.tile_count, 2);
		let relaxed = ConvertOpts { dedupe: Dedupe::Relaxed, threshold: 0.2, ..ConvertOpts::fit_source(w, h) };
		let merged = run(&rgba, w, h, relaxed);
		assert_eq!(merged.tile_count, 1, "near-identical tiles collapse");
		assert_eq!(merged.bigmap, vec![0, 0]);
	}

	#[test]
	fn relaxed_keeps_distinct_colours_apart() {
		// Red vs blue: at a small threshold the colour-set gate keeps them
		// separate even under relaxed dedupe (no shore/ground mixing).
		let (rgba, w, h) = solid_blocks(2, 1, |tx, _| if tx == 0 { [200, 30, 30] } else { [30, 30, 200] });
		let opts = ConvertOpts { dedupe: Dedupe::Relaxed, threshold: 0.05, ..ConvertOpts::fit_source(w, h) };
		assert_eq!(run(&rgba, w, h, opts).tile_count, 2);
	}

	#[test]
	fn per_pixel_mode_takes_the_majority() {
		let a = [5u8; TILE_DATA_SIZE];
		let mut b = [5u8; TILE_DATA_SIZE];
		b[0] = 9;
		let mut c = [5u8; TILE_DATA_SIZE];
		c[0] = 9;
		let mode = per_pixel_mode(&[a, b, c]);
		assert_eq!(mode[0], 9, "2 of 3 members chose 9");
		assert_eq!(mode[1], 5);
	}

	#[test]
	fn colour_set_and_edge_gates() {
		let pal: Vec<[f32; 3]> = (0..256).map(|i| [i as f32, 0.0, 0.0]).collect();
		// Same colours within margin; far colours rejected; uncovered rejected.
		assert!(colors_compatible(&[10, 20], &[11, 21], &pal, 5.0));
		assert!(!colors_compatible(&[10], &[200], &pal, 5.0));
		assert!(!colors_compatible(&[10, 20], &[10], &pal, 5.0));
		// A tile with a coloured top wall differs from a flat tile on the ring.
		let flat = [0u8; TILE_DATA_SIZE];
		let mut wall = [0u8; TILE_DATA_SIZE];
		wall[..TILE_SIZE].fill(100);
		assert_eq!(edge_ring_dist(&flat, &flat, &pal), 0.0);
		assert!(edge_ring_dist(&wall, &flat, &pal) > 10.0, "edge ring catches the wall");
	}

	#[test]
	fn progress_advances_monotonically_to_done() {
		let (rgba, w, h) = solid_blocks(2, 2, |tx, ty| [(tx * 80) as u8, (ty * 80) as u8, 60]);
		let mut s = ConvertSession::new(rgba, w, h, ConvertOpts::fit_source(w, h)).unwrap();
		let mut last = 0.0;
		let mut steps = 0;
		while !s.is_done() {
			s.step(20_000);
			let p = s.progress();
			assert!(p >= last - 1e-6, "progress must not go backwards");
			last = p;
			steps += 1;
			assert!(steps < 100_000, "must terminate");
		}
		assert_eq!(s.progress(), 1.0);
		assert!(s.finish().is_ok());
	}
}
