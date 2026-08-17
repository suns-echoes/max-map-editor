//! A floating [`Window`]: a titled panel that can be dragged by its titlebar and
//! resized from its bottom-right grip. It positions itself at its own
//! `pos`/`size` (ignoring the arranged rect), so several can float in a `Stack`.

use crate::draw::DrawList;
use crate::event::{Event, PointerButton};
use crate::geom::{Rect, Size, Vec2};
use crate::interact::{CursorIcon, WidgetId, next_id};
use crate::theme::TextRole;
use crate::widget::{DrawCtx, EventCtx, LayoutCtx, Semantics, Widget, kind_of};

const GRIP: f32 = 14.0;
const MIN_W: f32 = 100.0;
const MIN_H: f32 = 60.0;

/// Snaps a logical coordinate to a whole physical pixel (`round(v*scale)/scale`)
/// so the window aligns to the device pixel grid at any DPI factor.
fn snap_phys(v: f32, scale: f32) -> f32 {
    if scale > 0.0 {
        (v * scale).round() / scale
    } else {
        v
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    None,
    Move(Vec2),
    Resize,
}

/// Left pad of the titlebar's title text (a touch wider than the content pad).
const TITLE_PAD: f32 = 12.0;

/// A floating window around a single content widget. **Movable by default**
/// (drag the titlebar); resizable from the bottom-right grip unless
/// [`Window::resizable(false)`](Window::resizable). Opt into
/// [`auto_size`](Window::auto_size) to fit the content and [`centered`](Window::centered)
/// to centre it until the user drags it — the makings of a movable dialog.
#[must_use]
pub struct Window {
    id: WidgetId,
    title: String,
    content: Box<dyn Widget>,
    pos: Vec2,
    size: Size,
    titlebar_h: f32,
    /// Overrides the theme-resolved content pad (see [`padding`](Self::padding)).
    fixed_pad: Option<f32>,
    /// The resolved content inset on all four sides (below the titlebar):
    /// `fixed_pad`, else the theme's `metrics().pad` — refreshed at measure.
    pad: f32,
    drag: Drag,
    resizable: bool,
    auto_size: bool,
    /// Centre in the arranged rect until the user moves it.
    center: bool,
    moved: bool,
    rect: Rect,
}

impl Window {
    pub fn new(title: impl Into<String>, content: impl Widget + 'static) -> Self {
        Self {
            id: next_id(),
            title: title.into(),
            content: Box::new(content),
            pos: Vec2::new(20.0, 20.0),
            size: Size::new(200.0, 150.0),
            titlebar_h: 22.0,
            fixed_pad: None,
            pad: 8.0,
            drag: Drag::None,
            resizable: true,
            auto_size: false,
            center: false,
            moved: false,
            rect: Rect::ZERO,
        }
    }

    pub fn pos(mut self, pos: Vec2) -> Self {
        self.pos = pos;
        self
    }

    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }

    /// Whether the bottom-right resize grip is shown/active (default `true`).
    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// Overrides the content pad (all four sides, below the titlebar). The
    /// default follows the theme's `metrics().pad`; a host squeezing a dense
    /// tool window can go tighter.
    pub fn padding(mut self, pad: f32) -> Self {
        self.fixed_pad = Some(pad.max(0.0));
        self
    }

    /// Size the window to fit its content (titlebar + padded content) instead of
    /// the fixed [`size`](Window::size).
    pub fn auto_size(mut self) -> Self {
        self.auto_size = true;
        self
    }

    /// Centre the window in its arranged rect on each layout — until the user
    /// drags it, after which it stays put. Good for a freshly-opened dialog.
    pub fn centered(mut self) -> Self {
        self.center = true;
        self
    }

    pub fn id(&self) -> WidgetId {
        self.id
    }

    pub fn position(&self) -> Vec2 {
        self.pos
    }

    pub fn dimensions(&self) -> Size {
        self.size
    }

    fn titlebar(&self) -> Rect {
        Rect::new(self.pos.x, self.pos.y, self.size.w, self.titlebar_h)
    }

    fn grip(&self) -> Rect {
        Rect::new(
            self.pos.x + self.size.w - GRIP,
            self.pos.y + self.size.h - GRIP,
            GRIP,
            GRIP,
        )
    }

    fn content_rect(&self) -> Rect {
        // `pad` insets the content on all four sides, including between the
        // titlebar and the content (top).
        Rect::new(
            self.pos.x + self.pad,
            self.pos.y + self.titlebar_h + self.pad,
            (self.size.w - 2.0 * self.pad).max(0.0),
            (self.size.h - self.titlebar_h - 2.0 * self.pad).max(0.0),
        )
    }
}

