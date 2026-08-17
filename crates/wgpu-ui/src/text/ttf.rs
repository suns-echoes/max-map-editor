//! A hand-rolled TrueType (`glyf`-outline) font parser.
//!
//! Scope: the sfnt container and the tables a UI renderer needs — `head`,
//! `maxp`, `hhea`/`hmtx`, `cmap` (formats 4 and 12), `loca`, `glyf` (simple and
//! composite glyphs), and pair kerning from GPOS (the `kern` feature's PairPos
//! lookups) with the legacy `kern` table (format 0) as fallback. PostScript/CFF
//! outlines (`.otf`), bytecode hinting, and complex shaping (GSUB, the
//! non-pair GPOS lookup types) are intentionally out of scope.
//!
//! All reads are bounds-checked: a malformed or truncated font yields a
//! [`FontError`] rather than a panic.

/// Errors from parsing or querying a font.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontError {
    /// The data is too short to contain the sfnt header.
    TooShort,
    /// Not a TrueType (`glyf`) font — e.g. a CFF/OpenType (`OTTO`) file.
    NotTrueType,
    /// A required table is missing.
    MissingTable(&'static str),
    /// No usable `cmap` subtable (need format 4 or 12).
    UnsupportedCmap,
    /// A read ran past the end of a table/the data.
    OutOfBounds,
}

impl std::fmt::Display for FontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort => write!(f, "font data too short"),
            Self::NotTrueType => write!(f, "not a TrueType (glyf) font (CFF/.otf is unsupported)"),
            Self::MissingTable(t) => write!(f, "missing required table `{t}`"),
            Self::UnsupportedCmap => write!(f, "no usable cmap subtable (need format 4 or 12)"),
            Self::OutOfBounds => write!(f, "read out of bounds (malformed font)"),
        }
    }
}

impl std::error::Error for FontError {}

