//! A minimal TrueType (`glyf`) rasterizer - just enough to render the MAX UI
//! font (`assets/max_square.ttf`) to coverage atlases at runtime, so each UI
//! size (and UI-scale size) is rasterized at its **native** pixel size rather
//! than box-downsampled from one 60-px master (which softens sizes that aren't
//! clean divisors). Dependency-free: it parses only the tables we need
//! (`head`/`maxp`/`hhea`/`hmtx`/`cmap`/`loca`/`glyf`), reconstructs the
//! quadratic outlines, and fills them with an anti-aliased scanline rasterizer.
//!
//! Scope: printable ASCII via a format-4 `cmap`, simple **and** composite
//! `glyf` outlines. It is not a general font engine - no hinting, kerning, or
//! CFF/PostScript outlines (the MAX fonts are all `glyf`).

// ----- big-endian primitives ------------------------------------------------

fn rd_u16(d: &[u8], o: usize) -> u16 {
	u16::from_be_bytes([d[o], d[o + 1]])
}
fn rd_i16(d: &[u8], o: usize) -> i16 {
	i16::from_be_bytes([d[o], d[o + 1]])
}
fn rd_u32(d: &[u8], o: usize) -> u32 {
	u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}
/// A 2.14 fixed-point value (composite-glyph transforms).
fn rd_f2dot14(d: &[u8], o: usize) -> f32 {
	rd_i16(d, o) as f32 / 16384.0
}

/// One contour's points in font units, each flagged on-curve / off-curve
/// (control). A glyph is a set of these closed contours.
type Contour = Vec<(f32, f32, bool)>;

/// A line segment `[x0, y0, x1, y1]` in pixel space (after flattening).
type Edge = [f32; 4];

/// A parsed TrueType font (borrows the file bytes).
pub struct Font<'a> {
	data: &'a [u8],
	loca_long: bool,
	num_h_metrics: usize,
	loca: usize,
	glyf: usize,
	hmtx: usize,
	cmap4: usize, // offset of the chosen format-4 subtable (0 = none)
}

