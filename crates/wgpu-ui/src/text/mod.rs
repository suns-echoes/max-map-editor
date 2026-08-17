//! Text: a hand-rolled TrueType parser ([`ttf`]), an anti-aliased rasterizer
//! ([`raster`]), and line layout behind the [`TextBackend`] seam — the
//! default backend is the zero-dependency hand-rolled stack; the `cosmic`
//! feature adds a full shaping engine (cosmic-text: bidi, ligatures,
//! fallback, color emoji) a host opts into per [`Fonts`] instance.
//!
//! Layout (measuring, positioning) is pure/CPU, so widget sizing is
//! headless-testable. Rasterization produces bitmaps the GPU backend uploads
//! into the UI atlas.

pub mod ttf;

mod backend;
#[cfg(feature = "cosmic")]
mod cosmic;
mod raster;

pub use backend::{Handrolled, LineMetrics, ShapedGlyph, ShapedLine, TextBackend};
#[cfg(feature = "cosmic")]
pub use cosmic::Cosmic;
pub use raster::GlyphBitmap;
pub use ttf::{Font, FontError, Outline, Point};

use std::sync::Arc;

use crate::color::Rgba;
use crate::draw::DrawList;
use crate::geom::Vec2;

/// Identifies a font registered in a [`Fonts`] registry. Backends with
/// fallback also mint ids for faces they resolved on their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontId(pub usize);

/// The font registry every widget measures and draws through — a facade over
/// a [`TextBackend`]. [`Fonts::new`] is the hand-rolled zero-dep backend;
/// [`Fonts::with_backend`] swaps in another (e.g. `cosmic` behind its
/// feature). Owned by the application/UI context.
pub struct Fonts {
    backend: Box<dyn TextBackend>,
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}

impl Fonts {
    /// A registry over the default hand-rolled backend.
    pub fn new() -> Self {
        Self::with_backend(Handrolled::new())
    }

    /// A registry over a caller-chosen backend.
    pub fn with_backend(backend: impl TextBackend + 'static) -> Self {
        Self {
            backend: Box::new(backend),
        }
    }

    /// Parses and registers a font face, returning its id.
    pub fn add(&mut self, bytes: Vec<u8>) -> Result<FontId, FontError> {
        self.backend.add(bytes)
    }

    /// Shapes one line (no `\n`): positioned glyphs with cluster byte ranges.
    pub fn shape(&self, id: FontId, text: &str, px: f32) -> Arc<ShapedLine> {
        self.backend.shape(id, text, px)
    }

    /// Advance width of `text` at em size `px` — the layout hot path.
    pub fn measure(&self, id: FontId, text: &str, px: f32) -> f32 {
        self.backend.measure(id, text, px)
    }

    /// Vertical metrics of `id` at `px`.
    pub fn metrics(&self, id: FontId, px: f32) -> LineMetrics {
        self.backend.metrics(id, px)
    }

    /// Baseline-to-baseline distance at `px` (sugar over [`metrics`](Self::metrics)).
    pub fn line_height(&self, id: FontId, px: f32) -> f32 {
        self.backend.metrics(id, px).line_height
    }

    /// Greedy word wrap to `max_w` px (see [`TextBackend::wrap`]).
    pub fn wrap(&self, id: FontId, text: &str, px: f32, max_w: f32) -> Vec<String> {
        self.backend.wrap(id, text, px, max_w)
    }

    /// Rasterizes a glyph the backend produced, at (physical) `px`.
    pub fn rasterize(&self, id: FontId, glyph: u16, px: f32) -> GlyphBitmap {
        self.backend.rasterize(id, glyph, px)
    }

    /// Rasterizes with a quarter-pixel horizontal offset baked in (see
    /// [`TextBackend::rasterize_sub`]).
    pub fn rasterize_sub(&self, id: FontId, glyph: u16, px: f32, x_bin: u8) -> GlyphBitmap {
        self.backend.rasterize_sub(id, glyph, px, x_bin)
    }

    /// Whether the backend honors subpixel bins (see
    /// [`TextBackend::subpixel_bins`]).
    pub fn subpixel_bins(&self) -> bool {
        self.backend.subpixel_bins()
    }