// Bounds-checked big-endian readers.
fn ru8(d: &[u8], o: usize) -> Result<u8, FontError> {
    d.get(o).copied().ok_or(FontError::OutOfBounds)
}
fn ri8(d: &[u8], o: usize) -> Result<i8, FontError> {
    ru8(d, o).map(|b| b as i8)
}
fn ru16(d: &[u8], o: usize) -> Result<u16, FontError> {
    d.get(o..o + 2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .ok_or(FontError::OutOfBounds)
}
fn ri16(d: &[u8], o: usize) -> Result<i16, FontError> {
    ru16(d, o).map(|v| v as i16)
}
fn ru32(d: &[u8], o: usize) -> Result<u32, FontError> {
    d.get(o..o + 4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or(FontError::OutOfBounds)
}

/// A 2x3 affine transform (used for composite-glyph components).
#[derive(Clone, Copy)]
struct Affine {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl Affine {
    const ID: Affine = Affine {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    /// `self ∘ inner`: apply `inner` first, then `self`.
    fn compose(&self, inner: &Affine) -> Affine {
        Affine {
            a: self.a * inner.a + self.c * inner.b,
            b: self.b * inner.a + self.d * inner.b,
            c: self.a * inner.c + self.c * inner.d,
            d: self.b * inner.c + self.d * inner.d,
            e: self.a * inner.e + self.c * inner.f + self.e,
            f: self.b * inner.e + self.d * inner.f + self.f,
        }
    }
}

/// One point of a glyph outline, in font units.
#[derive(Clone, Copy, Debug)]
pub struct Point {
    pub x: f32,
    pub y: f32,
    /// On-curve points are anchors; off-curve points are quadratic controls.
    pub on_curve: bool,
}

/// A glyph outline: a list of closed contours, in font units (y up).
#[derive(Clone, Debug, Default)]
pub struct Outline {
    pub contours: Vec<Vec<Point>>,
}

impl Outline {
    pub fn is_empty(&self) -> bool {
        self.contours.iter().all(|c| c.is_empty())
    }
}

/// A parsed TrueType font.
pub struct Font {
    data: Vec<u8>,
    units_per_em: u16,
    num_glyphs: u16,
    long_loca: bool,
    num_h_metrics: u16,
    ascent: i16,
    descent: i16,
    line_gap: i16,
    loca: usize,
    glyf: usize,
    hmtx: usize,
    cmap_sub: usize,
    cmap_format: u16,
    kern: Option<usize>,
    /// GPOS `kern`-feature PairPos subtables: one inner list per lookup
    /// (absolute offsets), in lookup order. Empty when the font has no usable
    /// GPOS pair kerning.
    gpos_pair_lookups: Vec<Vec<usize>>,
}

impl Font {
    /// Parses a TrueType font from its raw bytes.
    pub fn from_bytes(data: Vec<u8>) -> Result<Self, FontError> {
        if data.len() < 12 {
            return Err(FontError::TooShort);
        }
        let sfnt = ru32(&data, 0)?;
        // 0x00010000 = TrueType outlines; 'true' = legacy Apple TrueType.
        if sfnt != 0x0001_0000 && sfnt != 0x7472_7565 {
            return Err(FontError::NotTrueType);
        }
        let num_tables = ru16(&data, 4)? as usize;

        let find = |tag: &[u8; 4]| -> Option<(usize, usize)> {
            for i in 0..num_tables {
                let rec = 12 + i * 16;
                let t = data.get(rec..rec + 4)?;
                if t == tag {
                    let off = ru32(&data, rec + 8).ok()? as usize;
                    let len = ru32(&data, rec + 12).ok()? as usize;
                    if off.checked_add(len)? <= data.len() {
                        return Some((off, len));
                    }
                }
            }
            None
        };

        let head = find(b"head").ok_or(FontError::MissingTable("head"))?.0;
        let maxp = find(b"maxp").ok_or(FontError::MissingTable("maxp"))?.0;
        let hhea = find(b"hhea").ok_or(FontError::MissingTable("hhea"))?.0;
        let (hmtx, _) = find(b"hmtx").ok_or(FontError::MissingTable("hmtx"))?;
        let (loca, _) = find(b"loca").ok_or(FontError::MissingTable("loca"))?;
        let (glyf, _) = find(b"glyf").ok_or(FontError::MissingTable("glyf"))?;
        let (cmap, _) = find(b"cmap").ok_or(FontError::MissingTable("cmap"))?;
        let kern = find(b"kern").map(|(o, _)| o);
        // A malformed GPOS degrades to no GPOS kerning (legacy `kern` fallback)
        // rather than failing the whole font — the outlines still render.
        let gpos_pair_lookups = find(b"GPOS")
            .map(|(o, _)| gpos_kern_pair_lookups(&data, o).unwrap_or_default())
            .unwrap_or_default();

        let units_per_em = ru16(&data, head + 18)?;
        let long_loca = ri16(&data, head + 50)? != 0;
        let num_glyphs = ru16(&data, maxp + 4)?;
        let ascent = ri16(&data, hhea + 4)?;
        let descent = ri16(&data, hhea + 6)?;
        let line_gap = ri16(&data, hhea + 8)?;
        let num_h_metrics = ru16(&data, hhea + 34)?;

        let (cmap_sub, cmap_format) = select_cmap(&data, cmap)?;

        Ok(Self {
            data,
            units_per_em,
            num_glyphs,
            long_loca,
            num_h_metrics,
            ascent,
            descent,
            line_gap,
            loca,
            glyf,
            hmtx,
            cmap_sub,
            cmap_format,
            kern,
            gpos_pair_lookups,
        })
    }

    pub fn units_per_em(&self) -> u16 {
        self.units_per_em
    }
    pub fn num_glyphs(&self) -> u16 {
        self.num_glyphs
    }
    pub fn ascent(&self) -> i16 {
        self.ascent
    }
    pub fn descent(&self) -> i16 {
        self.descent
    }
    pub fn line_gap(&self) -> i16 {
        self.line_gap
    }

    /// Pixels-per-font-unit at em size `px`.
    pub fn scale(&self, px: f32) -> f32 {
        px / self.units_per_em as f32
    }

    /// Baseline-to-baseline distance in pixels at em size `px`.
    pub fn line_height(&self, px: f32) -> f32 {
        (self.ascent as f32 - self.descent as f32 + self.line_gap as f32) * self.scale(px)
    }

    /// Maps a character to a glyph id (0 = `.notdef`/missing).
    pub fn glyph_index(&self, c: char) -> u16 {
        self.glyph_index_checked(c as u32).unwrap_or(0)
    }

    fn glyph_index_checked(&self, cp: u32) -> Result<u16, FontError> {
        let d = &self.data;
        let sub = self.cmap_sub;
        match self.cmap_format {
            4 => {
                if cp > 0xFFFF {
                    return Ok(0);
                }
                let seg_count = ru16(d, sub + 6)? as usize / 2;
                let end_codes = sub + 14;
                let start_codes = end_codes + seg_count * 2 + 2;
                let id_deltas = start_codes + seg_count * 2;
                let id_ranges = id_deltas + seg_count * 2;
                // Segments are sorted by end code (per spec): binary-search the
                // first segment covering `cp` — `measure` runs this per char per
                // label on the layout hot path. (A malformed unsorted table just
                // resolves wrong glyphs; every read stays bounds-checked.)
                let (mut lo, mut hi) = (0usize, seg_count);
                while lo < hi {
                    let mid = lo + (hi - lo) / 2;
                    if (ru16(d, end_codes + mid * 2)? as u32) < cp {
                        lo = mid + 1;
                    } else {
                        hi = mid;
                    }
                }
                if lo == seg_count {
                    return Ok(0);
                }
                let i = lo;
                let start = ru16(d, start_codes + i * 2)? as u32;
                if cp < start {
                    return Ok(0);
                }
                let delta = ri16(d, id_deltas + i * 2)? as i32;
                let range = ru16(d, id_ranges + i * 2)? as usize;
                if range == 0 {
                    return Ok(((cp as i32 + delta) & 0xFFFF) as u16);
                }
                let gi = id_ranges + i * 2 + range + (cp - start) as usize * 2;
                let g = ru16(d, gi)?;
                if g == 0 {
                    return Ok(0);
                }
                Ok(((g as i32 + delta) & 0xFFFF) as u16)
            }
            12 => {
                let n = ru32(d, sub + 12)? as usize;
                // Groups are sorted by start code (per spec): binary-search the
                // last group starting at or before `cp`.
                let (mut lo, mut hi) = (0usize, n);
                while lo < hi {
                    let mid = lo + (hi - lo) / 2;
                    if ru32(d, sub + 16 + mid * 12)? <= cp {
                        lo = mid + 1;
                    } else {
                        hi = mid;
                    }
                }
                if lo == 0 {
                    return Ok(0);
                }
                let g = sub + 16 + (lo - 1) * 12;
                let start = ru32(d, g)?;
                let end = ru32(d, g + 4)?;
                if cp >= start && cp <= end {
                    let start_gid = ru32(d, g + 8)?;
                    return Ok((start_gid + (cp - start)) as u16);
                }
                Ok(0)
            }
            _ => Ok(0),
        }
    }

    /// Horizontal advance of a glyph, in font units.
    pub fn h_advance(&self, gid: u16) -> u16 {
        let idx = (gid as usize).min(self.num_h_metrics.saturating_sub(1) as usize);
        ru16(&self.data, self.hmtx + idx * 4).unwrap_or(0)
    }

    /// Kerning adjustment for the pair `(left, right)`, in font units (0 if the
    /// font has no kerning or no entry for this pair). Reads the GPOS `kern`
    /// feature (PairPos) when the font has one — shapers ignore the legacy
    /// table in that case, so applying both would kern twice — and falls back
    /// to the legacy `kern` table (format 0) otherwise.
    pub fn kern(&self, left: u16, right: u16) -> i16 {
        if self.gpos_pair_lookups.is_empty() {
            self.kern_checked(left, right).unwrap_or(0)
        } else {
            self.gpos_kern_checked(left, right).unwrap_or(0)
        }
    }

    fn kern_checked(&self, left: u16, right: u16) -> Result<i16, FontError> {
        let Some(k) = self.kern else { return Ok(0) };
        let d = &self.data;
        let n_tables = ru16(d, k + 2)? as usize;
        let mut p = k + 4;
        for _ in 0..n_tables {
            let sub_len = ru16(d, p + 2)? as usize;
            let coverage = ru16(d, p + 4)?;
            let format = coverage >> 8;
            let horizontal = coverage & 0x1 != 0;
            if format == 0 && horizontal {
                let n_pairs = ru16(d, p + 6)? as i64;
                let pairs = p + 14;
                let key = ((left as u32) << 16) | right as u32;
                let (mut lo, mut hi) = (0i64, n_pairs - 1);
                while lo <= hi {
                    let mid = (lo + hi) / 2;
                    let e = pairs + mid as usize * 6;
                    let pair = ((ru16(d, e)? as u32) << 16) | ru16(d, e + 2)? as u32;
                    match pair.cmp(&key) {
                        std::cmp::Ordering::Equal => return ri16(d, e + 4),
                        std::cmp::Ordering::Less => lo = mid + 1,
                        std::cmp::Ordering::Greater => hi = mid - 1,
                    }
                }
                return Ok(0);
            }
            p += sub_len.max(6);
        }
        Ok(0)
    }

    /// GPOS pair kerning: lookups accumulate; within a lookup the first
    /// subtable that APPLIES to the pair decides (see
    /// [`pair_pos_x_advance`] for what applying means per format) — the
    /// shaper application model.
    fn gpos_kern_checked(&self, left: u16, right: u16) -> Result<i16, FontError> {
        let mut total = 0i32;
        for subtables in &self.gpos_pair_lookups {
            for &so in subtables {
                if let Some(adj) = pair_pos_x_advance(&self.data, so, left, right)? {
                    total += adj as i32;
                    break;
                }
            }
        }
        Ok(total.clamp(i16::MIN as i32, i16::MAX as i32) as i16)
    }

    /// The outline of a glyph in font units, resolving composite glyphs. Returns
    /// an empty outline for blank glyphs (e.g. space) or on malformed data.
    pub fn outline(&self, gid: u16) -> Outline {
        let mut contours = Vec::new();
        let _ = self.outline_into(gid, Affine::ID, 0, &mut contours);
        Outline { contours }
    }

    fn outline_into(
        &self,
        gid: u16,
        tf: Affine,
        depth: u32,
        out: &mut Vec<Vec<Point>>,
    ) -> Result<(), FontError> {
        if depth > 8 || gid >= self.num_glyphs {
            return Ok(());
        }
        let d = &self.data;
        let (start, end) = self.loca_range(gid)?;
        if start == end {
            return Ok(()); // empty glyph
        }
        let g = self.glyf + start;
        let num_contours = ri16(d, g)?;
        if num_contours < 0 {
            return self.composite_into(g, tf, depth, out);
        }
        let num_contours = num_contours as usize;

        let endpts = g + 10;
        let mut end_pts = Vec::with_capacity(num_contours);
        for i in 0..num_contours {
            end_pts.push(ru16(d, endpts + i * 2)? as usize);
        }
        // Per spec `endPtsOfContours` ascends. A malformed (non-increasing)
        // array would index past the flag/coordinate arrays built below —
        // reject it instead of trusting it.
        if !end_pts.windows(2).all(|w| w[0] < w[1]) {
            return Err(FontError::OutOfBounds);
        }
        let num_points = end_pts.last().map(|&e| e + 1).unwrap_or(0);

        let ins_len = ru16(d, endpts + num_contours * 2)? as usize;
        let mut p = endpts + num_contours * 2 + 2 + ins_len;

        // Flags (with the repeat-run encoding).
        let mut flags = Vec::with_capacity(num_points);
        while flags.len() < num_points {
            let f = ru8(d, p)?;
            p += 1;
            flags.push(f);
            if f & 0x08 != 0 {
                let repeat = ru8(d, p)?;
                p += 1;
                for _ in 0..repeat {
                    if flags.len() < num_points {
                        flags.push(f);
                    }
                }
            }
        }

        // X then Y coordinate deltas.
        let mut xs = Vec::with_capacity(num_points);
        let mut x = 0i32;
        for &f in &flags {
            if f & 0x02 != 0 {
                let dx = ru8(d, p)? as i32;
                p += 1;
                x += if f & 0x10 != 0 { dx } else { -dx };
            } else if f & 0x10 == 0 {
                x += ri16(d, p)? as i32;
                p += 2;
            }
            xs.push(x);
        }
        let mut ys = Vec::with_capacity(num_points);
        let mut y = 0i32;
        for &f in &flags {
            if f & 0x04 != 0 {
                let dy = ru8(d, p)? as i32;
                p += 1;
                y += if f & 0x20 != 0 { dy } else { -dy };
            } else if f & 0x20 == 0 {
                y += ri16(d, p)? as i32;
                p += 2;
            }
            ys.push(y);
        }

        let mut s = 0;
        for &e in &end_pts {
            let mut contour = Vec::with_capacity(e + 1 - s);
            for i in s..=e {
                let (px, py) = tf.apply(xs[i] as f32, ys[i] as f32);
                contour.push(Point {
                    x: px,
                    y: py,
                    on_curve: flags[i] & 0x01 != 0,
                });
            }
            out.push(contour);
            s = e + 1;
        }
        Ok(())
    }

    fn composite_into(
        &self,
        g: usize,
        tf: Affine,
        depth: u32,
        out: &mut Vec<Vec<Point>>,
    ) -> Result<(), FontError> {
        let d = &self.data;
        let mut p = g + 10;
        loop {
            let flags = ru16(d, p)?;
            let comp_gid = ru16(d, p + 2)?;
            p += 4;

            let (arg1, arg2);
            if flags & 0x0001 != 0 {
                arg1 = ri16(d, p)? as f32;
                arg2 = ri16(d, p + 2)? as f32;
                p += 4;
            } else {
                arg1 = ri8(d, p)? as f32;
                arg2 = ri8(d, p + 1)? as f32;
                p += 2;
            }

            let (mut a, mut b, mut c, mut dd) = (1.0f32, 0.0f32, 0.0f32, 1.0f32);
            if flags & 0x0008 != 0 {
                a = f2dot14(ri16(d, p)?);
                dd = a;
                p += 2;
            } else if flags & 0x0040 != 0 {
                a = f2dot14(ri16(d, p)?);
                dd = f2dot14(ri16(d, p + 2)?);
                p += 4;
            } else if flags & 0x0080 != 0 {
                a = f2dot14(ri16(d, p)?);
                b = f2dot14(ri16(d, p + 2)?);
                c = f2dot14(ri16(d, p + 4)?);
                dd = f2dot14(ri16(d, p + 6)?);
                p += 8;
            }

            // ARGS_ARE_XY_VALUES (0x0002): args are a font-unit offset. Point
            // matching (the alternative) is rare and unsupported.
            let (e, f) = if flags & 0x0002 != 0 {
                (arg1, arg2)
            } else {
                (0.0, 0.0)
            };
            let component = Affine {
                a,
                b,
                c,
                d: dd,
                e,
                f,
            };
            self.outline_into(comp_gid, tf.compose(&component), depth + 1, out)?;

            if flags & 0x0020 == 0 {
                break; // no MORE_COMPONENTS
            }
        }
        Ok(())
    }

    fn loca_range(&self, gid: u16) -> Result<(usize, usize), FontError> {
        let g = gid as usize;
        if self.long_loca {
            let a = ru32(&self.data, self.loca + g * 4)? as usize;
            let b = ru32(&self.data, self.loca + (g + 1) * 4)? as usize;
            Ok((a, b))
        } else {
            let a = ru16(&self.data, self.loca + g * 2)? as usize * 2;
            let b = ru16(&self.data, self.loca + (g + 1) * 2)? as usize * 2;
            Ok((a, b))
        }
    }
}

fn f2dot14(v: i16) -> f32 {
    v as f32 / 16384.0
}

// ---- GPOS pair kerning (the PairPos subset of the `kern` feature) ----

/// Collects the PairPos subtables of every GPOS `kern` feature: one inner list
/// per referenced lookup (deduped, in lookup order), each entry an absolute
/// subtable offset, resolving type-9 Extension wrappers. Feature selection
/// scans the FeatureList for the `kern` tag across all scripts (a
/// script-agnostic superset — this is pair kerning, not a shaper); the
/// non-pair lookup types a full `kern` feature may reference (contextual,
/// mark) are skipped.
fn gpos_kern_pair_lookups(d: &[u8], gpos: usize) -> Result<Vec<Vec<usize>>, FontError> {
    let feat_list = gpos + ru16(d, gpos + 6)? as usize;
    let lookup_list = gpos + ru16(d, gpos + 8)? as usize;

    let feat_count = ru16(d, feat_list)? as usize;
    let mut indices: Vec<u16> = Vec::new();
    for i in 0..feat_count {
        let rec = feat_list + 2 + i * 6;
        if d.get(rec..rec + 4) != Some(b"kern") {
            if d.get(rec..rec + 4).is_none() {
                return Err(FontError::OutOfBounds);
            }
            continue;
        }
        let feat = feat_list + ru16(d, rec + 4)? as usize;
        let count = ru16(d, feat + 2)? as usize;
        for j in 0..count {
            let idx = ru16(d, feat + 4 + j * 2)?;
            if !indices.contains(&idx) {
                indices.push(idx);
            }
        }
    }
    indices.sort_unstable();

    let lookup_count = ru16(d, lookup_list)? as usize;
    let mut lookups = Vec::new();
    for &li in &indices {
        if li as usize >= lookup_count {
            continue;
        }
        let lo = lookup_list + ru16(d, lookup_list + 2 + li as usize * 2)? as usize;
        let ty = ru16(d, lo)?;
        let sub_count = ru16(d, lo + 4)? as usize;
        let mut subs = Vec::new();
        for s in 0..sub_count {
            let mut so = lo + ru16(d, lo + 6 + s * 2)? as usize;
            let mut t = ty;
            // Extension positioning (type 9, format 1) wraps one real subtable.
            if t == 9 && ru16(d, so)? == 1 {
                t = ru16(d, so + 2)?;
                so += ru32(d, so + 4)? as usize;
            }
            if t == 2 {
                subs.push(so);
            }
        }
        if !subs.is_empty() {
            lookups.push(subs);
        }
    }
    Ok(lookups)
}

/// Coverage-table membership: the coverage index of `gid` at table `cov`, or
/// `None` when not covered. Formats 1 (sorted glyph array) and 2 (sorted
/// ranges), both binary-searched.
fn coverage_index(d: &[u8], cov: usize, gid: u16) -> Result<Option<u16>, FontError> {
    match ru16(d, cov)? {
        1 => {
            let n = ru16(d, cov + 2)? as i64;
            let (mut lo, mut hi) = (0i64, n - 1);
            while lo <= hi {
                let mid = (lo + hi) / 2;
                match ru16(d, cov + 4 + mid as usize * 2)?.cmp(&gid) {
                    std::cmp::Ordering::Equal => return Ok(Some(mid as u16)),
                    std::cmp::Ordering::Less => lo = mid + 1,
                    std::cmp::Ordering::Greater => hi = mid - 1,
                }
            }
            Ok(None)
        }
        2 => {
            let n = ru16(d, cov + 2)? as i64;
            let (mut lo, mut hi) = (0i64, n - 1);
            while lo <= hi {
                let mid = (lo + hi) / 2;
                let r = cov + 4 + mid as usize * 6;
                if ru16(d, r + 2)? < gid {
                    lo = mid + 1;
                } else if ru16(d, r)? > gid {
                    hi = mid - 1;
                } else {
                    return Ok(Some(ru16(d, r + 4)? + (gid - ru16(d, r)?)));
                }
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

/// Glyph class of `gid` per the class-definition table at `cd` (unlisted
/// glyphs are class 0). Formats 1 (dense array) and 2 (sorted ranges).
fn glyph_class(d: &[u8], cd: usize, gid: u16) -> Result<u16, FontError> {
    match ru16(d, cd)? {
        1 => {
            let start = ru16(d, cd + 2)?;
            let n = ru16(d, cd + 4)?;
            if gid >= start && (gid - start) < n {
                ru16(d, cd + 6 + (gid - start) as usize * 2)
            } else {
                Ok(0)
            }
        }
        2 => {
            let n = ru16(d, cd + 2)? as i64;
            let (mut lo, mut hi) = (0i64, n - 1);
            while lo <= hi {
                let mid = (lo + hi) / 2;
                let r = cd + 4 + mid as usize * 6;
                if ru16(d, r + 2)? < gid {
                    lo = mid + 1;
                } else if ru16(d, r)? > gid {
                    hi = mid - 1;
                } else {
                    return ru16(d, r + 4);
                }
            }
            Ok(0)
        }
        _ => Ok(0),
    }
}

/// Bytes one ValueRecord of format `vf` occupies (every present field is 2
/// bytes, device-table offsets included).
fn value_record_size(vf: u16) -> usize {
    vf.count_ones() as usize * 2
}

/// The XAdvance field of the ValueRecord at `off` with format `vf` (0 when the
/// record carries no XAdvance). XAdvance (bit 0x0004) sits after whichever of
/// XPlacement/YPlacement (bits 0x0001/0x0002) are present.
fn value_x_advance(d: &[u8], off: usize, vf: u16) -> Result<i16, FontError> {
    if vf & 0x0004 == 0 {
        return Ok(0);
    }
    ri16(d, off + (vf & 0x0003).count_ones() as usize * 2)
}

/// The pair adjustment (first glyph's XAdvance delta, font units) from ONE
/// PairPos subtable at `so`: `Some(adjustment)` when the subtable APPLIES to
/// the pair, `None` when it does not (try the lookup's next subtable). A
/// format-1 subtable applies only when it holds a record for the exact pair
/// — covered-left-but-recordless does NOT decide (the shaper model): fonts
/// like Noto Sans put a small per-glyph exceptions subtable in front of the
/// class matrix that carries the real kerning, so deciding at the fmt1 miss
/// would zero every ordinary pair. A format-2 subtable applies whenever
/// `left` is covered and both classes are in range (unlisted glyphs are
/// class 0, so an in-matrix zero still decides).
fn pair_pos_x_advance(
    d: &[u8],
    so: usize,
    left: u16,
    right: u16,
) -> Result<Option<i16>, FontError> {
    let format = ru16(d, so)?;
    if format != 1 && format != 2 {
        return Ok(None);
    }
    let Some(ci) = coverage_index(d, so + ru16(d, so + 2)? as usize, left)? else {
        return Ok(None);
    };
    let vf1 = ru16(d, so + 4)?;
    let vf2 = ru16(d, so + 6)?;
    if format == 1 {
        let ps_count = ru16(d, so + 8)? as usize;
        if ci as usize >= ps_count {
            return Ok(None);
        }
        let ps = so + ru16(d, so + 10 + ci as usize * 2)? as usize;
        let rec_size = 2 + value_record_size(vf1) + value_record_size(vf2);
        // PairValueRecords are sorted by second glyph: binary-search.
        let n = ru16(d, ps)? as i64;
        let (mut lo, mut hi) = (0i64, n - 1);
        while lo <= hi {
            let mid = (lo + hi) / 2;
            let rec = ps + 2 + mid as usize * rec_size;
            match ru16(d, rec)?.cmp(&right) {
                std::cmp::Ordering::Equal => {
                    return value_x_advance(d, rec + 2, vf1).map(Some);
                }
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid - 1,
            }
        }
        // No record for this exact pair: the subtable does not apply.
        Ok(None)
    } else {
        let c1 = glyph_class(d, so + ru16(d, so + 8)? as usize, left)?;
        let c2 = glyph_class(d, so + ru16(d, so + 10)? as usize, right)?;
        let c1_count = ru16(d, so + 12)?;
        let c2_count = ru16(d, so + 14)?;
        if c1 >= c1_count || c2 >= c2_count {
            return Ok(None);
        }
        let rec_size = value_record_size(vf1) + value_record_size(vf2);
        let rec = so + 16 + (c1 as usize * c2_count as usize + c2 as usize) * rec_size;
        value_x_advance(d, rec, vf1).map(Some)
    }
}

/// Picks the best `cmap` subtable, returning `(absolute offset, format)`.
/// Preference: Unicode full (format 12) → Unicode BMP (format 4).
fn select_cmap(d: &[u8], cmap: usize) -> Result<(usize, u16), FontError> {
    let n = ru16(d, cmap + 2)? as usize;
    let mut best: Option<(usize, u16, u8)> = None; // (offset, format, rank)
    for i in 0..n {
        let rec = cmap + 4 + i * 8;
        let platform = ru16(d, rec)?;
        let encoding = ru16(d, rec + 2)?;
        let off = cmap + ru32(d, rec + 4)? as usize;
        let format = ru16(d, off).unwrap_or(0);
        // Rank usable subtables; higher is better.
        let rank = match (platform, encoding, format) {
            (3, 10, 12) | (0, _, 12) => 4, // Unicode full
            (3, 1, 4) | (0, _, 4) => 3,    // Unicode BMP
            (_, _, 12) => 2,
            (_, _, 4) => 1,
            _ => 0,
        };
        if rank > 0 && best.is_none_or(|(_, _, r)| rank > r) {
            best = Some((off, format, rank));
        }
    }
    best.map(|(o, f, _)| (o, f))
        .ok_or(FontError::UnsupportedCmap)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font() -> Font {
        Font::from_bytes(include_bytes!("../../assets/DejaVuSans.ttf").to_vec()).unwrap()
    }

    /// A malformed font must yield a [`FontError`] (and an empty outline via the
    /// swallowing [`Font::outline`]), never a panic — `outline` runs inside the
    /// render path on user-supplied fonts. Corrupts a real glyph's
    /// `endPtsOfContours` into a descending array, which used to panic two ways:
    /// an index past the coordinate arrays (`num_points` derives from the *last*
    /// end, not the max) and a `with_capacity(e + 1 - s)` underflow.
    #[test]
    fn malformed_end_pts_error_instead_of_panicking() {
        let f = font();
        let gid = f.glyph_index('B');
        let (start, end) = f.loca_range(gid).unwrap();
        assert!(end > start, "B has an outline");
        let g = f.glyf + start;

        let mut data = f.data.clone();
        let num_contours = ri16(&data, g).unwrap();
        assert!(num_contours >= 2, "B is a multi-contour glyph");
        // Overwrite endPtsOfContours (at glyph offset +10) with a descending
        // sequence: the first end overshoots every later one.
        for i in 0..num_contours as usize {
            let e = ((num_contours as usize - i) * 3) as u16;
            let off = g + 10 + i * 2;
            data[off] = (e >> 8) as u8;
            data[off + 1] = (e & 0xff) as u8;
        }

        let bad = Font::from_bytes(data).expect("header/tables still parse");
        assert!(
            matches!(
                bad.outline_into(gid, Affine::ID, 0, &mut Vec::new()),
                Err(FontError::OutOfBounds)
            ),
            "non-ascending endPtsOfContours is rejected as OutOfBounds"
        );
        // The public entry point swallows the error into an empty outline.
        assert!(bad.outline(gid).contours.is_empty());
    }

    #[test]
    fn parses_header() {
        let f = font();
        assert_eq!(f.units_per_em(), 2048);
        assert_eq!(f.num_glyphs(), 6241);
        assert_eq!(f.ascent(), 1901);
        assert_eq!(f.descent(), -483);
    }

    #[test]
    fn cmap_maps_characters() {
        let f = font();
        let a = f.glyph_index('A');
        let b = f.glyph_index('B');
        assert_ne!(a, 0);
        assert_ne!(b, 0);
        assert_ne!(a, b);
        assert_ne!(f.glyph_index(' '), 0);
        // A codepoint beyond the font's coverage -> .notdef (exercises the
        // >BMP format-12 path; note DejaVu *does* include some U+1Fxxx glyphs).
        assert_eq!(f.glyph_index('\u{10FFFF}'), 0);
    }

    #[test]
    fn advances_are_positive() {
        let f = font();
        assert!(f.h_advance(f.glyph_index('A')) > 0);
        assert!(f.h_advance(f.glyph_index('m')) > f.h_advance(f.glyph_index('i')));
    }

    #[test]
    fn simple_glyph_has_contours() {
        let f = font();
        let a = f.outline(f.glyph_index('A'));
        assert!(!a.is_empty());
        assert!(a.contours.iter().any(|c| c.iter().any(|p| p.on_curve)));
        // Space is blank.
        assert!(f.outline(f.glyph_index(' ')).is_empty());
    }

    #[test]
    fn composite_glyph_resolves_components() {
        let f = font();
        // 'é' (U+00E9) is a composite of 'e' + acute accent in DejaVu Sans.
        let acute = f.outline(f.glyph_index('é'));
        let e = f.outline(f.glyph_index('e'));
        assert!(acute.contours.len() > e.contours.len());
    }

    #[test]
    fn kerning_pulls_av_together() {
        let f = font();
        // 'A'/'V' is a classic negative-kern pair.
        let k = f.kern(f.glyph_index('A'), f.glyph_index('V'));
        assert!(k < 0, "expected negative AV kern, got {k}");
    }

    /// Renames a top-level sfnt table so `find` no longer sees it.
    fn zap_table(data: &mut [u8], tag: &[u8; 4]) {
        let n = ru16(data, 4).unwrap() as usize;
        for i in 0..n {
            let rec = 12 + i * 16;
            if &data[rec..rec + 4] == tag {
                data[rec..rec + 4].copy_from_slice(b"zzzz");
                return;
            }
        }
        panic!("table {} not found", String::from_utf8_lossy(tag));
    }

    /// Pair values probed from DejaVuSans' GPOS with an independent (Node)
    /// parser — class-based PairPos format 2, ClassDef format 2, XAdvance-only
    /// value records. DejaVu generates GPOS and legacy `kern` from the same
    /// source, so these hold for both paths.
    const DEJAVU_PAIRS: [(char, char, i16); 7] = [
        ('A', 'V', -131),
        ('A', 'T', -159),
        ('T', 'o', -348),
        ('A', 'A', 57),
        ('L', 'T', -282),
        ('Y', 'o', -272),
        ('P', ',', 0),
    ];

    #[test]
    fn gpos_kern_matches_independent_parser() {
        let f = font();
        assert!(
            !f.gpos_pair_lookups.is_empty(),
            "DejaVu has GPOS pair kerning, so kern() must take the GPOS path"
        );
        for (l, r, k) in DEJAVU_PAIRS {
            assert_eq!(f.kern(f.glyph_index(l), f.glyph_index(r)), k, "{l}{r}");
        }
    }

    /// GPOS and the legacy `kern` table must each carry kerning alone: a font
    /// stripped of one still kerns through the other, stripped of both kerns
    /// zero. (DejaVu has both; modern fonts often ship GPOS only.)
    #[test]
    fn gpos_and_legacy_kern_each_work_alone() {
        let full = font();
        let pairs: Vec<(u16, u16, i16)> = DEJAVU_PAIRS
            .iter()
            .map(|&(l, r, k)| (full.glyph_index(l), full.glyph_index(r), k))
            .collect();

        let mut no_legacy = full.data.clone();
        zap_table(&mut no_legacy, b"kern");
        let f = Font::from_bytes(no_legacy).unwrap();
        assert!(!f.gpos_pair_lookups.is_empty());
        for &(l, r, k) in &pairs {
            assert_eq!(f.kern(l, r), k, "GPOS-only");
        }

        let mut no_gpos = full.data.clone();
        zap_table(&mut no_gpos, b"GPOS");
        let f = Font::from_bytes(no_gpos).unwrap();
        assert!(f.gpos_pair_lookups.is_empty());
        for &(l, r, k) in &pairs {
            assert_eq!(f.kern(l, r), k, "legacy-only");
        }

        let mut neither = full.data.clone();
        zap_table(&mut neither, b"kern");
        zap_table(&mut neither, b"GPOS");
        let f = Font::from_bytes(neither).unwrap();
        for &(l, r, _) in &pairs {
            assert_eq!(f.kern(l, r), 0, "no kerning tables");
        }
    }

    /// Big-endian u16 buffer builder for synthetic GPOS subtables (i16 values
    /// pass through `as u16`).
    fn be(words: &[u16]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    /// PairPos format 1 (per-glyph pair sets, coverage format 1) with layered
    /// value records: vf1 = XPlacement|XAdvance exercises the field offset
    /// within the record, vf2 = XAdvance exercises the record stride.
    #[test]
    fn synthetic_pair_pos_format_1() {
        #[rustfmt::skip]
        let d = be(&[
            // +0: header — format 1, coverage @14, vf1 0x0005, vf2 0x0004,
            //     2 pair sets @22 / @40
            1, 14, 0x0005, 0x0004, 2, 22, 40,
            // +14: coverage format 1 — glyphs {10, 20}
            1, 2, 10, 20,
            // +22: pair set for glyph 10 — records sorted by second glyph:
            //     (5:  xplace 1, xadv -40, v2 xadv 7)
            //     (30: xplace 0, xadv  25, v2 xadv 0)
            2, 5, 1, (-40i16) as u16, 7, 30, 0, 25, 0,
            // +40: pair set for glyph 20 — (10: xplace 0, xadv -77, v2 xadv 0)
            1, 10, 0, (-77i16) as u16, 0,
        ]);
        assert_eq!(pair_pos_x_advance(&d, 0, 10, 5), Ok(Some(-40)));
        assert_eq!(pair_pos_x_advance(&d, 0, 10, 30), Ok(Some(25)));
        assert_eq!(
            pair_pos_x_advance(&d, 0, 10, 99),
            Ok(None),
            "covered left but no pair record: the subtable does not apply"
        );
        assert_eq!(pair_pos_x_advance(&d, 0, 20, 10), Ok(Some(-77)));
        assert_eq!(
            pair_pos_x_advance(&d, 0, 99, 5),
            Ok(None),
            "uncovered left: fall through to the lookup's next subtable"
        );
    }

    /// PairPos format 2 (class matrix) with coverage format 2 (ranges) and
    /// ClassDef formats 1 (dense) and 2 (ranges).
    #[test]
    fn synthetic_pair_pos_format_2() {
        #[rustfmt::skip]
        let d = be(&[
            // +0: header — format 2, coverage @28, vf1 0x0004, vf2 0,
            //     classdefs @38 / @50, 2×3 classes
            2, 28, 0x0004, 0, 38, 50, 2, 3,
            // +16: class matrix, XAdvance only:
            //     class1 0: [0, -11, 22]   class1 1: [33, 0, -44]
            0, (-11i16) as u16, 22, 33, 0, (-44i16) as u16,
            // +28: coverage format 2 — one range 10..=12, coverage index 0
            2, 1, 10, 12, 0,
            // +38: classdef 1 format 1 — glyphs 10..13 → classes [0, 1, 1]
            1, 10, 3, 0, 1, 1,
            // +50: classdef 2 format 2 — ranges 5..=5 → 1, 6..=7 → 2
            2, 2, 5, 5, 1, 6, 7, 2,
        ]);
        assert_eq!(
            pair_pos_x_advance(&d, 0, 11, 6),
            Ok(Some(-44)),
            "classes (1,2)"
        );
        assert_eq!(
            pair_pos_x_advance(&d, 0, 10, 5),
            Ok(Some(-11)),
            "classes (0,1)"
        );
        assert_eq!(
            pair_pos_x_advance(&d, 0, 12, 7),
            Ok(Some(-44)),
            "range lookups"
        );
        assert_eq!(
            pair_pos_x_advance(&d, 0, 10, 99),
            Ok(Some(0)),
            "unlisted right glyph is class 0"
        );
        assert_eq!(pair_pos_x_advance(&d, 0, 9, 5), Ok(None), "uncovered left");
    }

    /// The lookup-level application model: a format-1 subtable that covers
    /// the left glyph but holds no record for the pair does not apply, so
    /// the walk continues to the next subtable — where class kerning
    /// usually lives. This is exactly how Noto Sans ships its Latin
    /// kerning (a small fmt1 exceptions subtable in front of the fmt2
    /// class matrix); deciding at the fmt1 miss zeroed every ordinary
    /// pair. The subtables are grafted onto a real font so the walk in
    /// `gpos_kern_checked` (not just the subtable probe) is what's tested.
    #[test]
    fn lookup_walk_falls_through_recordless_format_1() {
        let mut f = font();
        // fmt1: covers glyph 10 only; one record (right 5, xadv -40).
        #[rustfmt::skip]
        let sub1 = be(&[
            1, 12, 0x0004, 0, 1, 18, // header: coverage @12, one pair set @18
            1, 1, 10,                // coverage fmt 1: {10}
            1, 5, (-40i16) as u16,   // pair set: (5 → xadv -40)
        ]);
        // fmt2: covers glyph 10; class2 of 99 is 1; matrix [0, -25].
        #[rustfmt::skip]
        let sub2 = be(&[
            2, 20, 0x0004, 0, 26, 34, 1, 2, // header: cov @20, cd @26/@34, 1×2
            0, (-25i16) as u16,             // class matrix
            1, 1, 10,                       // coverage fmt 1: {10}
            1, 10, 1, 0,                    // classdef1: glyph 10 → class 0
            1, 99, 1, 1,                    // classdef2: glyph 99 → class 1
        ]);
        let s1 = f.data.len();
        f.data.extend_from_slice(&sub1);
        let s2 = f.data.len();
        f.data.extend_from_slice(&sub2);
        f.gpos_pair_lookups = vec![vec![s1, s2]];

        assert_eq!(f.kern(10, 5), -40, "the fmt1 record applies and decides");
        assert_eq!(
            f.kern(10, 99),
            -25,
            "recordless fmt1 falls through to the class matrix"
        );
        assert_eq!(f.kern(11, 5), 0, "uncovered in every subtable");
    }

    /// Big-endian u32 buffer builder (for cmap format-12 groups, table
    /// records, and GPOS extension offsets).
    fn be32(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    /// Each [`FontError`] must display its diagnosis — these strings are what
    /// a host logs when a user-supplied font fails to load.
    #[test]
    fn errors_display_their_diagnosis() {
        let cases = [
            (FontError::TooShort, "too short"),
            (FontError::NotTrueType, "CFF"),
            (FontError::MissingTable("head"), "`head`"),
            (FontError::UnsupportedCmap, "cmap"),
            (FontError::OutOfBounds, "out of bounds"),
        ];
        for (e, needle) in cases {
            let msg = e.to_string();
            assert!(msg.contains(needle), "{msg:?} should mention {needle:?}");
        }
    }

    /// Header-level rejections: data shorter than the sfnt header, CFF
    /// (`OTTO`) containers, and a table record pointing past the end of the
    /// data — which must make the table count as missing, not be trusted.
    #[test]
    fn rejects_short_data_cff_and_bogus_table_records() {
        assert_eq!(
            Font::from_bytes(vec![0; 11]).err(),
            Some(FontError::TooShort)
        );

        let mut otto = b"OTTO".to_vec();
        otto.extend([0; 8]);
        assert_eq!(Font::from_bytes(otto).err(), Some(FontError::NotTrueType));

        // sfnt 0x00010000, one table record: `head` at offset 0xFFFF, len 16
        // — past the end of the data, so the record is ignored.
        let mut d = be(&[1, 0, 1, 0, 0, 0]);
        d.extend_from_slice(b"head");
        d.extend(be32(&[0, 0xFFFF, 16]));
        assert_eq!(
            Font::from_bytes(d).err(),
            Some(FontError::MissingTable("head"))
        );
    }

    /// `line_gap` completes the vertical metrics: baseline-to-baseline
    /// distance is `(ascent - descent + lineGap)` scaled to the em size.
    #[test]
    fn line_gap_feeds_line_height() {
        let f = font();
        assert_eq!(f.line_gap(), 0, "DejaVu Sans carries no extra leading");
        assert_eq!(
            f.line_height(2048.0),
            (1901 + 483) as f32,
            "at one pixel per font unit"
        );
    }

    /// cmap format 4, on a synthetic two-segment subtable grafted onto the
    /// real font (DejaVu itself selects its format-12 table): segment 0 maps
    /// via idDelta alone, segment 1 via the glyph-id array — including the
    /// spec rules that array entry 0 stays `.notdef` (no delta applied),
    /// codepoints below a segment's start are missing, and the format is
    /// BMP-only.
    #[test]
    fn cmap_format_4_resolves_delta_and_array_segments() {
        let mut f = font();
        let sub = f.data.len();
        #[rustfmt::skip]
        f.data.extend(be(&[
            // format, length, language, segCountX2, searchRange,
            // entrySelector, rangeShift
            4, 0, 0, 4, 0, 0, 0,
            // endCode[2], reservedPad, startCode[2]
            67, 105, 0, 65, 100,
            // idDelta[2]: 'A'..='C' shift by -32; segment 1 shifts by +5
            (-32i16) as u16, 5,
            // idRangeOffset[2]: segment 0 uses idDelta; segment 1 points 2
            // bytes ahead of its own slot, to the glyph-id array
            0, 2,
            // glyphIdArray for codepoints 100..=105
            7, 0, 9, 12, 0, 3,
        ]));
        f.cmap_sub = sub;
        f.cmap_format = 4;
        assert_eq!(f.glyph_index('A'), 33, "idDelta segment: 65 - 32");
        assert_eq!(f.glyph_index('C'), 35, "end of the idDelta segment");
        assert_eq!(f.glyph_index('P'), 0, "below segment 1's start code");
        assert_eq!(
            f.glyph_index('d'),
            12,
            "array segment applies idDelta too: 7 + 5"
        );
        assert_eq!(
            f.glyph_index('e'),
            0,
            "array entry 0 is .notdef; no delta applied"
        );
        assert_eq!(f.glyph_index('g'), 17, "later array entry: 12 + 5");
        assert_eq!(f.glyph_index('È'), 0, "past every segment's end code");
        assert_eq!(f.glyph_index('\u{10000}'), 0, "format 4 is BMP-only");
    }

    /// cmap format 12: codepoints below the first group are missing, and an
    /// unrecognized subtable format maps everything to `.notdef` rather than
    /// erring mid-layout.
    #[test]
    fn cmap_format_12_below_range_and_unknown_format() {
        let mut f = font();
        let sub = f.data.len();
        // Header (format 12.0, length, language), one group: 100..=105 → 50..
        f.data.extend(be32(&[0x000C_0000, 28, 0, 1, 100, 105, 50]));
        f.cmap_sub = sub;
        f.cmap_format = 12;
        assert_eq!(f.glyph_index('c'), 0, "below the first group's start");
        assert_eq!(f.glyph_index('d'), 50, "group start");
        assert_eq!(f.glyph_index('i'), 55, "group end");
        f.cmap_format = 6; // parse-time selection never picks this, but stay safe
        assert_eq!(f.glyph_index('A'), 0, "unknown formats resolve to .notdef");
    }

    /// The legacy `kern` reader must skip non-horizontal / non-format-0
    /// subtables by their declared length, find an applicable one later in
    /// the table, and kern zero when none applies at all.
    #[test]
    fn legacy_kern_skips_inapplicable_subtables() {
        let mut f = font();
        f.gpos_pair_lookups = Vec::new(); // force the legacy path
        let k = f.data.len();
        #[rustfmt::skip]
        f.data.extend(be(&[
            // kern header: version, nTables = 2
            0, 2,
            // subtable 1: version, length 6, coverage format 0 but VERTICAL
            0, 6, 0x0000,
            // subtable 2: version, length 20, coverage format 0, horizontal
            0, 20, 0x0001,
            // nPairs, searchRange, entrySelector, rangeShift
            1, 0, 0, 0,
            // pair (3, 7) -> -25
            3, 7, (-25i16) as u16,
        ]));
        f.kern = Some(k);
        assert_eq!(
            f.kern(3, 7),
            -25,
            "found after skipping the vertical subtable"
        );

        // A table with ONLY inapplicable subtables (format 1) kerns zero.
        let k2 = f.data.len();
        f.data.extend(be(&[0, 1, 0, 6, 0x0101]));
        f.kern = Some(k2);
        assert_eq!(f.kern(3, 7), 0, "no applicable subtable kerns zero");
    }

    /// Glyph ids at or past `numGlyphs` outline empty — a stale or foreign
    /// gid must not read a bogus `loca` entry.
    #[test]
    fn out_of_range_gid_outlines_empty() {
        let f = font();
        assert!(f.outline(f.num_glyphs()).is_empty());
        assert!(f.outline(u16::MAX).is_empty());
    }

    /// Composite transforms — WE_HAVE_A_SCALE, X_AND_Y_SCALE, TWO_BY_TWO, and
    /// non-XY (point-matching) args degrading to no offset — checked by
    /// grafting a synthetic four-component composite onto the real font and
    /// comparing each component against the plain outline mapped through the
    /// expected affine.
    #[test]
    fn composite_transforms_apply_scales_and_matrices() {
        let mut f = font();
        let base = f.glyph_index('A');
        let plain = f.outline(base);
        assert!(!plain.is_empty());

        let g = f.data.len();
        // 10 bytes of glyph header (numContours = -1 + bbox); records follow.
        f.data.extend(be(&[(-1i16) as u16, 0, 0, 0, 0]));
        #[rustfmt::skip]
        f.data.extend(be(&[
            // MORE | WORDS | XY | WE_HAVE_A_SCALE: offset (100, -50), ×0.5
            0x002B, base, 100, (-50i16) as u16, 8192,
            // MORE | WORDS | XY | X_AND_Y_SCALE: no offset, ×1.0 / ×0.25
            0x0063, base, 0, 0, 16384, 4096,
            // MORE | WORDS | XY | TWO_BY_TWO: offset (10, 20), [0 .5; -.5 0]
            0x00A3, base, 10, 20, 0, 8192, (-8192i16) as u16, 0,
            // last: WORDS only — args are point indices, NOT an offset
            0x0001, base, 1, 2,
        ]));
        let mut out = Vec::new();
        f.composite_into(g, Affine::ID, 0, &mut out).unwrap();
        assert_eq!(out.len(), 4 * plain.contours.len());

        // (a, b, c, d, e, f) per component, in record order.
        let expect = [
            [0.5, 0.0, 0.0, 0.5, 100.0, -50.0],
            [1.0, 0.0, 0.0, 0.25, 0.0, 0.0],
            [0.0, 0.5, -0.5, 0.0, 10.0, 20.0],
            [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        ];
        for (ci, [a, b, c, d, e, ff]) in expect.iter().enumerate() {
            for (k, contour) in plain.contours.iter().enumerate() {
                let got = &out[ci * plain.contours.len() + k];
                assert_eq!(got.len(), contour.len());
                for (p, q) in contour.iter().zip(got) {
                    let (ex, ey) = (a * p.x + c * p.y + e, b * p.x + d * p.y + ff);
                    assert!(
                        (q.x - ex).abs() < 1e-3 && (q.y - ey).abs() < 1e-3,
                        "component {ci}: ({}, {}) mapped to ({}, {}), want ({ex}, {ey})",
                        p.x,
                        p.y,
                        q.x,
                        q.y
                    );
                    assert_eq!(p.on_curve, q.on_curve);
                }
            }
        }
    }

    /// The short (16-bit) `loca` format stores byte offsets halved, so reads
    /// must double them. DejaVu is long-loca; graft a short table on.
    #[test]
    fn short_loca_offsets_are_doubled() {
        let mut f = font();
        let loca = f.data.len();
        f.data.extend(be(&[0x0010, 0x0020, 0x0030]));
        f.loca = loca;
        f.long_loca = false;
        assert_eq!(f.loca_range(0).unwrap(), (0x20, 0x40));
        assert_eq!(f.loca_range(1).unwrap(), (0x40, 0x60));
        assert_eq!(
            f.loca_range(2),
            Err(FontError::OutOfBounds),
            "past the end of the table"
        );
    }

    /// GPOS collection resilience: a FeatureList that promises more records
    /// than the data holds errors (from_bytes then degrades to no GPOS
    /// kerning), and a `kern` feature referencing a lookup index past the
    /// LookupList is skipped rather than trusted.
    #[test]
    fn gpos_collection_rejects_truncated_and_out_of_range_lists() {
        // Header: version, scriptList, featureList @10, lookupList @12;
        // FeatureList claims 3 records but the data ends after the count.
        let d = be(&[1, 0, 0, 10, 12, 3]);
        assert_eq!(gpos_kern_pair_lookups(&d, 0), Err(FontError::OutOfBounds));

        // A well-formed `kern` feature whose lookup index (5) is out of
        // range for a 1-entry LookupList: no lookups collected.
        let mut d = be(&[1, 0, 0, 10, 24]); // featureList @10, lookupList @24
        d.extend(be(&[1])); // featureCount
        d.extend_from_slice(b"kern");
        d.extend(be(&[8])); // feature table @ featList + 8 = 18
        d.extend(be(&[0, 1, 5])); // featureParams, lookupIndexCount, index 5
        d.extend(be(&[1])); // lookupCount = 1 @24
        assert_eq!(gpos_kern_pair_lookups(&d, 0), Ok(Vec::new()));
    }

    /// Extension positioning (lookup type 9) wraps one real subtable behind a
    /// 32-bit offset: collection must unwrap it and record the *absolute*
    /// PairPos subtable offset.
    #[test]
    fn gpos_extension_lookup_unwraps_to_its_pair_subtable() {
        let mut d = be(&[1, 0, 0, 10, 24]); // featureList @10, lookupList @24
        d.extend(be(&[1])); // featureCount
        d.extend_from_slice(b"kern");
        d.extend(be(&[8])); // feature table @ featList + 8 = 18
        d.extend(be(&[0, 1, 0])); // featureParams, lookupIndexCount, index 0
        d.extend(be(&[1, 4])); // lookupCount, lookupOffset → lookup @28
        d.extend(be(&[9, 0, 1, 8])); // type 9, flag, subCount, offset → @36
        d.extend(be(&[1, 2])); // extension format 1, wrapped type 2
        d.extend(be32(&[10])); // extensionOffset → 36 + 10 = 46
        d.extend(be(&[0])); // pad to 46
        d.extend(be(&[2])); // the wrapped PairPos subtable's format word
        assert_eq!(gpos_kern_pair_lookups(&d, 0), Ok(vec![vec![46]]));
    }

    /// Coverage and class-def range tables: the binary search descends both
    /// directions, gaps between ranges miss, and unknown formats degrade (no
    /// coverage / class 0) instead of erring.
    #[test]
    fn coverage_and_class_lookups_handle_gaps_and_unknown_formats() {
        // Coverage format 2, two ranges: 10..=12 (from index 0), 20..=22
        // (from index 3).
        let cov = be(&[2, 2, 10, 12, 0, 20, 22, 3]);
        assert_eq!(
            coverage_index(&cov, 0, 21),
            Ok(Some(4)),
            "second range, searched upward past the first"
        );
        assert_eq!(
            coverage_index(&cov, 0, 15),
            Ok(None),
            "the gap between ranges"
        );
        assert_eq!(
            coverage_index(&be(&[3, 1, 10]), 0, 10),
            Ok(None),
            "unknown coverage format covers nothing"
        );

        // ClassDef format 1 (dense): glyphs 10..=12 → classes 5, 6, 7.
        let cd = be(&[1, 10, 3, 5, 6, 7]);
        assert_eq!(glyph_class(&cd, 0, 11), Ok(6));
        assert_eq!(glyph_class(&cd, 0, 9), Ok(0), "below the dense range");
        assert_eq!(glyph_class(&cd, 0, 13), Ok(0), "past the dense range");
        assert_eq!(
            glyph_class(&be(&[9, 0]), 0, 10),
            Ok(0),
            "unknown class-def format is class 0"
        );
    }

    /// PairPos guards: an unknown subtable format defers to the lookup's next
    /// subtable; a coverage index past the pair-set count defers (malformed —
    /// never a decision); a FOUND record with a placement-only value adjusts
    /// no advance but still decides; a missing record defers wherever the
    /// pair search lands (between records, below every record).
    #[test]
    fn pair_pos_guards_and_value_record_shapes() {
        assert_eq!(
            pair_pos_x_advance(&be(&[3, 0]), 0, 1, 1),
            Ok(None),
            "unknown subtable format"
        );

        #[rustfmt::skip]
        let d = be(&[
            // header: format 1, coverage @12, vf1 XPlacement ONLY, vf2 0,
            // ONE pair set @20 (coverage lists two glyphs)
            1, 12, 0x0001, 0, 1, 20,
            // coverage format 1: {10, 20}
            1, 2, 10, 20,
            // pair set: records (second glyph, xplacement): (5, -9), (30, 4)
            2, 5, (-9i16) as u16, 30, 4,
        ]);
        assert_eq!(
            pair_pos_x_advance(&d, 0, 20, 5),
            Ok(None),
            "coverage index past the pair sets does not apply"
        );
        assert_eq!(
            pair_pos_x_advance(&d, 0, 10, 5),
            Ok(Some(0)),
            "a found placement-only record adjusts no advance but decides"
        );
        assert_eq!(
            pair_pos_x_advance(&d, 0, 10, 20),
            Ok(None),
            "between records: no record, does not apply"
        );
        assert_eq!(
            pair_pos_x_advance(&d, 0, 10, 4),
            Ok(None),
            "below every record: does not apply"
        );
    }

    /// PairPos format 2 bounds: a class at or past the declared class counts
    /// reads no matrix cell — the subtable defers instead of indexing off
    /// the table.
    #[test]
    fn pair_pos_format_2_clamps_class_counts() {
        #[rustfmt::skip]
        let d = be(&[
            // header: format 2, coverage @20, vf1 XAdvance, vf2 0,
            // classdefs @26 / @34, class1Count 2, class2Count 1
            2, 20, 0x0004, 0, 26, 34, 2, 1,
            // matrix (2×1): class1 0 → -5, class1 1 → 8
            (-5i16) as u16, 8,
            // coverage format 1: {10}
            1, 1, 10,
            // classdef 1, format 1: glyph 10 → class 5 (out of range!)
            1, 10, 1, 5,
            // classdef 2: unknown format → class 0
            9,
        ]);
        assert_eq!(
            pair_pos_x_advance(&d, 0, 10, 99),
            Ok(None),
            "class 5 ≥ class1Count 2 defers, reads no cell"
        );
    }

    /// Subtable ranking in `select_cmap`: usable formats on unknown platform
    /// ids still rank (format 12 over format 4), so a font with only exotic
    /// platform records keeps working.
    #[test]
    fn select_cmap_ranks_unknown_platform_subtables() {
        let mut d = be(&[0, 2]); // version, two encoding records
        d.extend(be(&[5, 0]));
        d.extend(be32(&[20])); // platform 5 → the format-4 word @20
        d.extend(be(&[5, 1]));
        d.extend(be32(&[22])); // platform 5 → the format-12 word @22
        d.extend(be(&[4, 12]));
        assert_eq!(
            select_cmap(&d, 0),
            Ok((22, 12)),
            "format 12 outranks format 4 on unknown platforms"
        );
    }
}
