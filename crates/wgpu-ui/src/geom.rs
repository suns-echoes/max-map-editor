//! Pixel-space geometry primitives shared by layout, drawing, and hit-testing.
//!
//! Coordinates are in **logical UI pixels** — the space the widget tree, layout,
//! and events work in — with the origin at the top-left and `y` increasing
//! downward. The renderer multiplies by the UI scale at its boundary to reach
//! physical device pixels (see [`crate::gpu`]); the same primitives are reused
//! for physical-pixel quantities there (e.g. scissor rects), so a `Rect` is
//! whatever pixel space its producer works in.

use std::ops::{Add, Sub};

/// A 2D point or offset.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub const fn splat(v: f32) -> Self {
        Self { x: v, y: v }
    }
}

impl Add for Vec2 {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for Vec2 {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y)
    }
}

/// A width/height extent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub w: f32,
    pub h: f32,
}

impl Size {
    pub const ZERO: Self = Self { w: 0.0, h: 0.0 };

    pub const fn new(w: f32, h: f32) -> Self {
        Self { w, h }
    }
}

/// Per-edge padding/margin, used to inset rectangles.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Insets {
    pub const ZERO: Self = Self {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };

    pub const fn all(v: f32) -> Self {
        Self {
            left: v,
            top: v,
            right: v,
            bottom: v,
        }
    }

    /// `h` on the left/right edges, `v` on the top/bottom edges.
    pub const fn symmetric(h: f32, v: f32) -> Self {
        Self {
            left: h,
            top: v,
            right: h,
            bottom: v,
        }
    }

    pub fn horizontal(&self) -> f32 {
        self.left + self.right
    }

    pub fn vertical(&self) -> f32 {
        self.top + self.bottom
    }
}

/// An axis-aligned rectangle: top-left corner plus size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
    };

    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub const fn from_min_size(min: Vec2, size: Size) -> Self {
        Self {
            x: min.x,
            y: min.y,
            w: size.w,
            h: size.h,
        }
    }

    pub fn min(&self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }

    pub fn max(&self) -> Vec2 {
        Vec2::new(self.right(), self.bottom())
    }

    pub fn size(&self) -> Size {
        Size::new(self.w, self.h)
    }

    pub fn center(&self) -> Vec2 {
        Vec2::new(self.x + self.w * 0.5, self.y + self.h * 0.5)
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    /// True when the rectangle has no area.
    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }

    /// Half-open containment: the top/left edges are inside, bottom/right are
    /// not — so adjacent rects tile without double-counting a boundary pixel.
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    pub fn translate(&self, d: Vec2) -> Self {
        Self::new(self.x + d.x, self.y + d.y, self.w, self.h)
    }

    /// Shrinks the rectangle by `i` on each edge (clamped to non-negative size).
    pub fn inset(&self, i: Insets) -> Self {
        Self::new(
            self.x + i.left,
            self.y + i.top,
            (self.w - i.horizontal()).max(0.0),
            (self.h - i.vertical()).max(0.0),
        )
    }

    /// The overlapping region of two rectangles, or a zero-size rect when they
    /// do not overlap (check [`Rect::is_empty`]).
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let right = self.right().min(o.right());
        let bottom = self.bottom().min(o.bottom());
        Rect::new(x, y, (right - x).max(0.0), (bottom - y).max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_is_half_open() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(r.contains(Vec2::new(0.0, 0.0)));
        assert!(r.contains(Vec2::new(9.9, 9.9)));
        assert!(!r.contains(Vec2::new(10.0, 5.0)));
        assert!(!r.contains(Vec2::new(5.0, 10.0)));
    }

    #[test]
    fn intersect_overlap_and_disjoint() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        assert_eq!(a.intersect(&b), Rect::new(5.0, 5.0, 5.0, 5.0));

        let c = Rect::new(20.0, 20.0, 5.0, 5.0);
        assert!(a.intersect(&c).is_empty());
    }

    #[test]
    fn inset_clamps_to_zero() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert_eq!(r.inset(Insets::all(2.0)), Rect::new(2.0, 2.0, 6.0, 6.0));
        assert!(r.inset(Insets::all(100.0)).is_empty());
    }

    /// `splat` duplicates one scalar into both axes — a square offset/extent.
    #[test]
    fn splat_fills_both_axes() {
        assert_eq!(Vec2::splat(3.5), Vec2::new(3.5, 3.5));
    }

    /// `max` is the bottom-right corner (the counterpart of `min`), and
    /// `translate` moves the origin without touching the extent.
    #[test]
    fn rect_max_corner_and_translate() {
        let r = Rect::new(1.0, 2.0, 10.0, 20.0);
        assert_eq!(r.max(), Vec2::new(11.0, 22.0), "max = min + size");
        let moved = r.translate(Vec2::new(5.0, -2.0));
        assert_eq!(moved, Rect::new(6.0, 0.0, 10.0, 20.0));
        assert_eq!(moved.size(), r.size(), "translation preserves the extent");
    }
}
