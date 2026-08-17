//! Vertical scrolling — one implementation, two shapes.
//!
//! [`Scroller`] is the state machine: wheel, thumb drag, track paging, keyboard
//! paging, and the bar's paint. A widget **embeds** it over a viewport rect it
//! names, exactly the way a widget embeds [`ArmFire`](crate::ArmFire) over a hit
//! oracle it names — which is what lets a custom widget that scrolls only a
//! *sub-rect* of itself (a list under a pinned header) reuse the behavior
//! instead of re-writing it.
//!
//! [`ScrollArea`] is the stock container built on that machine: it wraps a
//! single child, crops it to the viewport, and scrolls it.
//!
//! Scroll offset is applied during `arrange` (content is positioned shifted by
//! `-offset`), so after a scroll the host's next `layout` reflects it — the usual
//! dispatch → layout → draw frame order.

use crate::draw::DrawList;
use crate::event::{Event, Key, PointerButton, ScrollDelta};
use crate::geom::{Rect, Size, Vec2};
use crate::interact::{WidgetId, WidgetState, next_id};
use crate::theme::Theme;
use crate::widget::{DrawCtx, EventCtx, LayoutCtx, Widget};

/// Pixels scrolled per wheel notch.
const WHEEL_STEP: f32 = 48.0;
/// Fraction of the viewport a page step covers (PgUp/PgDn, a track click).
const PAGE: f32 = 0.9;

/// Who PgUp/PgDn/Home/End page — the [`Scroller`] analogue of
/// [`CommitPolicy`](crate::CommitPolicy).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageKeys {
    /// Page only while the owning widget holds keyboard focus. The stock widget
    /// rule, and what [`ScrollArea`] uses.
    #[default]
    WhenFocused,
    /// Page while the pointer is over the viewport, focus or not — the host
    /// *accelerator* rule, for chrome that cannot take keyboard focus (a panel
    /// whose rows are not focusable still has to page under the cursor).
    WhenHovered,
}

/// The scroll state machine a widget embeds over a viewport rect.
///
/// The owner names the geometry once per layout via [`layout`](Self::layout)
/// (viewport rect + content height); everything else — the offset, an in-flight
/// thumb drag, the bar's hover — lives here. Feed it events with
/// [`event`](Self::event) and paint it with [`draw`](Self::draw).
///
/// The scrollbar occupies the right-hand [`Metrics::scrollbar`](crate::Metrics)
/// pixels *of the viewport*: the owner reserves that column when it lays its
/// content out (as [`ScrollArea`] does), or lets the bar overlay it.
#[derive(Debug, Default)]
pub struct Scroller {
    offset: f32,
    viewport: Rect,
    content_h: f32,
    bar_w: f32,
    min_thumb: f32,
    dragging: bool,
    /// Where inside the thumb the drag grabbed it.
    drag_grab: f32,
    /// The pointer is over the thumb (so the theme can light it).
    hover: bool,
}

impl Scroller {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records this layout's geometry: the `viewport` rect scrolled over (the
    /// scrollbar column is its right edge) and the full `content_h` of what
    /// scrolls through it. Call from the owner's `arrange`; the offset is
    /// re-clamped against the new extent.
    pub fn layout(&mut self, ctx: &LayoutCtx, viewport: Rect, content_h: f32) {
        let m = ctx.theme.metrics();
        self.bar_w = m.scrollbar;
        self.min_thumb = m.scrollbar_min_thumb;
        self.viewport = viewport;
        self.content_h = content_h;
        self.offset = self.offset.clamp(0.0, self.max_offset());
    }

    /// The viewport rect from the last [`layout`](Self::layout).
    pub fn viewport(&self) -> Rect {
        self.viewport
    }

    /// The content height from the last [`layout`](Self::layout).
    pub fn content_height(&self) -> f32 {
        self.content_h
    }

    /// How far the content is scrolled, in pixels from its top.
    pub fn offset(&self) -> f32 {
        self.offset
    }

    /// Scrolls to `offset`, clamped to `0..=max_offset`.
    pub fn set_offset(&mut self, offset: f32) {
        self.offset = offset.clamp(0.0, self.max_offset());
    }

    /// Scrolls by `dy` pixels (positive = toward the content's end), clamped.
    pub fn scroll_by(&mut self, dy: f32) {
        self.set_offset(self.offset + dy);
    }

    /// The largest legal offset (0 when the content fits).
    pub fn max_offset(&self) -> f32 {
        (self.content_h - self.viewport.h).max(0.0)
    }

    /// True when the content overflows the viewport, so a bar is shown.
    pub fn has_bar(&self) -> bool {
        self.content_h > self.viewport.h + 0.5
    }

    /// True while a thumb drag is in flight.
    pub fn dragging(&self) -> bool {
        self.dragging
    }

    /// One page step, in pixels.
    fn page(&self) -> f32 {
        self.viewport.h * PAGE
    }

