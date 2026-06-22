//! Palette texture + color-cycling.
//!
//! MAX's water shimmer / plasma / energy glows are driven by rotating
//! contiguous ranges of palette entries at independent rates. Each range
//! keeps its own "last-advanced" timestamp so the cadence is stable under
//! variable frame times.
//!
//! Ranges and rates are taken from the reference implementation in
//! `wgl-map.ts::COLOR_CYCLE_RANGES`.

#[derive(Debug, Clone, Copy)]
pub enum CycleDirection {
	/// Shift colors so the first entry wraps to the last - "dark to light".
	Backward,
	/// Shift colors so the last entry wraps to the first - "light to dark".
	Forward,
}

#[derive(Debug, Clone, Copy)]
pub struct ColorCycleRange {
	pub start: u8,
	pub end: u8,
	pub direction: CycleDirection,
	pub fps: f32,
}

/// The 11 ranges used by original MAX, ported from the JS reference.
pub const DEFAULT_RANGES: &[ColorCycleRange] = &[
	// Water shimmer (9..=31)
	ColorCycleRange { start: 9, end: 12, direction: CycleDirection::Backward, fps: 9.0 },
	ColorCycleRange { start: 13, end: 16, direction: CycleDirection::Forward, fps: 6.0 },
	ColorCycleRange { start: 17, end: 20, direction: CycleDirection::Forward, fps: 9.0 },
	ColorCycleRange { start: 21, end: 24, direction: CycleDirection::Forward, fps: 6.0 },
	ColorCycleRange { start: 25, end: 30, direction: CycleDirection::Forward, fps: 2.0 },
	ColorCycleRange { start: 31, end: 31, direction: CycleDirection::Forward, fps: 6.0 },
	// Special effects (96..=127)
	ColorCycleRange { start: 96, end: 102, direction: CycleDirection::Forward, fps: 8.0 },
	ColorCycleRange { start: 103, end: 109, direction: CycleDirection::Forward, fps: 8.0 },
	ColorCycleRange { start: 110, end: 116, direction: CycleDirection::Forward, fps: 10.0 },
	ColorCycleRange { start: 117, end: 122, direction: CycleDirection::Forward, fps: 6.0 },
	ColorCycleRange { start: 123, end: 127, direction: CycleDirection::Forward, fps: 6.0 },
];

/// Holds the 256-entry working palette in RGBA8 and cycles ranges on `tick`.
pub struct PaletteCycler {
	rgba: Vec<u8>,
	ranges: Vec<ColorCycleRange>,
	last_advance_s: Vec<f32>,
	dirty: bool,
	/// In-Game render mode: the output palette is quantized to 6-bit
	/// channels (low 2 bits zeroed), matching what the original engine displays.
	ingame: bool,
	/// Scratch buffer for the quantized output (only used when `ingame`).
	masked: Vec<u8>,
}

impl PaletteCycler {
	/// `rgb` must be 256*3 bytes; alpha is set opaque for every entry except
	/// index 0 (treated transparent by the original art pipeline).
	pub fn from_rgb(rgb: &[u8]) -> Self {
		assert_eq!(rgb.len(), 256 * 3, "palette must be 256*3 bytes");
		let mut rgba = Vec::with_capacity(256 * 4);
		for (i, chunk) in rgb.chunks_exact(3).enumerate() {
			rgba.extend_from_slice(&[chunk[0], chunk[1], chunk[2]]);
			rgba.push(if i == 0 { 0 } else { 255 });
		}
		let ranges = DEFAULT_RANGES.to_vec();
		let last_advance_s = vec![0.0; ranges.len()];
		Self { rgba, ranges, last_advance_s, dirty: true, ingame: false, masked: Vec::new() }
	}

	/// `bgra` must be 256*4 bytes laid out as `[B, G, R, A]` per entry -
	/// MAX's FRAMEPIC palette convention. The sampler expects RGBA, so each
	/// entry's blue/red bytes are swapped on the way in. Index 0 alpha is
	/// forced to 0 to match the transparent-slot contract.
	/// (Unused here so far - kept in sync with the re-MAX original.)
	#[allow(dead_code)]
	pub fn from_bgra(bgra: &[u8]) -> Self {
		assert_eq!(bgra.len(), 256 * 4, "palette must be 256*4 bgra bytes");
		let mut rgba = Vec::with_capacity(256 * 4);
		for (i, chunk) in bgra.chunks_exact(4).enumerate() {
			rgba.extend_from_slice(&[chunk[2], chunk[1], chunk[0]]);
			rgba.push(if i == 0 { 0 } else { chunk[3] });
		}
		let ranges = DEFAULT_RANGES.to_vec();
		let last_advance_s = vec![0.0; ranges.len()];
		Self { rgba, ranges, last_advance_s, dirty: true, ingame: false, masked: Vec::new() }
	}

	/// The live working palette (true colours, not quantized).
	pub fn rgba(&self) -> &[u8] {
		&self.rgba
	}

