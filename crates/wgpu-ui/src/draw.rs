//! The display list: a backend-agnostic record of what to draw.
//!
//! Widgets push commands in painter's order (later commands draw on top). The
//! GPU backend ([`crate::gpu`]) walks the list, tessellates quads, and applies
//! clip rectangles as scissor regions. Because the list is plain data, widget
//! drawing can be unit-tested without a GPU by inspecting the emitted commands.

use crate::color::Rgba;
use crate::geom::{Rect, Vec2};
use crate::text::FontId;

/// Normalized texture coordinates into a texture (`0.0..=1.0`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TexRect {
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
}

impl TexRect {
    /// The whole texture.
    pub const FULL: TexRect = TexRect::new(0.0, 0.0, 1.0, 1.0);

    pub const fn new(u0: f32, v0: f32, u1: f32, v1: f32) -> Self {
        Self { u0, v0, u1, v1 }
    }
}

/// Identifies a texture the renderer can sample. [`TextureId::ATLAS`] is the
/// built-in UI atlas (solids + glyphs); other ids come from
/// [`UiRenderer::register_texture`](crate::gpu::UiRenderer::register_texture),
/// which the host calls to upload its own RGBA images (sprites, backgrounds,
/// world previews) and draw them through [`DrawList::image`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureId(pub(crate) u32);

impl TextureId {
    /// The built-in UI atlas — what solid fills and glyphs sample.
    pub const ATLAS: TextureId = TextureId(0);

    /// The raw registry index (0 is the atlas). Mostly useful for debugging.
    pub fn index(self) -> u32 {
        self.0
    }
}

/// One entry in a [`DrawList`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DrawCmd {
    /// Intersect the clip region with `rect` (pushes onto the clip stack).
    PushClip(Rect),
    /// Restore the previous clip region.
    PopClip,
    /// A solid-colored rectangle.
    Solid { rect: Rect, color: Rgba },
    /// A single glyph, tinted by `color`. `pen` is the origin on the baseline;
    /// the backend resolves the glyph bitmap (rasterizing/caching on demand) and
    /// places it relative to `pen`. `px` is the em size in logical pixels (the
    /// backend rasterizes at `px × ui_scale` for crispness).
    Glyph {
        font: FontId,
        glyph: u16,
        px: f32,
        pen: Vec2,
        color: Rgba,
    },
    /// A rectangle textured from a host-registered texture (`tex`), tinted by
    /// `color`. Use [`Rgba::WHITE`] for an untinted image. `uv` selects a
    /// sub-region (e.g. a cell of a sprite sheet); [`TexRect::FULL`] is the whole
    /// texture.
    Image {
        tex: TextureId,
        rect: Rect,
        uv: TexRect,
        color: Rgba,
    },
}

/// An ordered list of drawing commands for one frame.
#[derive(Clone, Debug, Default)]
pub struct DrawList {
    pub cmds: Vec<DrawCmd>,
}

impl DrawList {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drops all commands, keeping the allocation for reuse next frame.
    pub fn clear(&mut self) {
        self.cmds.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }

    /// Restricts following draws to `rect` (intersected with the current clip)
    /// until the matching [`DrawList::pop_clip`].
    pub fn push_clip(&mut self, rect: Rect) {
        self.cmds.push(DrawCmd::PushClip(rect));
    }

    pub fn pop_clip(&mut self) {
        self.cmds.push(DrawCmd::PopClip);
    }

    /// Fills `rect` with a solid color.
    pub fn fill_rect(&mut self, rect: Rect, color: Rgba) {
        self.cmds.push(DrawCmd::Solid { rect, color });
    }

    /// Draws a `thickness`-wide outline just inside `rect`.
    pub fn stroke_rect(&mut self, rect: Rect, thickness: f32, color: Rgba) {
        if rect.is_empty() || thickness <= 0.0 {
            return;
        }
        let t = thickness.min(rect.w).min(rect.h);
        // Top, bottom, left, right (corners covered by top/bottom).
        self.fill_rect(Rect::new(rect.x, rect.y, rect.w, t), color);
        self.fill_rect(Rect::new(rect.x, rect.bottom() - t, rect.w, t), color);
        self.fill_rect(Rect::new(rect.x, rect.y + t, t, rect.h - 2.0 * t), color);
        self.fill_rect(
            Rect::new(rect.right() - t, rect.y + t, t, rect.h - 2.0 * t),
            color,
        );
    }