    /// The scrollbar track: the viewport's right-hand column.
    pub fn track_rect(&self) -> Rect {
        Rect::new(
            self.viewport.right() - self.bar_w,
            self.viewport.y,
            self.bar_w,
            self.viewport.h,
        )
    }

    fn thumb_h(&self) -> f32 {
        let vh = self.viewport.h;
        if self.content_h <= vh {
            return vh;
        }
        // `min_thumb` can exceed a short viewport — clamp the floor first.
        (vh * vh / self.content_h).clamp(self.min_thumb.min(vh), vh)
    }

    /// The thumb, positioned for the current offset (the full track when the
    /// content fits).
    pub fn thumb_rect(&self) -> Rect {
        let track = self.track_rect();
        let th = self.thumb_h();
        let max = self.max_offset();
        let ty = if max > 0.0 {
            track.y + (self.offset / max) * (track.h - th)
        } else {
            track.y
        };
        Rect::new(track.x, ty, track.w, th)
    }

    /// The bar's [`WidgetState`] — hovered over the thumb, pressed while
    /// dragging it.
    pub fn state(&self) -> WidgetState {
        WidgetState {
            hovered: self.hover,
            pressed: self.dragging,
            ..Default::default()
        }
    }

    /// Sets the offset so the thumb's top sits at screen `y` minus the grab.
    fn drag_to(&mut self, y: f32) {
        let track = self.track_rect();
        let travel = (track.h - self.thumb_h()).max(1.0);
        let ty = (y - self.drag_grab - track.y).clamp(0.0, travel);
        self.offset = (ty / travel) * self.max_offset();
    }

