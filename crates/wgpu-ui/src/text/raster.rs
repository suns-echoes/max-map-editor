//! Anti-aliased glyph rasterizer: a glyph [`Outline`] (font units) scaled to a
//! target pixel size becomes a coverage bitmap.
//!
//! Quadratic Béziers are flattened to polylines by adaptive subdivision, then
//! filled with the non-zero winding rule. Anti-aliasing is analytic across each
//! scanline (exact horizontal span coverage) and 4× super-sampled vertically —
//! a simple, robust scheme that is correct for holes (opposite-wound contours)
//! and good enough for crisp UI text. Glyphs are cached per (glyph, size) by the
//! caller, so per-glyph cost is paid once.

use super::ttf::Outline;

/// Vertical super-samples per output row.
const SUBSAMPLES: usize = 4;
/// Bézier flattening tolerance, in device pixels.
const FLATTEN_TOL: f32 = 0.25;
/// Transparent border around each glyph (avoids edge clipping / atlas bleed).
const PAD: i32 = 1;

/// A rasterized glyph: a bitmap plus its placement relative to the pen origin
/// on the baseline. Coverage by default; a shaping backend may hand back a
/// COLOR glyph (emoji), flagged so the renderer draws it untinted.
#[derive(Clone, Debug, Default)]
pub struct GlyphBitmap {
    pub width: u32,
    pub height: u32,
    /// X offset from the pen origin to the bitmap's left edge.
    pub left: i32,
    /// Y offset from the baseline *up* to the bitmap's top edge.
    pub top: i32,
    /// Row-major pixels. Coverage: `width * height` bytes (0 = transparent,
    /// 255 = ink), tinted by the draw color. Color (`color == true`):
    /// `width * height * 4` RGBA bytes, drawn as-is.
    pub coverage: Vec<u8>,
    /// True when `coverage` holds RGBA color pixels (an emoji) instead of
    /// single-channel coverage.
    pub color: bool,
}