impl Widget for Window {
    fn semantics(&self) -> Semantics<'_> {
        Semantics::labeled(kind_of::<Self>(), &self.title)
    }

    fn measure(&mut self, avail: Size, ctx: &mut LayoutCtx) -> Size {
        self.titlebar_h = ctx.theme.metrics().titlebar;
        self.pad = self.fixed_pad.unwrap_or(ctx.theme.metrics().pad);
        if self.auto_size {
            // Fit the content: measure it, then wrap with the titlebar + padding
            // (the same `pad` on all four sides, top included).
            let inner = Size::new(
                (avail.w - 2.0 * self.pad).max(0.0),
                (avail.h - self.titlebar_h - 2.0 * self.pad).max(0.0),
            );
            let c = self.content.measure(inner, ctx);
            self.size = Size::new(c.w + 2.0 * self.pad, c.h + self.titlebar_h + 2.0 * self.pad);
        } else {
            let cr = self.content_rect();
            self.content.measure(cr.size(), ctx);
        }
        self.size
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        // Pixel-lock to whole *physical* pixels: the window (and so everything
        // laid out relative to it) lands on the device grid, and dragging moves
        // it in rigid whole-pixel steps. Because a whole-pixel translation never
        // changes any child's sub-pixel phase, elements stay put relative to one
        // another instead of drifting as the window slides. Size is snapped too,
        // so the right/bottom frame edges stay crisp.
        let s = ctx.scale;
        self.size = Size::new(snap_phys(self.size.w, s), snap_phys(self.size.h, s));
        // Centre in the arranged rect until the user has dragged it.
        if self.center && !self.moved {
            self.pos = Vec2::new(
                (rect.x + (rect.w - self.size.w) * 0.5).max(rect.x),
                (rect.y + (rect.h - self.size.h) * 0.5).max(rect.y),
            );
        }
        // Keep the window reachable: a titlebar dragged past an edge (or
        // stranded by a viewport shrink) must keep enough visible to grab —
        // the same rule `Workspace::clamp_floating` applies to its floats.
        // Clamping here (not in the drag) also covers resize with no extra
        // event handling.
        const MIN_VISIBLE: f32 = 32.0;
        self.pos.x = self.pos.x.clamp(
            rect.x + MIN_VISIBLE - self.size.w,
            (rect.right() - MIN_VISIBLE).max(rect.x + MIN_VISIBLE - self.size.w),
        );
        self.pos.y = self
            .pos
            .y
            .clamp(rect.y, (rect.bottom() - MIN_VISIBLE).max(rect.y));
        self.pos = Vec2::new(snap_phys(self.pos.x, s), snap_phys(self.pos.y, s));
        // Float at our own position/size, ignoring the parent's slot.
        self.rect = Rect::from_min_size(self.pos, self.size);
        let cr = self.content_rect();
        self.content.arrange(cr, ctx);
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        // Frame, titlebar and grip are chrome — base pass only (see `Panel::draw`),
        // so the overlay pass doesn't erase base-only content (e.g. a closed
        // `Select` inside the window). Content draws in both passes.
        if ctx.is_base() {
            ctx.theme.panel(dl, self.rect);
            // Titlebar band (a theme may blend it into the panel) + title. No
            // extra frame — the panel's bevel already encloses the titlebar.
            ctx.theme.titlebar(dl, self.titlebar());
            let px = ctx.theme.font_px(TextRole::Title);
            let tb = self.titlebar();
            // Titlebar title: a dedicated left pad (wider than content pad), the
            // title vertically centred in the band.
            let baseline = Vec2::new(tb.x + TITLE_PAD, tb.center().y + px * 0.34);
            ctx.theme
                .text(dl, ctx.fonts, baseline, &self.title, TextRole::Title);
        }

        self.content.draw(dl, ctx);

        if ctx.is_base() && self.resizable {
            // Resize grip (three diagonal ticks).
            let g = self.grip();
            for i in 1..=3 {
                let o = i as f32 * 3.0;
                dl.fill_rect(
                    Rect::new(g.right() - o, g.bottom() - 3.0, 2.0, 2.0),
                    ctx.theme.ink_dim(),
                );
                dl.fill_rect(
                    Rect::new(g.right() - 3.0, g.bottom() - o, 2.0, 2.0),
                    ctx.theme.ink_dim(),
                );
            }
        }
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        // Content first (so its widgets get their clicks).
        if self.content.event(ev, ctx) {
            return true;
        }
        match ev {
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                ..
            } if ctx.is_target(self.id) => {
                ctx.consume_pointer();
                if self.resizable && self.grip().contains(ctx.pointer) {
                    self.drag = Drag::Resize;
                    ctx.capture(self.id);
                } else if self.titlebar().contains(ctx.pointer) {
                    self.drag = Drag::Move(ctx.pointer - self.pos);
                    self.moved = true;
                    ctx.capture(self.id);
                }
                true
            }
            Event::PointerMoved { .. } if self.drag != Drag::None && ctx.is_target(self.id) => {
                match self.drag {
                    Drag::Move(off) => self.pos = ctx.pointer - off,
                    Drag::Resize => {
                        self.size = Size::new(
                            (ctx.pointer.x - self.pos.x).max(MIN_W),
                            (ctx.pointer.y - self.pos.y).max(MIN_H),
                        );
                    }
                    Drag::None => {}
                }
                ctx.consume_pointer();
                true
            }
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: false,
                ..
            } if self.drag != Drag::None => {
                self.drag = Drag::None;
                ctx.consume_pointer();
                true
            }
            // Opaque: swallow any other pointer event, but only when this window
            // is the topmost target — otherwise two overlapping windows would
            // both "handle" a click in the shared region. Gating on `is_target`
            // (the central topmost-hit resolution) leaves the click to the front
            // window alone.
            _ if ev.is_pointer() && ctx.is_target(self.id) => {
                ctx.consume_pointer();
                true
            }
            _ => false,
        }
    }

    /// Diagonal arrows over the grip and throughout a resize drag (the pointer
    /// outruns the grip mid-drag); the arrow otherwise — a titlebar move keeps
    /// the plain arrow, the desktop convention.
    fn cursor(&self, pos: Vec2) -> CursorIcon {
        let over_grip = self.resizable && self.grip().contains(pos);
        if self.drag == Drag::Resize || (self.drag == Drag::None && over_grip) {
            CursorIcon::ResizeNWSE
        } else {
            CursorIcon::Default
        }
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn id(&self) -> WidgetId {
        self.id
    }

    fn child_count(&self) -> usize {
        1
    }

    fn child(&self, i: usize) -> Option<&dyn Widget> {
        (i == 0).then_some(self.content.as_ref())
    }

    fn child_mut(&mut self, i: usize) -> Option<&mut dyn Widget> {
        (i == 0).then_some(self.content.as_mut())
    }

    fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
        if !self.rect.contains(pos) {
            return None;
        }
        if self.content_rect().contains(pos) {
            return self.content.hit_test(pos).or(Some(self.id));
        }
        Some(self.id)
    }
}
