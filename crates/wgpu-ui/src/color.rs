//! Colors.
//!
//! Colors are stored as 8-bit **sRGB** RGBA — the values you would write in CSS
//! or a palette editor. The renderer converts sRGB → linear in the shader so
//! blending (anti-aliased glyph edges in particular) happens in linear light and
//! the sRGB render target re-encodes to exactly the value you specified.

/// An 8-bit-per-channel sRGB color with straight (non-premultiplied) alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);

    // `rgb`/`rgba` is the deliberate, ergonomic constructor pair (cf. CSS); the
    // lint only fires because `rgba` case-matches the type name.
    #[allow(clippy::self_named_constructors)]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::rgba(r, g, b, 255)
    }

    /// Parses `0xRRGGBB` (opaque) — handy for theme constants.
    pub const fn hex(rgb: u32) -> Self {
        Self::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
    }

    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    /// Straight sRGB channels in `0.0..=1.0` (what vertices carry; the shader
    /// decodes to linear).
    pub fn to_srgb_f32(self) -> [f32; 4] {
        [
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            self.a as f32 / 255.0,
        ]
    }

    /// Linear-light channels in `0.0..=1.0` (what a `LoadOp::Clear` wants, since
    /// the clear value bypasses the shader but is still re-encoded by the
    /// sRGB target). Alpha stays linear.
    pub fn to_linear_f32(self) -> [f32; 4] {
        let s = self.to_srgb_f32();
        [
            srgb_to_linear(s[0]),
            srgb_to_linear(s[1]),
            srgb_to_linear(s[2]),
            s[3],
        ]
    }

    /// Linear interpolation between two colors in sRGB space; `t` is clamped to
    /// `0.0..=1.0`.
    pub fn lerp(self, o: Rgba, t: f32) -> Rgba {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Rgba::rgba(
            mix(self.r, o.r),
            mix(self.g, o.g),
            mix(self.b, o.b),
            mix(self.a, o.a),
        )
    }

    /// Multiplies the RGB channels by `factor` (alpha untouched), clamped to
    /// `0..=255`. The basis for inset/outset frames: `shade(1.15)` lightens an
    /// edge, `shade(0.6)` darkens the opposite one.
    pub fn shade(self, factor: f32) -> Rgba {
        let ch = |c: u8| (c as f32 * factor).round().clamp(0.0, 255.0) as u8;
        Rgba::rgba(ch(self.r), ch(self.g), ch(self.b), self.a)
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_unpacks_channels() {
        assert_eq!(Rgba::hex(0x1A2B3C), Rgba::rgb(0x1A, 0x2B, 0x3C));
    }

    #[test]
    fn pure_channels_roundtrip_to_linear_extremes() {
        // sRGB 0 and 255 map to linear 0.0 and 1.0 exactly.
        assert_eq!(Rgba::rgb(255, 0, 0).to_linear_f32(), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(Rgba::BLACK.to_linear_f32(), [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn shade_clamps() {
        assert_eq!(Rgba::rgb(200, 100, 50).shade(0.5), Rgba::rgb(100, 50, 25));
        assert_eq!(Rgba::rgb(200, 100, 50).shade(2.0), Rgba::rgb(255, 200, 100));
    }

    #[test]
    fn lerp_endpoints_and_midpoint() {
        let a = Rgba::rgb(0, 0, 0);
        let b = Rgba::rgb(255, 100, 50);
        assert_eq!(a.lerp(b, 0.0), a);
        assert_eq!(a.lerp(b, 1.0), b);
        assert_eq!(a.lerp(b, 0.5), Rgba::rgb(128, 50, 25));
    }
}