    /// The hand-rolled [`Font`] behind `id` — parser-level access (glyph
    /// indices, raw metrics) that only exists on the default backend.
    ///
    /// # Panics
    /// When `id` is not registered here, or the backend is not the
    /// hand-rolled one (a shaping backend has no `Font` to hand out — go
    /// through [`shape`](Self::shape)/[`measure`](Self::measure) instead).
    pub fn get(&self, id: FontId) -> &Font {
        self.backend.handrolled_font(id).unwrap_or_else(|| {
            panic!(
                "FontId({}) has no hand-rolled Font (registry len {}) — \
                 unregistered id, a foreign Fonts' id, or a non-default \
                 backend (use shape/measure/metrics instead of get)",
                id.0,
                self.backend.font_count()
            )
        })
    }

    pub fn len(&self) -> usize {
        self.backend.font_count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Appends `text` to `dl` as one shaped line, its left end at `origin` on the
/// baseline, at em size `px`. Returns the advance width. Each glyph is
/// emitted under the face the backend RESOLVED it to (with fallback that may
/// differ from `id`).
pub fn draw_line(
    dl: &mut DrawList,
    fonts: &Fonts,
    id: FontId,
    text: &str,
    origin: Vec2,
    px: f32,
    color: Rgba,
) -> f32 {
    let line = fonts.shape(id, text, px);
    for g in &line.glyphs {
        dl.glyph(g.font, g.glyph, px, origin + g.offset, color);
    }
    line.width
}

/// A glyph positioned on a baseline: its id and pen offset (pixels) from the
/// run origin (the origin sits on the baseline, x at the run start).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedGlyph {
    pub glyph: u16,
    pub offset: Vec2,
}

impl Font {
    /// Rasterizes glyph `gid` at em size `px` (pixels).
    pub fn rasterize(&self, gid: u16, px: f32) -> GlyphBitmap {
        raster::rasterize(&self.outline(gid), self.scale(px))
    }

    /// Lays out `text` as one left-to-right line at em size `px`, applying
    /// kerning. Returns the positioned glyphs and the total advance width.
    pub fn layout_line(&self, text: &str, px: f32) -> (Vec<PositionedGlyph>, f32) {
        let scale = self.scale(px);
        let mut glyphs = Vec::new();
        let mut pen = 0.0f32;
        let mut prev: Option<u16> = None;
        for ch in text.chars() {
            let gid = self.glyph_index(ch);
            if let Some(p) = prev {
                pen += self.kern(p, gid) as f32 * scale;
            }
            glyphs.push(PositionedGlyph {
                glyph: gid,
                offset: Vec2::new(pen, 0.0),
            });
            pen += self.h_advance(gid) as f32 * scale;
            prev = Some(gid);
        }
        (glyphs, pen)
    }

    /// Width in pixels of `text` at em size `px` (advances + kerning). Computes
    /// the advance sum directly — no positioned-glyph allocation — since this
    /// runs on the per-frame layout hot path for every label/button/field.
    pub fn measure(&self, text: &str, px: f32) -> f32 {
        let scale = self.scale(px);
        let mut pen = 0.0f32;
        let mut prev: Option<u16> = None;
        for ch in text.chars() {
            let gid = self.glyph_index(ch);
            if let Some(p) = prev {
                pen += self.kern(p, gid) as f32 * scale;
            }
            pen += self.h_advance(gid) as f32 * scale;
            prev = Some(gid);
        }
        pen
    }

