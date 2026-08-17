//! The text backend seam: everything that *interprets* text — font identity,
//! shaping, measurement, wrapping, rasterization — behind one trait, so a host
//! can swap the zero-dep hand-rolled stack for a full shaping engine (bidi,
//! ligatures, fallback, color emoji) without any widget noticing. The draw
//! side is NOT behind the seam: shaped glyphs still leave as plain
//! [`DrawCmd::Glyph`](crate::draw::DrawCmd) entries through the one existing
//! pipeline, in painter's order.
//!
//! [`Handrolled`] is the default backend — today's `Font` code verbatim; the
//! `cosmic` feature adds [`Cosmic`](super::cosmic::Cosmic) (cosmic-text).

use std::sync::Arc;

use crate::geom::Vec2;

use super::FontId;
use super::raster::GlyphBitmap;
use super::ttf::{Font, FontError};

/// One glyph of a shaped line: which face resolved it (fallback may pick a
/// face the host never registered — the backend mints ids for those), its
/// glyph id, pen offset from the run origin, its advance, and the SOURCE BYTE
/// RANGE it renders (the cluster — the caret/hit-test currency; a ligature
/// spans several bytes, so a caret can only land on cluster edges).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapedGlyph {
    pub font: FontId,
    pub glyph: u16,
    pub offset: Vec2,
    pub advance: f32,
    pub cluster: (usize, usize),
}

/// A shaped single line: positioned glyphs plus the total advance width.
/// Handed out as `Arc` so a caching backend shares one shaped result across
/// frames without cloning glyph vectors.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapedLine {
    pub glyphs: Vec<ShapedGlyph>,
    pub width: f32,
}

impl ShapedLine {
    /// The x of every caret boundary in this line, in **prefix-measure**
    /// semantics: boundary after glyph `i` sits at `offset + advance` (NOT at
    /// the next glyph's offset, which would absorb the kern between them).
    /// Yields `(byte, x)` pairs, one per cluster edge, ascending in bytes for
    /// left-to-right text.
    pub fn boundaries(&self) -> impl Iterator<Item = (usize, f32)> + '_ {
        self.glyphs
            .iter()
            .map(|g| (g.cluster.1, g.offset.x + g.advance))
    }
}

/// Vertical metrics of a face at an em size, in pixels. `ascent` is up from
/// the baseline (positive), `descent` down from it (positive).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub line_height: f32,
}

/// Everything that turns strings into glyphs. `&self` throughout (a caching
/// backend uses interior mutability), except [`add`](Self::add) — font
/// registration is setup-time. Fonts enter ONLY as raw bytes: no backend may
/// reach for system fonts, which hosts with a bundled-fonts policy depend on.
pub trait TextBackend: Send + Sync {
    /// Parses and registers a font face, returning its id.
    fn add(&mut self, bytes: Vec<u8>) -> Result<FontId, FontError>;

    /// Shapes one line (no `\n`): positioned glyphs with cluster byte ranges.
    fn shape(&self, font: FontId, text: &str, px: f32) -> Arc<ShapedLine>;

    /// Advance width only — the per-frame layout hot path; backends keep this
    /// allocation-free where they can.
    fn measure(&self, font: FontId, text: &str, px: f32) -> f32;

    /// Vertical metrics of `font` at `px`.
    fn metrics(&self, font: FontId, px: f32) -> LineMetrics;

    /// Greedy word wrap (spaces + CJK break opportunities); `\n` always
    /// breaks. A backend may substitute better break rules.
    fn wrap(&self, font: FontId, text: &str, px: f32, max_w: f32) -> Vec<String>;

    /// Rasterizes a glyph this backend produced, at (physical) `px`.
    fn rasterize(&self, font: FontId, glyph: u16, px: f32) -> GlyphBitmap;

    /// Rasterizes with a horizontal quarter-pixel offset baked into the
    /// bitmap (`x_bin` in `0..=3` = the fraction in quarters). Backends
    /// that can't subpixel-position fall back to the integer raster —
    /// the renderer only asks for non-zero bins when
    /// [`subpixel_bins`](Self::subpixel_bins) says so.
    fn rasterize_sub(&self, font: FontId, glyph: u16, px: f32, x_bin: u8) -> GlyphBitmap {
        debug_assert!(x_bin == 0, "renderer asked for a bin the backend disowned");
        self.rasterize(font, glyph, px)
    }

    /// Whether [`rasterize_sub`](Self::rasterize_sub) honors non-zero
    /// bins. When true, the renderer places glyphs at floored pen
    /// positions with the fraction rasterized in — even spacing for
    /// fractional advances (masked-input bullets made the per-glyph
    /// rounding jitter visible). When false, pens round to whole
    /// pixels — the pixel-locked default.
    fn subpixel_bins(&self) -> bool {
        false
    }

    /// Number of registered faces (fallback-minted ids included).
    fn font_count(&self) -> usize;

    /// The hand-rolled [`Font`] behind `id`, when this backend is the
    /// hand-rolled one — the escape hatch [`Fonts::get`](super::Fonts::get)
    /// rides; other backends return `None`.
    fn handrolled_font(&self, _id: FontId) -> Option<&Font> {
        None
    }
}