    /// Paints the bar through [`Theme::scrollbar`](crate::Theme::scrollbar) —
    /// nothing when the content fits, and base-pass only (the overlay pass is
    /// for popups, so a dropdown opened over the bar isn't overpainted).
    pub fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if ctx.is_base() {
            self.draw_bar(dl, ctx.theme);
        }
    }

    /// [`draw`](Self::draw)'s paint without the pass gate — for an owner whose
    /// scrolling surface *is* the overlay pass (a [`Select`](crate::Select)'s
    /// option list, which draws its bar as part of the popup). Still nothing
    /// when the content fits.
    pub fn draw_bar(&self, dl: &mut DrawList, theme: &dyn Theme) {
        if self.has_bar() {
            theme.scrollbar(dl, self.track_rect(), self.thumb_rect(), self.state());
        }
    }

    /// Feeds one event through the machine; returns whether it was consumed.
    /// `id` is the owning widget's id — presses on the bar are only taken when
    /// it is the dispatch target, and the thumb drag captures the pointer under
    /// it. Paging follows [`PageKeys::WhenFocused`]; use
    /// [`event_with`](Self::event_with) for the accelerator rule.
    ///
    /// The owner keeps first refusal: call this *after* offering the event to
    /// whatever lives inside the viewport, so a drag that starts on content
    /// near the gutter isn't stolen.
    pub fn event(&mut self, ev: &Event, ctx: &mut EventCtx, id: WidgetId) -> bool {
        self.event_with(ev, ctx, id, PageKeys::WhenFocused)
    }

    /// [`event`](Self::event) with an explicit [`PageKeys`] rule.
    pub fn event_with(
        &mut self,
        ev: &Event,
        ctx: &mut EventCtx,
        id: WidgetId,
        keys: PageKeys,
    ) -> bool {
        // Track thumb hover on the way past: the bar is chrome the owner does
        // not hit-test, so nothing else can resolve its hover state.
        match ev {
            Event::PointerMoved { .. } | Event::PointerButton { .. } => {
                self.hover = self.has_bar() && self.thumb_rect().contains(ctx.pointer);
            }
            Event::PointerLeft => self.hover = false,
            _ => {}
        }

        match ev {
            // Wheel anywhere over the viewport.
            Event::Scroll { delta, .. }
                if self.viewport.contains(ctx.pointer) && self.has_bar() =>
            {
                let dy = match delta {
                    ScrollDelta::Lines(v) => v.y * WHEEL_STEP,
                    ScrollDelta::Pixels(v) => v.y,
                };
                self.scroll_by(dy);
                ctx.consume_pointer();
                true
            }
            // Keyboard paging.
            Event::Key {
                key, pressed: true, ..
            } if self.pages(ctx, id, keys) => {
                let page = self.page();
                match key {
                    Key::PageDown => self.scroll_by(page),
                    Key::PageUp => self.scroll_by(-page),
                    Key::Home => self.offset = 0.0,
                    Key::End => self.offset = self.max_offset(),
                    _ => return false,
                }
                ctx.consume_keyboard();
                true
            }
            // Scrollbar press: drag the thumb, or page on the track.
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                ..
            } if ctx.is_target(id) && self.has_bar() && self.track_rect().contains(ctx.pointer) => {
                ctx.consume_pointer();
                let thumb = self.thumb_rect();
                if thumb.contains(ctx.pointer) {
                    self.dragging = true;
                    self.drag_grab = ctx.pointer.y - thumb.y;
                    ctx.capture(id);
                } else if ctx.pointer.y < thumb.y {
                    self.scroll_by(-self.page());
                } else {
                    self.scroll_by(self.page());
                }
                true
            }
            Event::PointerMoved { .. } if self.dragging && ctx.is_target(id) => {
                self.drag_to(ctx.pointer.y);
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

    /// Whether a paging key reaches this scroller under `keys`.
    fn pages(&self, ctx: &EventCtx, id: WidgetId, keys: PageKeys) -> bool {
        match keys {
            PageKeys::WhenFocused => ctx.is_target(id),
            PageKeys::WhenHovered => self.viewport.contains(ctx.pointer),
        }
    }
}

/// A scrollable container around a single (typically tall) child.
#[must_use]
pub struct ScrollArea {
    id: WidgetId,
    content: Box<dyn Widget>,
    scroller: Scroller,
    content_h: f32,
    scrollbar_w: f32,
    page_keys: PageKeys,
    rect: Rect,
}

impl ScrollArea {
    pub fn new(content: impl Widget + 'static) -> Self {
        Self {
            id: next_id(),
            content: Box::new(content),
            scroller: Scroller::new(),
            content_h: 0.0,
            scrollbar_w: 8.0,
            page_keys: PageKeys::WhenFocused,
            rect: Rect::ZERO,
        }
    }

    /// Who PgUp/PgDn/Home/End page (default [`PageKeys::WhenFocused`]).
    ///
    /// [`PageKeys::WhenHovered`] **also stops the area taking keyboard focus on
    /// a press**: focus was only ever grabbed so the paging keys would reach it,
    /// and an area that pages under the cursor needs nothing. The difference is
    /// visible the moment the content holds a text field — a press on the form's
    /// inert chrome would otherwise blur the field, and a blurred field commits.
    pub fn page_keys(mut self, keys: PageKeys) -> Self {
        self.page_keys = keys;
        self
    }

    pub fn id(&self) -> WidgetId {
        self.id
    }

    pub fn offset(&self) -> f32 {
        self.scroller.offset()
    }

    pub fn set_offset(&mut self, offset: f32) {
        self.scroller.set_offset(offset);
    }

    fn content_w(&self) -> f32 {
        (self.rect.w - self.scrollbar_w).max(0.0)
    }
}

impl Widget for ScrollArea {
    fn measure(&mut self, avail: Size, ctx: &mut LayoutCtx) -> Size {
        self.scrollbar_w = ctx.theme.metrics().scrollbar;
        let cw = (avail.w - self.scrollbar_w).max(0.0);
        // Measure content at its natural height.
        let cs = self.content.measure(Size::new(cw, 1.0e6), ctx);
        self.content_h = cs.h;
        // Natural width is the content's plus the bar gutter — claiming
        // `avail.w` would widen an auto-sized dialog to the whole screen.
        Size::new((cs.w + self.scrollbar_w).min(avail.w), self.content_h)
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        self.scroller.layout(ctx, rect, self.content_h);
        let cw = self.content_w();
        self.content.arrange(
            Rect::new(rect.x, rect.y - self.scroller.offset(), cw, self.content_h),
            ctx,
        );
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        // Content draws in **both** passes so a `Select` inside can reach the
        // overlay — but the overlay pass carries only what has to escape this
        // container: a popup is placed against the *window*, so cropping it to
        // the viewport (or to the scrolled content) would clip the option list
        // the moment it hangs past the area's edge, and hide it outright when it
        // flips up above it. The crop and the bar are the base pass's alone.
        if !ctx.is_base() {
            self.content.draw(dl, ctx);
            return;
        }
        // Crop content to the viewport.
        let viewport = Rect::new(self.rect.x, self.rect.y, self.content_w(), self.rect.h);
        dl.push_clip(viewport);
        self.content.draw(dl, ctx);
        dl.pop_clip();
        self.scroller.draw(dl, ctx);
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        // Content (inner widgets / nested scrollers) gets first refusal.
        if self.content.event(ev, ctx) {
            return true;
        }
        // A press anywhere in the area focuses it, so keyboard paging works —
        // and only then (see `page_keys`).
        if let Event::PointerButton {
            button: PointerButton::Primary,
            pressed: true,
            ..
        } = ev
            && self.page_keys == PageKeys::WhenFocused
            && self.rect.contains(ctx.pointer)
        {
            ctx.request_focus(self.id);
        }
        if self.scroller.event_with(ev, ctx, self.id, self.page_keys) {
            return true;
        }
        // A press on the empty viewport is swallowed either way: it belongs to
        // this area, whether or not it moved focus.
        match ev {
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                ..
            } if self.rect.contains(ctx.pointer) => {
                ctx.consume_pointer();
                true
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
        // The scrollbar column targets the scroll area itself.
        if self.scroller.has_bar() && pos.x >= self.rect.right() - self.scrollbar_w {
            return Some(self.id);
        }
        // Otherwise hit-test the (clipped) content, falling back to self so the
        // empty viewport is focusable.
        self.content.hit_test(pos).or(Some(self.id))
    }
}
