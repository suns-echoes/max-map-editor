//! [`Split`] — two children sharing one axis, resized by a draggable divider.
//!
//! The space-partitioning counterpart of [`crate::layout::Linear`]: instead of
//! flowing N children by length rules, a `Split` gives the **first** child an
//! explicit extent (logical px), hands the rest to the second, and lets the
//! user move the boundary. The extent — not a ratio — is the retained state:
//! a sidebar keeps its 240 px when the window grows, exactly the way every
//! panel/document layout wants it (a ratio-based split like
//! [`DockArea`](crate::DockArea)'s interior widens the sidebar with the
//! window, which is right for a workspace of peer panels and wrong for
//! chrome-beside-content).
//!
//! The divider is the widget's own interactive chrome, like a
//! [`Scroller`](crate::Scroller)'s bar: it hit-tests to the `Split` itself,
//! drags under pointer capture, and paints through one theme hook
//! ([`Theme::divider`](crate::theme::Theme::divider)) so one theme is one
//! divider everywhere. The children own everything else — a press inside a
//! child is the child's (or nobody's: two non-interactive children make the
//! split transparent to hit-testing outside the divider).

use crate::draw::DrawList;
use crate::event::{Event, PointerButton};
use crate::geom::{Rect, Size, Vec2};
use crate::interact::{CursorIcon, WidgetId, WidgetState, next_id};
use crate::widget::{Axis, DrawCtx, EventCtx, LayoutCtx, Widget};

/// Default divider thickness (logical px) — wide enough to grab, narrow
/// enough to read as a seam. Override per split with
/// [`thickness`](Split::thickness).
const THICKNESS: f32 = 4.0;

/// A two-child container with a draggable divider between them.
///
/// [`row`](Self::row) lays the children side by side (the divider is a
/// vertical bar, dragged horizontally); [`column`](Self::column) stacks them
/// (a horizontal bar, dragged vertically) — the same vocabulary as
/// [`Linear::row`](crate::layout::Linear::row) /
/// [`column`](crate::layout::Linear::column).
///
/// **The first child's extent is the state.** A drag moves it within the
/// [`min_first`](Self::min_first) / [`max_first`](Self::max_first) clamps
/// (and never past the container — the second child cannot go negative); the
/// second child always takes what remains. When the container itself is too
/// small, `min_first` wins and the second child collapses toward zero — the
/// [`Limits`](crate::widget::Limits) law (min beats max), applied to a
/// partition.
#[must_use]
pub struct Split {
    id: WidgetId,
    axis: Axis,
    a: Box<dyn Widget>,
    b: Box<dyn Widget>,
    /// The first child's requested main extent (logical px). Kept as asked —
    /// arrange clamps *effectively* against the rect it is given, so a window
    /// squeezed and re-grown restores the extent the user chose.
    first: f32,
    min: f32,
    max: f32,
    thickness: f32,
    /// The divider's arranged rect (valid after `arrange`) — the grab target.
    divider: Rect,
    dragging: bool,
    /// Where inside the divider the drag grabbed it (main-axis offset), so the
    /// bar doesn't jump to center under the pointer on the first move.
    drag_grab: f32,
    /// The pointer is over the divider (so the theme can light it).
    hover: bool,
    rect: Rect,
}

impl Split {
    /// Side-by-side children: `a` left, `b` right, a vertical divider between.
    pub fn row(a: impl Widget + 'static, b: impl Widget + 'static) -> Self {
        Self::new(Axis::Horizontal, a, b)
    }

    /// Stacked children: `a` above, `b` below, a horizontal divider between.
    pub fn column(a: impl Widget + 'static, b: impl Widget + 'static) -> Self {
        Self::new(Axis::Vertical, a, b)
    }

    fn new(axis: Axis, a: impl Widget + 'static, b: impl Widget + 'static) -> Self {
        Self {
            id: next_id(),
            axis,
            a: Box::new(a),
            b: Box::new(b),
            first: 0.0,
            min: 0.0,
            max: f32::INFINITY,
            thickness: THICKNESS,
            divider: Rect::ZERO,
            dragging: false,
            drag_grab: 0.0,
            hover: false,
            rect: Rect::ZERO,
        }
    }

    /// The first child's initial main extent (logical px).
    pub fn first(mut self, px: f32) -> Self {
        self.first = px;
        self
    }

    /// The smallest extent a drag may leave the first child (default 0).
    pub fn min_first(mut self, px: f32) -> Self {
        self.min = px;
        self.first = self.first.max(self.min);
        self
    }

    /// The largest extent a drag may give the first child (default unbounded).
    pub fn max_first(mut self, px: f32) -> Self {
        self.max = px;
        self.first = self.first.min(self.max).max(self.min);
        self
    }

    /// The divider's thickness (logical px, default 4).
    pub fn thickness(mut self, px: f32) -> Self {
        self.thickness = px;
        self
    }

    pub fn id(&self) -> WidgetId {
        self.id
    }

    /// The first child's current requested extent (logical px). What a drag
    /// moved it to — persist this to restore a layout across sessions.
    pub fn first_extent(&self) -> f32 {
        self.first
    }

    /// Sets the first child's extent, clamped into the min/max bounds. The
    /// container clamp (the second child cannot go negative) applies at the
    /// next arrange, so calling this before any layout keeps the full value.
    pub fn set_first(&mut self, px: f32) {
        self.first = px.min(self.max).max(self.min);
    }

    /// True while a divider drag is in flight.
    pub fn dragging(&self) -> bool {
        self.dragging
    }

    /// The divider's arranged rect (valid after `arrange`).
    pub fn divider_rect(&self) -> Rect {
        self.divider
    }