impl<'a> Font<'a> {
	/// Parse the table directory and the handful of tables we render from.
	pub fn parse(data: &'a [u8]) -> Font<'a> {
		let num_tables = rd_u16(data, 4) as usize;
		let table = |tag: &[u8; 4]| -> usize {
			(0..num_tables)
				.map(|i| 12 + i * 16)
				.find(|&o| &data[o..o + 4] == tag)
				.map_or(0, |o| rd_u32(data, o + 8) as usize)
		};
		let head = table(b"head");
		let hhea = table(b"hhea");
		let cmap = table(b"cmap");
		// Prefer a Unicode BMP format-4 subtable (platform 3/1, 3/0, or 0/*).
		let mut cmap4 = 0;
		let nsub = rd_u16(data, cmap + 2) as usize;
		for i in 0..nsub {
			let o = cmap + 4 + i * 8;
			let (pid, eid) = (rd_u16(data, o), rd_u16(data, o + 2));
			let sub = cmap + rd_u32(data, o + 4) as usize;
			if rd_u16(data, sub) == 4 && (pid == 0 || (pid == 3 && (eid == 0 || eid == 1))) {
				cmap4 = sub;
			}
		}
		Font {
			data,
			loca_long: rd_i16(data, head + 50) == 1,
			num_h_metrics: rd_u16(data, hhea + 34) as usize,
			loca: table(b"loca"),
			glyf: table(b"glyf"),
			hmtx: table(b"hmtx"),
			cmap4,
		}
	}

	/// Glyph id for a character via the format-4 `cmap` (0 = `.notdef`).
	pub fn glyph_index(&self, ch: char) -> u16 {
		let cp = ch as u32;
		let (d, o) = (self.data, self.cmap4);
		if o == 0 || cp > 0xffff {
			return 0;
		}
		let cp = cp as u16;
		let seg = rd_u16(d, o + 6) as usize / 2;
		let end = o + 14;
		let start = end + seg * 2 + 2;
		let delta = start + seg * 2;
		let range = delta + seg * 2;
		for s in 0..seg {
			if cp <= rd_u16(d, end + s * 2) {
				if cp < rd_u16(d, start + s * 2) {
					return 0;
				}
				let id_delta = rd_u16(d, delta + s * 2);
				let ro = rd_u16(d, range + s * 2) as usize;
				if ro == 0 {
					return cp.wrapping_add(id_delta);
				}
				let gi = rd_u16(d, range + s * 2 + ro + (cp - rd_u16(d, start + s * 2)) as usize * 2);
				return if gi == 0 { 0 } else { gi.wrapping_add(id_delta) };
			}
		}
		0
	}

	/// Advance width (font units) from `hmtx` (last metric repeats for the tail).
	pub fn advance_width(&self, gid: u16) -> u16 {
		let g = (gid as usize).min(self.num_h_metrics.saturating_sub(1));
		rd_u16(self.data, self.hmtx + g * 4)
	}

	/// Byte range of glyph `gid` inside the `glyf` table (start == end → no
	/// outline, e.g. the space).
	fn glyph_span(&self, gid: u16) -> (usize, usize) {
		let (d, g) = (self.data, gid as usize);
		if self.loca_long {
			(rd_u32(d, self.loca + g * 4) as usize, rd_u32(d, self.loca + (g + 1) * 4) as usize)
		} else {
			(rd_u16(d, self.loca + g * 2) as usize * 2, rd_u16(d, self.loca + (g + 1) * 2) as usize * 2)
		}
	}

	/// The glyph's contours in font units (y-up). Recurses into components for
	/// composite glyphs; `depth` guards against cyclic references.
	pub fn outline(&self, gid: u16, depth: u8) -> Vec<Contour> {
		let (s, e) = self.glyph_span(gid);
		if s >= e || depth > 5 {
			return Vec::new();
		}
		let g = self.glyf + s;
		let nc = rd_i16(self.data, g);
		if nc >= 0 { self.simple_outline(g, nc as usize) } else { self.composite_outline(g, depth) }
	}

	fn simple_outline(&self, g: usize, nc: usize) -> Vec<Contour> {
		let d = self.data;
		let mut o = g + 10; // numberOfContours (2) + bbox (8)
		let mut ends = vec![0usize; nc];
		for slot in ends.iter_mut() {
			*slot = rd_u16(d, o) as usize;
			o += 2;
		}
		let n = ends.last().map_or(0, |&e| e + 1);
		o += 2 + rd_u16(d, o) as usize; // skip instructions
		// Flags (with the repeat-run encoding).
		let mut flags = Vec::with_capacity(n);
		while flags.len() < n {
			let f = d[o];
			o += 1;
			flags.push(f);
			if f & 0x08 != 0 {
				let r = d[o];
				o += 1;
				flags.extend(std::iter::repeat_n(f, r as usize));
			}
		}
		// X then Y as deltas (short = 1 byte + sign flag; else 2 bytes or "same").
		let mut xs = vec![0i32; n];
		let mut acc = 0i32;
		for i in 0..n {
			let f = flags[i];
			if f & 0x02 != 0 {
				let dx = d[o] as i32;
				o += 1;
				acc += if f & 0x10 != 0 { dx } else { -dx };
			} else if f & 0x10 == 0 {
				acc += rd_i16(d, o) as i32;
				o += 2;
			}
			xs[i] = acc;
		}
		let mut ys = vec![0i32; n];
		acc = 0;
		for i in 0..n {
			let f = flags[i];
			if f & 0x04 != 0 {
				let dy = d[o] as i32;
				o += 1;
				acc += if f & 0x20 != 0 { dy } else { -dy };
			} else if f & 0x20 == 0 {
				acc += rd_i16(d, o) as i32;
				o += 2;
			}
			ys[i] = acc;
		}
		// Split the flat point list into the contours.
		let mut contours = Vec::with_capacity(nc);
		let mut start = 0;
		for &end in &ends {
			let pts = (start..=end).map(|i| (xs[i] as f32, ys[i] as f32, flags[i] & 0x01 != 0)).collect();
			contours.push(pts);
			start = end + 1;
		}
		contours
	}

	fn composite_outline(&self, g: usize, depth: u8) -> Vec<Contour> {
		let d = self.data;
		let mut o = g + 10;
		let mut contours = Vec::new();
		loop {
			let flags = rd_u16(d, o);
			let comp = rd_u16(d, o + 2);
			o += 4;
			// ARGS_ARE_XY_VALUES (0x0002) is assumed; point-matching is ignored.
			let (dx, dy) = if flags & 0x0001 != 0 {
				let v = (rd_i16(d, o) as f32, rd_i16(d, o + 2) as f32);
				o += 4;
				v
			} else {
				let v = (d[o] as i8 as f32, d[o + 1] as i8 as f32);
				o += 2;
				v
			};
			let (mut a, mut b, mut c, mut e) = (1.0f32, 0.0, 0.0, 1.0);
			if flags & 0x0008 != 0 {
				a = rd_f2dot14(d, o);
				e = a;
				o += 2;
			} else if flags & 0x0040 != 0 {
				a = rd_f2dot14(d, o);
				e = rd_f2dot14(d, o + 2);
				o += 4;
			} else if flags & 0x0080 != 0 {
				a = rd_f2dot14(d, o);
				b = rd_f2dot14(d, o + 2);
				c = rd_f2dot14(d, o + 4);
				e = rd_f2dot14(d, o + 6);
				o += 8;
			}
			for cont in self.outline(comp, depth + 1) {
				contours
					.push(cont.into_iter().map(|(x, y, on)| (a * x + c * y + dx, b * x + e * y + dy, on)).collect());
			}
			if flags & 0x0020 == 0 {
				break; // no MORE_COMPONENTS
			}
		}
		contours
	}
}

/// Flatten one closed contour (points in **pixel** space, on/off-curve flagged)
/// into line-segment edges. Implied on-curve points between consecutive control
/// points are reconstructed per the TrueType convention.
pub fn flatten(pts: &[(f32, f32, bool)], edges: &mut Vec<Edge>) {
	let n = pts.len();
	if n < 2 {
		return;
	}
	// Walk from an on-curve anchor: the first/last on-curve point, or - for an
	// all-off-curve contour - the synthesized midpoint of the first and last.
	let mid = |p: (f32, f32, bool), q: (f32, f32, bool)| ((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0);
	let (anchor, seq): (_, &[(f32, f32, bool)]) = if pts[0].2 {
		((pts[0].0, pts[0].1), &pts[1..])
	} else if pts[n - 1].2 {
		((pts[n - 1].0, pts[n - 1].1), &pts[..n - 1])
	} else {
		(mid(pts[0], pts[n - 1]), pts)
	};
	let (mut px, mut py) = anchor;
	let mut ctrl: Option<(f32, f32)> = None;
	let line_or_curve =
		|x: f32, y: f32, ctrl: &mut Option<(f32, f32)>, px: &mut f32, py: &mut f32, edges: &mut Vec<Edge>| {
			match ctrl.take() {
				None => edges.push([*px, *py, x, y]),
				Some(c) => quad(*px, *py, c.0, c.1, x, y, edges),
			}
			*px = x;
			*py = y;
		};
	for &(x, y, on) in seq {
		if on {
			line_or_curve(x, y, &mut ctrl, &mut px, &mut py, edges);
		} else if let Some(c) = ctrl {
			// Two control points in a row: split at their implied midpoint.
			let m = mid((c.0, c.1, false), (x, y, false));
			quad(px, py, c.0, c.1, m.0, m.1, edges);
			px = m.0;
			py = m.1;
			ctrl = Some((x, y));
		} else {
			ctrl = Some((x, y));
		}
	}
	// Close back to the anchor.
	line_or_curve(anchor.0, anchor.1, &mut ctrl, &mut px, &mut py, edges);
}

/// Flatten a quadratic Bézier into fixed line segments (enough for UI sizes).
fn quad(x0: f32, y0: f32, cx: f32, cy: f32, x1: f32, y1: f32, edges: &mut Vec<Edge>) {
	const STEPS: usize = 8;
	let (mut px, mut py) = (x0, y0);
	for i in 1..=STEPS {
		let t = i as f32 / STEPS as f32;
		let mt = 1.0 - t;
		let x = mt * mt * x0 + 2.0 * mt * t * cx + t * t * x1;
		let y = mt * mt * y0 + 2.0 * mt * t * cy + t * t * y1;
		edges.push([px, py, x, y]);
		px = x;
		py = y;
	}
}

/// Fill `edges` (pixel space) into a `w`×`h` 8-bit coverage bitmap with the
/// nonzero winding rule. Anti-aliased by sampling [`SS`] sub-scanlines per row
/// and accumulating exact horizontal span coverage (sub-pixel endpoints).
pub fn fill(edges: &[Edge], w: usize, h: usize) -> Vec<u8> {
	const SS: usize = 4;
	let mut cov = vec![0f32; w * h];
	let mut xs: Vec<(f32, i32)> = Vec::new();
	for py in 0..h {
		let row = &mut cov[py * w..py * w + w];
		for s in 0..SS {
			let sy = py as f32 + (s as f32 + 0.5) / SS as f32;
			xs.clear();
			for &[x0, y0, x1, y1] in edges {
				let (lo, hi) = (y0.min(y1), y0.max(y1));
				if sy >= lo && sy < hi {
					let x = x0 + (sy - y0) / (y1 - y0) * (x1 - x0);
					xs.push((x, if y1 > y0 { 1 } else { -1 }));
				}
			}
			if xs.len() < 2 {
				continue;
			}
			xs.sort_by(|a, b| a.0.total_cmp(&b.0));
			// Between consecutive crossings, fill where the running winding != 0.
			let mut wind = 0;
			for k in 0..xs.len() - 1 {
				wind += xs[k].1;
				if wind != 0 {
					add_span(row, xs[k].0, xs[k + 1].0, 1.0 / SS as f32);
				}
			}
		}
	}
	cov.iter().map(|&c| (c.clamp(0.0, 1.0) * 255.0).round() as u8).collect()
}

/// Add `weight` coverage over `[xa, xb)` to a row, with fractional coverage on
/// the partially-covered end pixels.
fn add_span(row: &mut [f32], xa: f32, xb: f32, weight: f32) {
	let w = row.len() as f32;
	let (xa, xb) = (xa.max(0.0), xb.min(w));
	if xb <= xa {
		return;
	}
	let (ia, ib) = (xa.floor() as usize, (xb.ceil() as usize).min(row.len()));
	for (px, slot) in row.iter_mut().enumerate().take(ib).skip(ia) {
		let l = (px as f32).max(xa);
		let r = (px as f32 + 1.0).min(xb);
		if r > l {
			*slot += weight * (r - l);
		}
	}
}
