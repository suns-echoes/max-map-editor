//! The cosmic-text shaping backend (`cosmic` feature): real shaping — bidi,
//! ligatures/ZWJ, per-glyph font fallback across every registered face, and
//! color emoji (CBDT via swash) — behind the same [`TextBackend`] seam the
//! hand-rolled stack implements. Hosts that bundle their fonts keep their
//! guarantee: the font database starts EMPTY and faces enter only through
//! [`add`](TextBackend::add) bytes, so no system-font path exists.
//!
//! Shaping is cached per `(font, text, px)` — cosmic shaping is far too
//! expensive to redo per frame per label — behind a `Mutex` (the seam is
//! `&self`; widgets measure through shared references on the layout path).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cosmic_text::{
    Attrs, Buffer, CacheKey, CacheKeyFlags, Family, FontSystem, Metrics, Shaping, SubpixelBin,
    SwashCache, SwashContent, fontdb,
};

use crate::geom::Vec2;

use super::FontId;
use super::backend::{LineMetrics, ShapedGlyph, ShapedLine, TextBackend};
use super::raster::GlyphBitmap;
use super::ttf::FontError;

/// The cosmic-text backend. Construct with [`Cosmic::new`], register faces
/// with [`add`](TextBackend::add) (first face registered = the id text
/// defaults to), hand to [`Fonts::with_backend`](super::Fonts::with_backend).
pub struct Cosmic {
    inner: Mutex<Inner>,
}

struct Inner {
    fs: FontSystem,
    swash: SwashCache,
    /// `FontId` → fontdb face, in mint order: ids the host registered first,
    /// then ids minted for faces fallback resolved.
    faces: Vec<fontdb::ID>,
    /// The reverse map, for interning fallback faces during shaping.
    rev: HashMap<fontdb::ID, FontId>,
    /// `FontId` → primary family name (what `Attrs` selects by).
    families: Vec<String>,
    /// Shaped-line cache: `(font, text, px bits)` → shared shaped result.
    cache: HashMap<(usize, String, u32), Arc<ShapedLine>>,
}

impl Default for Cosmic {
    fn default() -> Self {
        Self::new()
    }
}

impl Cosmic {
    /// An EMPTY font system — no system fonts, no locale-picked defaults;
    /// every face this backend can use arrives through `add`.
    pub fn new() -> Self {
        let db = fontdb::Database::new();
        Self {
            inner: Mutex::new(Inner {
                fs: FontSystem::new_with_locale_and_db("en-US".into(), db),
                swash: SwashCache::new(),
                faces: Vec::new(),
                rev: HashMap::new(),
                families: Vec::new(),
                cache: HashMap::new(),
            }),
        }
    }
}

impl Inner {
    /// Interns a fontdb face under a `FontId`, minting one on first sight
    /// (how fallback faces the host never registered become addressable).
    fn intern(&mut self, id: fontdb::ID) -> FontId {
        if let Some(&fid) = self.rev.get(&id) {
            return fid;
        }
        let fid = FontId(self.faces.len());
        self.faces.push(id);
        self.rev.insert(id, fid);
        let family = self
            .fs
            .db()
            .face(id)
            .and_then(|f| f.families.first().map(|(name, _)| name.clone()))
            .unwrap_or_default();
        self.families.push(family);
        fid
    }

    fn face(&self, id: FontId) -> fontdb::ID {
        *self.faces.get(id.0).unwrap_or_else(|| {
            panic!(
                "FontId({}) is not registered in this backend (len {}) — \
                 was the theme's font id taken from a different Fonts?",
                id.0,
                self.faces.len()
            )
        })
    }