    /// Greedily word-wraps `text` to lines no wider than `max_w` (px) at em size
    /// `px` — [`greedy_wrap`] measured through this font.
    pub fn wrap(&self, text: &str, px: f32, max_w: f32) -> Vec<String> {
        greedy_wrap(text, max_w, |unit| self.measure(unit, px))
    }
}

/// Greedily word-wraps `text` to lines no wider than `max_w` px under
/// `measure`, breaking on spaces — and between CJK codepoints, so an unspaced
/// CJK paragraph wraps instead of overflowing as one "word" (a UAX #14-lite
/// rule). A single unbreakable word wider than `max_w` gets its own
/// (overflowing) line rather than being split mid-word; explicit `\n` always
/// forces a break. Boundary kerning across a wrap is approximated (each unit
/// measured independently), which is imperceptible for wrapping. Returns at
/// least one line (empty text → one empty line). The wrap logic every backend
/// shares — only the measurement differs.
pub fn greedy_wrap(text: &str, max_w: f32, mut measure: impl FnMut(&str) -> f32) -> Vec<String> {
    let space = measure(" ");
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut current = String::new();
        let mut width = 0.0f32;
        for (unit, spaced) in wrap_units(paragraph) {
            let ww = measure(unit);
            // Re-emit the source space when the join stays on one line;
            // CJK units glue to their neighbors without one.
            let sep_w = if spaced && !current.is_empty() {
                space
            } else {
                0.0
            };
            if current.is_empty() {
                current.push_str(unit);
                width = ww;
            } else if width + sep_w + ww <= max_w {
                if sep_w > 0.0 {
                    current.push(' ');
                }
                current.push_str(unit);
                width += sep_w + ww;
            } else {
                lines.push(std::mem::take(&mut current));
                current.push_str(unit);
                width = ww;
            }
        }
        lines.push(current);
    }
    lines
}

/// True for codepoints a line may break *between* without a space — CJK
/// ideographs, kana, Hangul, fullwidth forms (a UAX #14-lite class; enough for
/// unspaced CJK paragraphs to wrap).
fn cjk_breakable(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF        // Hangul Jamo
        | 0x2E80..=0x303F      // CJK radicals, Kangxi, CJK symbols/punctuation
        | 0x3040..=0x30FF      // Hiragana, Katakana
        | 0x3130..=0x318F      // Hangul compatibility Jamo
        | 0x31C0..=0x9FFF      // strokes, extension A, unified ideographs
        | 0xAC00..=0xD7AF      // Hangul syllables
        | 0xF900..=0xFAFF      // compatibility ideographs
        | 0xFF00..=0xFFEF      // fullwidth / halfwidth forms
        | 0x20000..=0x3FFFF    // supplementary ideographic planes
    )
}