    /// Draws `tex` (a host-registered texture) into `rect`, sampling `uv` and
    /// tinted by `color`. See [`DrawCmd::Image`].
    pub fn image(&mut self, tex: TextureId, rect: Rect, uv: TexRect, color: Rgba) {
        self.cmds.push(DrawCmd::Image {
            tex,
            rect,
            uv,
            color,
        });
    }

    /// Draws the whole of `tex` into `rect`, untinted — the common case for an
    /// image background or sprite.
    pub fn sprite(&mut self, tex: TextureId, rect: Rect) {
        self.image(tex, rect, TexRect::FULL, Rgba::WHITE);
    }

    /// Draws a single glyph with its baseline origin at `pen`. Prefer
    /// [`crate::text::draw_line`] for whole strings.
    pub fn glyph(&mut self, font: FontId, glyph: u16, px: f32, pen: Vec2, color: Rgba) {
        self.cmds.push(DrawCmd::Glyph {
            font,
            glyph,
            px,
            pen,
            color,
        });
    }
}

/// The present/submit half of idle gating: remembers the last frame that
/// actually reached the screen and answers whether a new draw list would look
/// any different. `DrawCmd` is plain data, so the comparison costs
/// microseconds; skipping the render is the win. The [`app`](crate::app)
/// runner gates its presents with one of these; a host driving
/// [`UiRenderer::render_into`](crate::gpu::UiRenderer::render_into) itself
/// applies the same check to its UI overlay before recording it.
///
/// The two calls are split so failure keeps the frame owed: `changed` only
/// asks, [`accept`](Self::accept) records — call it after the present/submit
/// **succeeded**. A transient render failure (a timed-out or occluded frame)
/// then leaves the gate reporting `changed` until the frame really lands.
/// The outer half of the story is [`Ui::take_dirty`](crate::ui::Ui::take_dirty),
/// which skips layout+draw entirely when nothing could have changed.
#[derive(Debug, Default)]
pub struct IdleGate {
    list: DrawList,
    size: (u32, u32),
    scale: f32,
}

impl IdleGate {
    /// A gate with no accepted frame yet: the first real frame always counts
    /// as changed.
    pub fn new() -> Self {
        Self::default()
    }

    /// True when `list`, drawn at `size` physical pixels and `scale`, differs
    /// from the last [`accept`](Self::accept)ed frame — i.e. presenting it
    /// would change what's on screen.
    pub fn changed(&self, list: &DrawList, size: (u32, u32), scale: f32) -> bool {
        self.size != size || self.scale != scale || self.list.cmds != list.cmds
    }

    /// Records `list` as the frame on screen. Call after the present/submit
    /// succeeded, not before — an unrecorded failure retries by construction.
    pub fn accept(&mut self, list: DrawList, size: (u32, u32), scale: f32) {
        self.list = list;
        self.size = size;
        self.scale = scale;
    }