    fn shape(&mut self, id: FontId, text: &str, px: f32) -> Arc<ShapedLine> {
        let key = (id.0, text.to_owned(), px.to_bits());
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }
        let line_height = self.metrics(id, px).line_height;
        let family = self.families.get(id.0).cloned().unwrap_or_default();
        let attrs = Attrs::new().family(Family::Name(&family));
        let mut buffer = Buffer::new(&mut self.fs, Metrics::new(px, line_height));
        buffer.set_size(None, None);
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fs, false);

        let mut glyphs = Vec::new();
        let mut width = 0.0f32;
        // A single line (the seam's contract: no `\n`) shapes to one run.
        if let Some(run) = buffer.layout_runs().next() {
            width = run.line_w;
            for g in run.glyphs {
                let font = self.intern(g.font_id);
                glyphs.push(ShapedGlyph {
                    font,
                    glyph: g.glyph_id,
                    offset: Vec2::new(g.x, g.y),
                    advance: g.w,
                    cluster: (g.start, g.end),
                });
            }
        }
        let line = Arc::new(ShapedLine { glyphs, width });
        self.cache.insert(key, line.clone());
        line
    }

    fn metrics(&mut self, id: FontId, px: f32) -> LineMetrics {
        let face = self.face(id);
        let Some(font) = self.fs.get_font(face, fontdb::Weight::NORMAL) else {
            return LineMetrics {
                ascent: px * 0.8,
                descent: px * 0.2,
                line_height: px * 1.2,
            };
        };
        let m = font.as_swash().metrics(&[]);
        let scale = px / f32::from(m.units_per_em.max(1));
        let (ascent, descent, leading) = (m.ascent * scale, m.descent * scale, m.leading * scale);
        LineMetrics {
            ascent,
            descent,
            line_height: ascent + descent + leading,
        }
    }
}

impl TextBackend for Cosmic {
    fn add(&mut self, bytes: Vec<u8>) -> Result<FontId, FontError> {
        let inner = self.inner.get_mut().unwrap_or_else(|e| e.into_inner());
        // fontdb returns no ids; diff the face set to find what loaded. A
        // collection (TTC) adds several — the returned id is the first, the
        // rest stay reachable through fallback interning.
        let before: Vec<fontdb::ID> = inner.fs.db().faces().map(|f| f.id).collect();
        inner.fs.db_mut().load_font_data(bytes);
        let new: Vec<fontdb::ID> = inner
            .fs
            .db()
            .faces()
            .map(|f| f.id)
            .filter(|id| !before.contains(id))
            .collect();
        // fontdb parses more formats than the hand-rolled path; "nothing
        // loaded" maps onto the closest existing refusal.
        let &first = new.first().ok_or(FontError::NotTrueType)?;
        let fid = inner.intern(first);
        for id in new.into_iter().skip(1) {
            inner.intern(id);
        }
        // New coverage can change every fallback decision already cached.
        inner.cache.clear();
        Ok(fid)
    }

    fn shape(&self, font: FontId, text: &str, px: f32) -> Arc<ShapedLine> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.shape(font, text, px)
    }

    fn measure(&self, font: FontId, text: &str, px: f32) -> f32 {
        self.shape(font, text, px).width
    }

    fn metrics(&self, font: FontId, px: f32) -> LineMetrics {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.metrics(font, px)
    }

    fn wrap(&self, font: FontId, text: &str, px: f32, max_w: f32) -> Vec<String> {
        super::greedy_wrap(text, max_w, |unit| self.measure(font, unit, px))
    }

    fn rasterize(&self, font: FontId, glyph: u16, px: f32) -> GlyphBitmap {
        self.rasterize_sub(font, glyph, px, 0)
    }

    fn rasterize_sub(&self, font: FontId, glyph: u16, px: f32, x_bin: u8) -> GlyphBitmap {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let face = inner.face(font);
        let x_bin = match x_bin {
            1 => SubpixelBin::One,
            2 => SubpixelBin::Two,
            3 => SubpixelBin::Three,
            _ => SubpixelBin::Zero,
        };
        let key = CacheKey {
            font_id: face,
            glyph_id: glyph,
            font_size_bits: px.to_bits(),
            x_bin,
            y_bin: SubpixelBin::Zero,
            font_weight: fontdb::Weight::NORMAL,
            flags: CacheKeyFlags::empty(),
        };
        let Inner { fs, swash, .. } = &mut *inner;
        let Some(image) = swash.get_image_uncached(fs, key) else {
            return GlyphBitmap::default();
        };
        let (w, h) = (image.placement.width, image.placement.height);
        if w == 0 || h == 0 {
            return GlyphBitmap::default();
        }
        match image.content {
            SwashContent::Mask => GlyphBitmap {
                width: w,
                height: h,
                left: image.placement.left,
                top: image.placement.top,
                coverage: image.data,
                color: false,
            },
            SwashContent::Color => GlyphBitmap {
                width: w,
                height: h,
                left: image.placement.left,
                top: image.placement.top,
                coverage: image.data,
                color: true,
            },
            // Subpixel masks are for LCD filtering the toolkit doesn't do;
            // collapse to nothing rather than misrender.
            SwashContent::SubpixelMask => GlyphBitmap::default(),
        }
    }

    fn font_count(&self) -> usize {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.faces.len()
    }

    fn subpixel_bins(&self) -> bool {
        true
    }
}