/// Splits a paragraph into wrappable units: space-delimited words, further
/// split so each CJK-breakable codepoint is its own unit. The flag says
/// whether a source space preceded the unit (re-emitted on a same-line join).
fn wrap_units(paragraph: &str) -> Vec<(&str, bool)> {
    let mut out = Vec::new();
    for (wi, word) in paragraph.split(' ').enumerate() {
        let spaced = wi > 0;
        let mut start = 0;
        let mut first = true;
        for (i, c) in word.char_indices() {
            if cjk_breakable(c) {
                if i > start {
                    out.push((&word[start..i], spaced && first));
                    first = false;
                }
                out.push((&word[i..i + c.len_utf8()], spaced && first));
                first = false;
                start = i + c.len_utf8();
            }
        }
        if start < word.len() || first {
            out.push((&word[start..], spaced && first));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font() -> Font {
        Font::from_bytes(include_bytes!("../../assets/DejaVuSans.ttf").to_vec()).unwrap()
    }

    #[test]
    fn empty_string_measures_zero() {
        assert_eq!(font().measure("", 16.0), 0.0);
    }

    #[test]
    fn measure_scales_with_size() {
        let f = font();
        let a = f.measure("Hello", 16.0);
        let b = f.measure("Hello", 32.0);
        assert!((b - 2.0 * a).abs() < 0.01, "{a} vs {b}");
    }

    #[test]
    fn kerning_shrinks_av_pair() {
        let f = font();
        let pair = f.measure("AV", 48.0);
        let sum = f.measure("A", 48.0) + f.measure("V", 48.0);
        assert!(pair < sum, "kerned {pair} should be < unkerned {sum}");
    }

    #[test]
    fn wrap_breaks_on_width_and_keeps_words_whole() {
        let f = font();
        // Width that fits "Hello world" but not a third word.
        let two = f.measure("Hello world", 16.0);
        let lines = f.wrap("Hello world wrapping here", 16.0, two + 1.0);
        assert!(lines.len() >= 2, "wrapped into multiple lines: {lines:?}");
        // Every line fits (each word stays whole, none exceeds far past the cap).
        for l in &lines {
            assert!(!l.contains("  "), "no doubled spaces");
        }
        assert_eq!(lines.join(" "), "Hello world wrapping here", "lossless");
    }

    /// An unspaced CJK paragraph must wrap between codepoints (each glyph is a
    /// break opportunity) and rejoin losslessly *without* inserted spaces.
    #[test]
    fn wrap_breaks_unspaced_cjk() {
        let f = font();
        let text = "漢字漢字漢字漢字";
        let three = f.measure("漢字漢", 16.0);
        assert!(three > 0.0, ".notdef advances still measure");
        let lines = f.wrap(text, 16.0, three + 0.5);
        assert!(lines.len() >= 2, "wrapped: {lines:?}");
        assert_eq!(lines.concat(), text, "lossless, no inserted spaces");
        // Mixed Latin + CJK keeps the source space between the scripts.
        let mixed = f.wrap("ab 漢字", 16.0, 10_000.0);
        assert_eq!(mixed, vec!["ab 漢字".to_string()]);
    }

    #[test]
    fn wrap_honors_explicit_newlines() {
        let f = font();
        let lines = f.wrap("a\nb\nc", 16.0, 1000.0);
        assert_eq!(lines, vec!["a", "b", "c"]);
    }

    #[test]
    fn wrap_keeps_an_overlong_word_on_its_own_line() {
        let f = font();
        let lines = f.wrap("supercalifragilistic", 16.0, 10.0);
        assert_eq!(lines, vec!["supercalifragilistic"]);
    }

    /// Using a [`FontId`] from a *different* registry is a host programming
    /// error the registry must name loudly — not an opaque index panic deep
    /// in layout.
    #[test]
    #[should_panic(expected = "has no hand-rolled Font")]
    fn get_panics_with_a_diagnostic_for_foreign_ids() {
        Fonts::new().get(FontId(3));
    }

    /// Registry inventory: `len`/`is_empty` track `add`, and the returned id
    /// resolves back to the loaded font.
    #[test]
    fn registry_reports_len_and_emptiness() {
        let mut fonts = Fonts::new();
        assert!(fonts.is_empty());
        assert_eq!(fonts.len(), 0);
        let id = fonts
            .add(include_bytes!("../../assets/DejaVuSans.ttf").to_vec())
            .unwrap();
        assert!(!fonts.is_empty());
        assert_eq!(fonts.len(), 1);
        assert_eq!(fonts.get(id).units_per_em(), 2048);
    }

    /// Every CJK-breakable class is a break opportunity: a Hangul syllable, a
    /// compatibility ideograph, a fullwidth form, and a supplementary-plane
    /// ideograph each wrap onto their own line at a tiny width — and rejoin
    /// losslessly with no inserted spaces.
    #[test]
    fn wrap_breaks_between_all_cjk_classes() {
        let f = font();
        let text = "한豈Ａ𠀀";
        let lines = f.wrap(text, 16.0, 1.0);
        assert_eq!(lines.len(), 4, "each codepoint is its own unit: {lines:?}");
        assert_eq!(lines.concat(), text, "lossless, no inserted spaces");
    }

    /// A CJK codepoint directly after Latin *within one word* splits the
    /// Latin prefix into its own unit: "ab漢" may break after "ab", and when
    /// there is room the pieces rejoin without a space (none was in the
    /// source).
    #[test]
    fn wrap_splits_latin_cjk_within_a_word() {
        let f = font();
        let ab = f.measure("ab", 16.0);
        assert_eq!(
            f.wrap("ab漢", 16.0, ab + 0.5),
            vec!["ab".to_string(), "漢".to_string()]
        );
        assert_eq!(f.wrap("ab漢c", 16.0, 10_000.0), vec!["ab漢c".to_string()]);
    }

    #[test]
    fn glyphs_advance_left_to_right() {
        let f = font();
        let (glyphs, width) = f.layout_line("abc", 24.0);
        assert_eq!(glyphs.len(), 3);
        assert_eq!(glyphs[0].offset.x, 0.0);
        assert!(glyphs[1].offset.x > 0.0);
        assert!(glyphs[2].offset.x > glyphs[1].offset.x);
        assert!(width > glyphs[2].offset.x);
    }
}