	/// Toggle In-Game quantization. Forces the next [`take_if_dirty`] so the
	/// GPU palette re-uploads with/without the 6-bit mask.
	pub fn set_ingame(&mut self, on: bool) {
		if self.ingame != on {
			self.ingame = on;
			self.dirty = true;
		}
	}

	pub fn tick(&mut self, time_s: f32) {
		for (i, range) in self.ranges.iter().enumerate() {
			let interval = 1.0 / range.fps;
			if time_s - self.last_advance_s[i] >= interval {
				cycle_in_place(&mut self.rgba, range.start, range.end, range.direction);
				self.last_advance_s[i] = time_s;
				self.dirty = true;
			}
		}
	}

	/// Returns the palette bytes if they've changed since the last call. In
	/// In-Game mode each RGB channel is quantized to 6 bits (low 2 bits zeroed),
	/// matching the engine's output; alpha is left intact.
	pub fn take_if_dirty(&mut self) -> Option<&[u8]> {
		if !self.dirty {
			return None;
		}
		self.dirty = false;
		if self.ingame {
			self.masked.clear();
			self.masked.extend(self.rgba.iter().enumerate().map(|(i, &b)| if i % 4 == 3 { b } else { b & 0xFC }));
			Some(&self.masked)
		} else {
			Some(&self.rgba)
		}
	}
}

fn cycle_in_place(palette: &mut [u8], start: u8, end: u8, direction: CycleDirection) {
	let s = start as usize * 4;
	let e = end as usize * 4;
	match direction {
		CycleDirection::Forward => {
			let last = [palette[e], palette[e + 1], palette[e + 2], palette[e + 3]];
			let mut i = e;
			while i > s {
				palette[i] = palette[i - 4];
				palette[i + 1] = palette[i - 3];
				palette[i + 2] = palette[i - 2];
				palette[i + 3] = palette[i - 1];
				i -= 4;
			}
			palette[s..s + 4].copy_from_slice(&last);
		}
		CycleDirection::Backward => {
			let first = [palette[s], palette[s + 1], palette[s + 2], palette[s + 3]];
			let mut i = s;
			while i < e {
				palette[i] = palette[i + 4];
				palette[i + 1] = palette[i + 5];
				palette[i + 2] = palette[i + 6];
				palette[i + 3] = palette[i + 7];
				i += 4;
			}
			palette[e..e + 4].copy_from_slice(&first);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn mk_palette() -> Vec<u8> {
		// Encode each slot's RGBA as (i, i, i, 255) so we can check movement by index.
		let mut p = vec![0; 256 * 4];
		for i in 0..256 {
			p[i * 4] = i as u8;
			p[i * 4 + 1] = i as u8;
			p[i * 4 + 2] = i as u8;
			p[i * 4 + 3] = 255;
		}
		p
	}

	#[test]
	fn forward_wraps_last_into_first() {
		let mut p = mk_palette();
		cycle_in_place(&mut p, 10, 13, CycleDirection::Forward);
		assert_eq!(p[10 * 4], 13, "slot 10 should now hold former slot 13");
		assert_eq!(p[11 * 4], 10);
		assert_eq!(p[12 * 4], 11);
		assert_eq!(p[13 * 4], 12);
	}

	#[test]
	fn backward_wraps_first_into_last() {
		let mut p = mk_palette();
		cycle_in_place(&mut p, 10, 13, CycleDirection::Backward);
		assert_eq!(p[10 * 4], 11);
		assert_eq!(p[11 * 4], 12);
		assert_eq!(p[12 * 4], 13);
		assert_eq!(p[13 * 4], 10, "slot 13 should hold former slot 10");
	}

	#[test]
	fn ingame_quantizes_rgb_but_keeps_alpha() {
		// In-Game output masks each RGB channel to 6 bits; alpha untouched.
		let mut rgb = vec![0u8; 256 * 3];
		for i in 0..256 {
			rgb[i * 3] = 0xff;
			rgb[i * 3 + 1] = 0x7d;
			rgb[i * 3 + 2] = 0x03;
		}
		let mut c = PaletteCycler::from_rgb(&rgb);
		c.set_ingame(true);
		let out = c.take_if_dirty().expect("ingame toggle marks dirty");
		assert_eq!(&out[4..8], &[0xfc, 0x7c, 0x00, 0xff], "slot 1 RGB→6-bit, alpha intact");
		assert!(out.iter().enumerate().all(|(i, &b)| i % 4 == 3 || b & 0x03 == 0), "all RGB low 2 bits zero");
	}

	#[test]
	fn tick_advances_on_interval() {
		let mut rgb = vec![0u8; 256 * 3];
		for i in 0..256 {
			rgb[i * 3] = i as u8;
		}
		let mut c = PaletteCycler::from_rgb(&rgb);
		// Drain initial dirty flag.
		let _ = c.take_if_dirty();
		c.tick(0.001);
		assert!(c.take_if_dirty().is_none(), "no range should have advanced yet");

		// At t = 1s, the slowest range (2 fps -> 0.5s interval) should have advanced.
		c.tick(1.0);
		assert!(c.take_if_dirty().is_some());
	}
}
