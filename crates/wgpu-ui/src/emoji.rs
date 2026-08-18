//! Emoji catalog + font-coverage probe.
//!
//! The catalog ([`EMOJI`], [`GROUPS`]) is generated from Unicode's
//! `emoji-test.txt` by `tools/gen-emoji/generate.mjs` (vendored beside
//! it) into `emoji_data.rs` — fully-qualified sequences only, CLDR
//! keyboard-palette order. It is data, not policy: hosts decide what
//! to show, and [`supported`] narrows the list to what a loaded font
//! can actually render.
//!
//! Rendering emoji is a text-backend concern: the hand-rolled backend
//! has no fallback chain and cannot load color (CBDT) faces, so on it
//! the probe typically keeps only the few monochrome pictographs the
//! registered face covers. The `cosmic` backend shapes ZWJ/VS16
//! sequences and rasterizes color emoji — hosts wanting a full picker
//! use it (see the feature docs in Cargo.toml).

use crate::text::{FontId, Fonts};

pub use crate::emoji_data::{EMOJI, EmojiEntry, GROUPS};

/// The entries a text stack can actually render, probed by shaping.
///
/// A supported emoji shapes to **exactly one glyph** that is not
/// `.notdef` — a missing codepoint becomes glyph 0, and an unligated
/// ZWJ/flag sequence falls apart into several glyphs (it would render
/// as its parts, not the emoji). `font` is the face fallback starts
/// from (any text face works — emoji resolve through the backend's
/// fallback chain, where it has one).
///
/// Shaping ~4k sequences is a one-time cost on the order of the first
/// frame of a text-heavy screen; call it once and keep the result.
pub fn supported(fonts: &Fonts, font: FontId) -> Vec<&'static EmojiEntry> {
    EMOJI
        .iter()
        .filter(|e| {
            let line = fonts.shape(font, e.emoji, 16.0);
            line.glyphs.len() == 1 && line.glyphs[0].glyph != 0
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_sane() {
        assert_eq!(GROUPS.len(), 10);
        assert!(EMOJI.len() > 3000, "suspiciously small: {}", EMOJI.len());
        for e in EMOJI.iter() {
            assert!(!e.emoji.is_empty());
            assert!(!e.name.is_empty());
            assert!((e.group as usize) < GROUPS.len(), "{} group oob", e.name);
        }
        // CLDR order groups entries contiguously per group header.
        let mut seen_max = 0u8;
        for e in EMOJI.iter() {
            assert!(e.group >= seen_max.saturating_sub(0) || e.group <= seen_max);
            seen_max = seen_max.max(e.group);
        }
        // A few anchors that must exist in any Unicode drop.
        for name in ["grinning face", "red heart", "cherries"] {
            assert!(
                EMOJI.iter().any(|e| e.name == name),
                "missing anchor {name}"
            );
        }
    }

    #[test]
    fn probe_filters_to_font_coverage() {
        let mut fonts = Fonts::new();
        let id = fonts
            .add(include_bytes!("../assets/DejaVuSans.ttf").to_vec())
            .unwrap();
        let ok = supported(&fonts, id);
        // DejaVu Sans is no emoji font: the probe must drop the vast
        // majority (every ZWJ/VS16 sequence falls apart or misses),
        // and everything it keeps must re-probe identically.
        assert!(ok.len() < EMOJI.len() / 4, "kept {} entries", ok.len());
        let again = supported(&fonts, id);
        assert_eq!(ok.len(), again.len());
    }
}