impl GlyphBitmap {
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// Rasterizes `outline` (font units) at `scale` (pixels per font unit).
pub fn rasterize(outline: &Outline, scale: f32) -> GlyphBitmap {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for c in &outline.contours {
        for p in c {
            let (dx, dy) = (p.x * scale, p.y * scale);
            min_x = min_x.min(dx);
            min_y = min_y.min(dy);
            max_x = max_x.max(dx);
            max_y = max_y.max(dy);
        }
    }
    if !min_x.is_finite() || max_x < min_x {
        return GlyphBitmap::default();
    }

    let left = min_x.floor() as i32 - PAD;
    let top = max_y.ceil() as i32 + PAD;
    let right = max_x.ceil() as i32 + PAD;
    let bottom = min_y.floor() as i32 - PAD;
    let width = (right - left).max(0) as u32;
    let height = (top - bottom).max(0) as u32;
    if width == 0 || height == 0 {
        return GlyphBitmap::default();
    }

    // Flatten contours into device-space polylines (y flipped: font up -> down).
    let (ox, oy) = (left as f32, top as f32);
    let mut polys: Vec<Vec<(f32, f32)>> = Vec::with_capacity(outline.contours.len());
    for c in &outline.contours {
        let dpts: Vec<DPoint> = c
            .iter()
            .map(|p| DPoint {
                x: p.x * scale - ox,
                y: oy - p.y * scale,
                on: p.on_curve,
            })
            .collect();
        let mut poly = Vec::new();
        build_polyline(&dpts, &mut poly);
        if poly.len() >= 2 {
            polys.push(poly);
        }
    }

    let coverage = fill(width as usize, height as usize, &polys);
    GlyphBitmap {
        color: false,
        width,
        height,
        left,
        top,
        coverage,
    }
}

#[derive(Clone, Copy)]
struct DPoint {
    x: f32,
    y: f32,
    on: bool,
}

/// Converts a TrueType contour (on/off-curve points) into a flattened polyline.
fn build_polyline(pts: &[DPoint], out: &mut Vec<(f32, f32)>) {
    let n = pts.len();
    if n == 0 {
        return;
    }
    // Rotate so the contour starts on-curve; synthesize a start if none is.
    let seq: Vec<DPoint> = match pts.iter().position(|p| p.on) {
        Some(i) => (0..n).map(|k| pts[(i + k) % n]).collect(),
        None => {
            let mid = DPoint {
                x: (pts[0].x + pts[n - 1].x) * 0.5,
                y: (pts[0].y + pts[n - 1].y) * 0.5,
                on: true,
            };
            std::iter::once(mid).chain(pts.iter().copied()).collect()
        }
    };

    // Insert the implied on-curve midpoint between consecutive off-curve points.
    let mut e: Vec<DPoint> = Vec::with_capacity(seq.len());
    for &p in &seq {
        if !p.on
            && let Some(&last) = e.last()
            && !last.on
        {
            e.push(DPoint {
                x: (last.x + p.x) * 0.5,
                y: (last.y + p.y) * 0.5,
                on: true,
            });
        }
        e.push(p);
    }

    let m = e.len();
    out.push((e[0].x, e[0].y));
    let mut cur = (e[0].x, e[0].y);
    let mut k = 1;
    while k < m {
        let p = e[k];
        if p.on {
            out.push((p.x, p.y));
            cur = (p.x, p.y);
            k += 1;
        } else {
            let ctrl = (p.x, p.y);
            // The segment ends at the next on-curve point, wrapping to the start.
            let end = if k + 1 < m {
                (e[k + 1].x, e[k + 1].y)
            } else {
                (e[0].x, e[0].y)
            };
            flatten_quad(cur, ctrl, end, 0, out);
            cur = end;
            k += 2;
        }
    }
}

/// Recursively subdivides a quadratic Bézier until flat within [`FLATTEN_TOL`].
fn flatten_quad(
    p0: (f32, f32),
    c: (f32, f32),
    p1: (f32, f32),
    depth: u32,
    out: &mut Vec<(f32, f32)>,
) {
    let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
    let cross = (c.0 - p0.0) * dy - (c.1 - p0.1) * dx;
    let len = (dx * dx + dy * dy).sqrt();
    let dev = if len > 1e-6 {
        cross.abs() / len
    } else {
        ((c.0 - p0.0).powi(2) + (c.1 - p0.1).powi(2)).sqrt()
    };
    if depth >= 16 || dev <= FLATTEN_TOL {
        out.push(p1);
        return;
    }
    let p01 = ((p0.0 + c.0) * 0.5, (p0.1 + c.1) * 0.5);
    let c1 = ((c.0 + p1.0) * 0.5, (c.1 + p1.1) * 0.5);
    let mid = ((p01.0 + c1.0) * 0.5, (p01.1 + c1.1) * 0.5);
    flatten_quad(p0, p01, mid, depth + 1, out);
    flatten_quad(mid, c1, p1, depth + 1, out);
}

/// Fills closed `polys` into a `width`x`height` coverage bitmap (non-zero rule).
fn fill(width: usize, height: usize, polys: &[Vec<(f32, f32)>]) -> Vec<u8> {
    let mut cov = vec![0f32; width * height];
    let inv = 1.0 / SUBSAMPLES as f32;
    let mut crossings: Vec<(f32, i32)> = Vec::new();

    for row in 0..height {
        for s in 0..SUBSAMPLES {
            let yc = row as f32 + (s as f32 + 0.5) * inv;
            crossings.clear();
            for poly in polys {
                let n = poly.len();
                for i in 0..n {
                    let (x0, y0) = poly[i];
                    let (x1, y1) = poly[(i + 1) % n];
                    if (y0 <= yc && y1 > yc) || (y1 <= yc && y0 > yc) {
                        let t = (yc - y0) / (y1 - y0);
                        crossings.push((x0 + t * (x1 - x0), if y1 > y0 { 1 } else { -1 }));
                    }
                }
            }
            if crossings.is_empty() {
                continue;
            }
            crossings.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

            let mut wind = 0;
            let mut span_start = 0.0;
            for &(x, dir) in &crossings {
                let before = wind;
                wind += dir;
                if before == 0 && wind != 0 {
                    span_start = x;
                } else if before != 0 && wind == 0 {
                    add_span(&mut cov, width, row, span_start, x, inv);
                }
            }
        }
    }

    cov.iter()
        .map(|&c| (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
        .collect()
}

/// Adds `weight`-scaled horizontal coverage for `[xa, xb)` on `row`.
fn add_span(cov: &mut [f32], width: usize, row: usize, xa: f32, xb: f32, weight: f32) {
    let xa = xa.max(0.0);
    let xb = xb.min(width as f32);
    if xb <= xa {
        return;
    }
    let base = row * width;
    let first = xa.floor() as usize;
    let last = ((xb.ceil() as usize).saturating_sub(1)).min(width - 1);
    if first >= width {
        return;
    }
    if first == last {
        cov[base + first] += weight * (xb - xa);
    } else {
        cov[base + first] += weight * ((first as f32 + 1.0) - xa);
        for px in (first + 1)..last {
            cov[base + px] += weight;
        }
        cov[base + last] += weight * (xb - last as f32);
    }
}

#[cfg(test)]
mod tests {
    use super::super::ttf::{Font, Point};
    use super::*;

    fn font() -> Font {
        Font::from_bytes(include_bytes!("../../assets/DejaVuSans.ttf").to_vec()).unwrap()
    }

    #[test]
    fn rasterizes_glyph_with_ink() {
        let f = font();
        let g = f.glyph_index('A');
        let bmp = rasterize(&f.outline(g), f.scale(48.0));
        assert!(!bmp.is_empty());
        assert_eq!(bmp.coverage.len(), (bmp.width * bmp.height) as usize);
        // Some fully-covered ink and some transparent background.
        assert!(bmp.coverage.iter().any(|&c| c > 200));
        assert!(bmp.coverage.contains(&0));
        // 'A' near 48px em is roughly cap-height tall.
        assert!(
            bmp.height >= 24 && bmp.height <= 56,
            "height {}",
            bmp.height
        );
    }

    #[test]
    fn blank_glyph_is_empty() {
        let f = font();
        let bmp = rasterize(&f.outline(f.glyph_index(' ')), f.scale(48.0));
        assert!(bmp.is_empty());
    }

    #[test]
    fn anti_aliasing_produces_partial_coverage() {
        let f = font();
        // A round glyph should have soft (partial-coverage) edge pixels.
        let bmp = rasterize(&f.outline(f.glyph_index('o')), f.scale(48.0));
        assert!(bmp.coverage.iter().any(|&c| c > 0 && c < 255));
    }

    fn on(x: f32, y: f32) -> Point {
        Point {
            x,
            y,
            on_curve: true,
        }
    }

    fn off(x: f32, y: f32) -> Point {
        Point {
            x,
            y,
            on_curve: false,
        }
    }

    /// A malformed empty contour must not spoil its siblings: the rasterizer
    /// skips it and fills the real contour exactly as if it weren't there.
    #[test]
    fn empty_contours_are_skipped() {
        let square = vec![
            on(0.0, 0.0),
            on(100.0, 0.0),
            on(100.0, 100.0),
            on(0.0, 100.0),
        ];
        let with_empty = Outline {
            contours: vec![square.clone(), Vec::new()],
        };
        let plain = Outline {
            contours: vec![square],
        };
        let a = rasterize(&with_empty, 0.25);
        let b = rasterize(&plain, 0.25);
        assert_eq!((a.width, a.height), (b.width, b.height));
        assert_eq!(
            a.coverage, b.coverage,
            "the empty contour contributes nothing"
        );
        assert!(
            a.coverage.contains(&255),
            "the square still fills solid ink"
        );
    }

    /// TrueType allows a contour with NO on-curve point (a closed quadratic
    /// ring — some fonts' 'o' is built this way): the rasterizer synthesizes
    /// the start anchor from the wrap-around midpoint and still fills ink.
    #[test]
    fn all_off_curve_contour_still_fills() {
        let ring = Outline {
            contours: vec![vec![
                off(50.0, 0.0),
                off(100.0, 50.0),
                off(50.0, 100.0),
                off(0.0, 50.0),
            ]],
        };
        let bmp = rasterize(&ring, 0.5);
        assert!(!bmp.is_empty());
        let center = bmp.coverage[(bmp.height / 2 * bmp.width + bmp.width / 2) as usize];
        assert_eq!(center, 255, "the ring's center is solid ink");
        assert!(
            bmp.coverage.iter().any(|&c| c > 0 && c < 255),
            "the curved rim anti-aliases"
        );
    }

    /// A degenerate quadratic whose endpoints coincide has no chord to
    /// measure flatness against; deviation falls back to the control-point
    /// distance, so flattening still terminates and the surrounding contour
    /// fills normally (the zero-area spike itself adds no ink).
    #[test]
    fn degenerate_quadratic_flattens_and_terminates() {
        let spike_then_square = Outline {
            contours: vec![vec![
                on(0.0, 0.0),
                off(40.0, 120.0), // curve out and back to the same point
                on(0.0, 0.0),
                on(80.0, 0.0),
                on(80.0, 80.0),
                on(0.0, 80.0),
            ]],
        };
        let bmp = rasterize(&spike_then_square, 1.0);
        assert!(!bmp.is_empty());
        // Font (40, 40) — the middle of the square — maps to device
        // (40 + PAD, top - 40) with top = ceil(120) + PAD.
        let (x, y) = (40 + PAD as u32, (120 + PAD - 40) as u32);
        assert_eq!(
            bmp.coverage[(y * bmp.width + x) as usize],
            255,
            "the square body fills despite the degenerate curve"
        );
    }
}