    /// Forgets the accepted frame, so the next [`changed`](Self::changed) is
    /// true — for pixels that moved *without* the draw list changing (a
    /// host texture updated in place via
    /// [`update_texture`](crate::gpu::UiRenderer::update_texture)).
    pub fn invalidate(&mut self) {
        self.size = (0, 0);
        self.scale = 0.0;
        self.list.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_and_clip_record_in_order() {
        let mut dl = DrawList::new();
        dl.push_clip(Rect::new(0.0, 0.0, 10.0, 10.0));
        dl.fill_rect(Rect::new(1.0, 1.0, 2.0, 2.0), Rgba::WHITE);
        dl.pop_clip();
        assert_eq!(dl.cmds.len(), 3);
        assert!(matches!(dl.cmds[0], DrawCmd::PushClip(_)));
        assert!(matches!(dl.cmds[1], DrawCmd::Solid { .. }));
        assert!(matches!(dl.cmds[2], DrawCmd::PopClip));
    }

    #[test]
    fn stroke_rect_emits_four_edges() {
        let mut dl = DrawList::new();
        dl.stroke_rect(Rect::new(0.0, 0.0, 20.0, 20.0), 2.0, Rgba::WHITE);
        assert_eq!(dl.cmds.len(), 4);
    }

    /// `clear` empties the list for next-frame reuse, and degenerate strokes
    /// (an empty rect, a non-positive thickness) draw nothing rather than
    /// emitting inverted quads.
    #[test]
    fn clear_resets_and_degenerate_strokes_draw_nothing() {
        let mut dl = DrawList::new();
        dl.fill_rect(Rect::new(0.0, 0.0, 1.0, 1.0), Rgba::WHITE);
        assert!(!dl.is_empty());
        dl.clear();
        assert!(dl.is_empty(), "clear drops all recorded commands");

        dl.stroke_rect(Rect::ZERO, 2.0, Rgba::WHITE);
        dl.stroke_rect(Rect::new(0.0, 0.0, 5.0, 5.0), 0.0, Rgba::WHITE);
        assert!(dl.is_empty(), "degenerate strokes are no-ops");
    }

    /// `TextureId::index` exposes the raw registry slot; the built-in atlas is
    /// always slot 0.
    #[test]
    fn texture_id_index_is_the_registry_slot() {
        assert_eq!(TextureId::ATLAS.index(), 0);
        assert_eq!(TextureId(7).index(), 7);
    }

    #[test]
    fn sprite_is_a_full_uv_untinted_image() {
        let mut dl = DrawList::new();
        let tex = TextureId(7);
        dl.sprite(tex, Rect::new(0.0, 0.0, 10.0, 10.0));
        assert!(matches!(
            dl.cmds[0],
            DrawCmd::Image { tex: t, uv, color, .. }
                if t == tex && uv == TexRect::FULL && color == Rgba::WHITE
        ));
    }

    fn one_rect(x: f32) -> DrawList {
        let mut dl = DrawList::new();
        dl.fill_rect(Rect::new(x, 0.0, 5.0, 5.0), Rgba::WHITE);
        dl
    }

    /// A fresh gate owes the first frame; an accepted frame suppresses its
    /// identical re-presents; any of list, size, or scale differing makes the
    /// next frame count as changed again.
    #[test]
    fn idle_gate_accepts_then_suppresses_identical_frames() {
        let mut gate = IdleGate::new();
        let dl = one_rect(1.0);
        assert!(gate.changed(&dl, (100, 100), 1.0), "first frame is owed");

        gate.accept(dl.clone(), (100, 100), 1.0);
        assert!(
            !gate.changed(&dl, (100, 100), 1.0),
            "an identical frame is suppressed"
        );
        assert!(
            gate.changed(&one_rect(2.0), (100, 100), 1.0),
            "list differs"
        );
        assert!(gate.changed(&dl, (200, 100), 1.0), "size differs");
        assert!(gate.changed(&dl, (100, 100), 2.0), "scale differs");
    }

    /// `changed` records nothing: a frame that failed to present (so `accept`
    /// was never called) stays owed, and `invalidate` re-owes a frame whose
    /// pixels moved without the list changing (an updated host texture).
    #[test]
    fn idle_gate_failure_and_invalidate_keep_the_frame_owed() {
        let mut gate = IdleGate::new();
        let dl = one_rect(1.0);
        assert!(gate.changed(&dl, (100, 100), 1.0));
        assert!(
            gate.changed(&dl, (100, 100), 1.0),
            "asking is not recording: an unaccepted frame stays owed"
        );

        gate.accept(dl.clone(), (100, 100), 1.0);
        assert!(!gate.changed(&dl, (100, 100), 1.0));
        gate.invalidate();
        assert!(
            gate.changed(&dl, (100, 100), 1.0),
            "invalidate re-owes even an identical list"
        );
    }
}