    /// The effective first extent for a container `main` px long: the
    /// requested extent under the min/max clamps and the container's own
    /// ceiling, min winning over everything (the second child collapses
    /// before the first goes below its floor).
    fn effective_first(&self, main: f32) -> f32 {
        self.first
            .min(self.max)
            .min((main - self.thickness).max(0.0))
            .max(self.min)
    }

    /// Clamps a dragged extent against the bounds and the *arranged* rect —
    /// drags happen between layouts, so the last rect is the honest ceiling.
    fn clamp_drag(&self, px: f32) -> f32 {
        px.min(self.max)
            .min((self.axis.main(self.rect.size()) - self.thickness).max(0.0))
            .max(self.min)
    }

    /// The resize cursor for this split's divider orientation.
    fn resize_cursor(&self) -> CursorIcon {
        match self.axis {
            Axis::Horizontal => CursorIcon::ResizeEW,
            Axis::Vertical => CursorIcon::ResizeNS,
        }
    }

    /// A point's coordinate along the split's main axis.
    fn along(&self, p: Vec2) -> f32 {
        match self.axis {
            Axis::Horizontal => p.x,
            Axis::Vertical => p.y,
        }
    }
}

impl Widget for Split {
    fn measure(&mut self, avail: Size, ctx: &mut LayoutCtx) -> Size {
        // A split fills what it is given (it partitions space, it does not
        // report content) — but the children still get their measure pass, at
        // the extents the partition would give them, so measure-dependent
        // content (wrapped text) sees honest widths.
        let main = self.axis.main(avail);
        let cross = self.axis.cross(avail);
        let eff = self.effective_first(main);
        self.a.measure(self.axis.size(eff, cross), ctx);
        self.b.measure(
            self.axis
                .size((main - eff - self.thickness).max(0.0), cross),
            ctx,
        );
        avail
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        let main = self.axis.main(rect.size());
        let cross = self.axis.cross(rect.size());
        let eff = self.effective_first(main);
        let origin = Vec2::new(rect.x, rect.y);
        self.a
            .arrange(self.axis.rect(origin, 0.0, 0.0, eff, cross), ctx);
        self.divider = self.axis.rect(
            origin,
            eff,
            0.0,
            self.thickness.min(main - eff).max(0.0),
            cross,
        );
        self.b.arrange(
            self.axis.rect(
                origin,
                eff + self.thickness,
                0.0,
                (main - eff - self.thickness).max(0.0),
                cross,
            ),
            ctx,
        );
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        // Children draw in both passes (a popup inside must reach the
        // overlay); the divider is base-pass chrome.
        self.a.draw(dl, ctx);
        self.b.draw(dl, ctx);
        if ctx.is_base() {
            ctx.theme.divider(
                dl,
                self.divider,
                self.axis == Axis::Horizontal,
                WidgetState {
                    hovered: self.hover,
                    pressed: self.dragging,
                    ..Default::default()
                },
            );
        }
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        // Track divider hover on the way past: the divider is chrome nothing
        // else can resolve hover for (the Scroller rule).
        match ev {
            Event::PointerMoved { .. } | Event::PointerButton { .. } => {
                self.hover = self.divider.contains(ctx.pointer);
            }
            Event::PointerLeft => self.hover = false,
            _ => {}
        }

        // Children get first refusal — a drag that starts on content near the
        // divider isn't stolen.
        if self.a.event(ev, ctx) || self.b.event(ev, ctx) {
            return true;
        }

        match ev {
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                ..
            } if ctx.is_target(self.id) && self.divider.contains(ctx.pointer) => {
                self.dragging = true;
                self.drag_grab =
                    self.along(ctx.pointer) - self.along(Vec2::new(self.divider.x, self.divider.y));
                ctx.capture(self.id);
                ctx.consume_pointer();
                true
            }
            Event::PointerMoved { .. } if self.dragging && ctx.is_target(self.id) => {
                let pointer_main =
                    self.along(ctx.pointer) - self.along(Vec2::new(self.rect.x, self.rect.y));
                self.first = self.clamp_drag(pointer_main - self.drag_grab);
                ctx.consume_pointer();
                true
            }
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: false,
                ..
            } if self.dragging => {
                self.dragging = false;
                ctx.consume_pointer();
                true
            }
            // Window focus loss: the drag's release will never arrive.
            Event::Focus(false) => {
                self.dragging = false;
                false
            }
            _ => false,
        }
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn id(&self) -> WidgetId {
        self.id
    }

    fn child_count(&self) -> usize {
        2
    }

    fn child(&self, i: usize) -> Option<&dyn Widget> {
        match i {
            0 => Some(self.a.as_ref()),
            1 => Some(self.b.as_ref()),
            _ => None,
        }
    }

    fn child_mut(&mut self, i: usize) -> Option<&mut dyn Widget> {
        match i {
            0 => Some(self.a.as_mut()),
            1 => Some(self.b.as_mut()),
            _ => None,
        }
    }

    fn cursor(&self, pos: Vec2) -> CursorIcon {
        // Mid-drag the pointer legitimately outruns the bar — the capture
        // keeps the cursor honest (the resize arrows persist).
        if self.dragging || self.divider.contains(pos) {
            self.resize_cursor()
        } else {
            CursorIcon::Default
        }
    }

    fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
        if !self.rect.contains(pos) {
            return None;
        }
        if self.divider.contains(pos) {
            return Some(self.id);
        }
        // Children don't overlap; outside the divider the split itself claims
        // nothing (two hit-transparent children leave the split transparent).
        self.a.hit_test(pos).or_else(|| self.b.hit_test(pos))
    }
}