/// The default zero-dependency backend: the hand-rolled TTF parser,
/// per-`char` cmap lookup + GPOS kerning, left-to-right only, one face per
/// run. Cheap enough that nothing is cached.
#[derive(Default)]
pub struct Handrolled {
    fonts: Vec<Font>,
}

impl Handrolled {
    pub fn new() -> Self {
        Self::default()
    }

    fn font(&self, id: FontId) -> &Font {
        self.fonts.get(id.0).unwrap_or_else(|| {
            panic!(
                "FontId({}) is not registered in this backend (len {}) — \
                 was the theme's font id taken from a different Fonts?",
                id.0,
                self.fonts.len()
            )
        })
    }
}

impl TextBackend for Handrolled {
    fn add(&mut self, bytes: Vec<u8>) -> Result<FontId, FontError> {
        let font = Font::from_bytes(bytes)?;
        let id = FontId(self.fonts.len());
        self.fonts.push(font);
        Ok(id)
    }

    fn shape(&self, id: FontId, text: &str, px: f32) -> Arc<ShapedLine> {
        let font = self.font(id);
        let scale = font.scale(px);
        let mut glyphs = Vec::new();
        let mut pen = 0.0f32;
        let mut prev: Option<u16> = None;
        for (i, ch) in text.char_indices() {
            let gid = font.glyph_index(ch);
            if let Some(p) = prev {
                pen += font.kern(p, gid) as f32 * scale;
            }
            let advance = font.h_advance(gid) as f32 * scale;
            glyphs.push(ShapedGlyph {
                font: id,
                glyph: gid,
                offset: Vec2::new(pen, 0.0),
                advance,
                cluster: (i, i + ch.len_utf8()),
            });
            pen += advance;
            prev = Some(gid);
        }
        Arc::new(ShapedLine { glyphs, width: pen })
    }

    fn measure(&self, id: FontId, text: &str, px: f32) -> f32 {
        self.font(id).measure(text, px)
    }

    fn metrics(&self, id: FontId, px: f32) -> LineMetrics {
        let font = self.font(id);
        let scale = font.scale(px);
        LineMetrics {
            ascent: font.ascent() as f32 * scale,
            descent: -(font.descent() as f32) * scale,
            line_height: font.line_height(px),
        }
    }

    fn wrap(&self, id: FontId, text: &str, px: f32, max_w: f32) -> Vec<String> {
        self.font(id).wrap(text, px, max_w)
    }

    fn rasterize(&self, id: FontId, glyph: u16, px: f32) -> GlyphBitmap {
        self.font(id).rasterize(glyph, px)
    }

    fn font_count(&self) -> usize {
        self.fonts.len()
    }

    fn handrolled_font(&self, id: FontId) -> Option<&Font> {
        self.fonts.get(id.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend() -> Handrolled {
        let mut b = Handrolled::new();
        b.add(include_bytes!("../../assets/DejaVuSans.ttf").to_vec())
            .unwrap();
        b
    }

    /// Hand-rolled shaping is per-char: one glyph per codepoint, clusters are
    /// exactly the char byte ranges, and boundary x in prefix-measure
    /// semantics equals `Font::measure` of the prefix — the x-table the
    /// text widgets build stays byte-identical to the pre-seam one.
    #[test]
    fn handrolled_clusters_match_prefix_measures() {
        let b = backend();
        let id = FontId(0);
        let text = "AVa"; // A–V kerns in DejaVu
        let line = b.shape(id, text, 32.0);
        assert_eq!(line.glyphs.len(), 3);
        assert_eq!(line.glyphs[0].cluster, (0, 1));
        assert_eq!(line.glyphs[1].cluster, (1, 2));
        assert_eq!(line.width, b.measure(id, text, 32.0));

        let font = b.handrolled_font(id).unwrap();
        for (byte, x) in line.boundaries() {
            assert_eq!(
                x,
                font.measure(&text[..byte], 32.0),
                "boundary at byte {byte} must sit at the prefix measure"
            );
        }
    }

    /// The kern between two glyphs lives in the SECOND glyph's offset, not in
    /// the first one's boundary — offsets and boundaries differ exactly there.
    #[test]
    fn kern_shifts_offset_not_boundary() {
        let b = backend();
        let id = FontId(0);
        let line = b.shape(id, "AV", 48.0);
        let a = line.glyphs[0];
        let v = line.glyphs[1];
        assert!(
            v.offset.x < a.offset.x + a.advance,
            "V is kerned left of A's boundary ({} vs {})",
            v.offset.x,
            a.offset.x + a.advance
        );
    }

    #[test]
    fn metrics_are_positive_and_scale() {
        let b = backend();
        let m16 = b.metrics(FontId(0), 16.0);
        let m32 = b.metrics(FontId(0), 32.0);
        assert!(m16.ascent > 0.0 && m16.descent > 0.0 && m16.line_height > 0.0);
        assert!((m32.ascent - 2.0 * m16.ascent).abs() < 0.01);
        assert!((m32.line_height - 2.0 * m16.line_height).abs() < 0.01);
    }
}
