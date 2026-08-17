//! Layout containers ([`Linear`], [`Wrap`], [`Stack`]) and simple leaves
//! ([`Fill`], [`Spacer`]). All sizes are logical UI pixels.

use crate::color::Rgba;
use crate::draw::DrawList;
use crate::event::{Event, PointerButton};
use crate::geom::{Insets, Rect, Size, Vec2};
use crate::interact::WidgetId;
use crate::widget::{
    Axis, CrossAlign, DrawCtx, EventCtx, LayoutCtx, Length, Limits, MainAlign, Widget,
};

/// A solid-color rectangle with a preferred size — a useful leaf for layouts and
/// backgrounds (and the test oracle for the layout engine).
#[must_use]
pub struct Fill {
    color: Rgba,
    pref: Size,
    rect: Rect,
}

impl Fill {
    pub fn new(color: Rgba, pref: Size) -> Self {
        Self {
            color,
            pref,
            rect: Rect::ZERO,
        }
    }
}

impl Widget for Fill {
    fn measure(&mut self, _avail: Size, _ctx: &mut LayoutCtx) -> Size {
        self.pref
    }

    fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if !ctx.is_base() {
            return;
        }
        dl.fill_rect(self.rect, self.color);
    }

    fn rect(&self) -> Rect {
        self.rect
    }
}

/// An empty, zero-preferred-size leaf. Combined with [`Length::Flex`] it pushes
/// siblings apart.
#[derive(Default)]
#[must_use]
pub struct Spacer {
    rect: Rect,
}

impl Spacer {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Widget for Spacer {
    fn measure(&mut self, _avail: Size, _ctx: &mut LayoutCtx) -> Size {
        Size::ZERO
    }

    fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn draw(&self, _dl: &mut DrawList, _ctx: &DrawCtx) {}

    fn rect(&self) -> Rect {
        self.rect
    }
}

/// A child that can be **collapsed** out of the layout without leaving the tree
/// — an optional row in a form, a section that only some selections have.
///
/// Hidden, it measures to [`Size::ZERO`], draws nothing, takes no event, hits
/// nothing, and reports **no children** — so it is skipped by Tab traversal and
/// by [`descendant`](crate::ui::descendant) alike. That last part is the
/// contract to plan around: a host syncs its tree **top-down**, showing an outer
/// `Reveal` before it reaches for anything inside it.
///
/// Give the parent [`Length::Fit`](crate::widget::Length) so the collapse is
/// visible in the layout; a `Length::Fixed` slot would hold the space open.
/// When the child measures to its own content but the list wants a uniform row,
/// name the row with [`height`](Self::height) — that is the size a `Fit` parent
/// cannot supply once the slot has to be able to vanish.
#[must_use]
pub struct Reveal {
    id: WidgetId,
    child: Box<dyn Widget>,
    shown: bool,
    height: Option<f32>,
    rect: Rect,
}

impl Reveal {
    /// Wraps `child`, shown.
    pub fn new(child: impl Widget + 'static) -> Self {
        Self {
            id: crate::interact::next_id(),
            child: Box::new(child),
            shown: true,
            height: None,
            rect: Rect::ZERO,
        }
    }

    /// The id to reach this slot by (`descendant_mut::<Reveal>`) when the host
    /// shows and hides it per frame.
    pub fn id(&self) -> WidgetId {
        self.id
    }

    /// Builder form of [`set_shown`](Self::set_shown).
    pub fn with_shown(mut self, shown: bool) -> Self {
        self.shown = shown;
        self
    }

    /// The height the slot claims while shown, overriding the child's measured
    /// one (the width still measures to the child).
    pub fn height(mut self, h: f32) -> Self {
        self.height = Some(h.max(0.0));
        self
    }

    pub fn set_shown(&mut self, shown: bool) {
        self.shown = shown;
    }

    pub fn is_shown(&self) -> bool {
        self.shown
    }
}

impl Widget for Reveal {
    fn measure(&mut self, avail: Size, ctx: &mut LayoutCtx) -> Size {
        if !self.shown {
            return Size::ZERO;
        }
        let c = self.child.measure(avail, ctx);
        Size::new(c.w, self.height.unwrap_or(c.h))
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        // A collapsed slot keeps its position but no extent, so a stale child
        // rect can never answer a hit test or place a caret.
        self.rect = if self.shown {
            rect
        } else {
            Rect::new(rect.x, rect.y, 0.0, 0.0)
        };
        self.child.arrange(self.rect, ctx);
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if self.shown {
            self.child.draw(dl, ctx);
        }
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        self.shown && self.child.event(ev, ctx)
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn id(&self) -> WidgetId {
        self.id
    }

    /// Zero when collapsed: the subtree is out of Tab order and out of every
    /// tree walk while it is hidden.
    fn child_count(&self) -> usize {
        usize::from(self.shown)
    }

    fn child(&self, i: usize) -> Option<&dyn Widget> {
        (self.shown && i == 0).then_some(self.child.as_ref())
    }

    fn child_mut(&mut self, i: usize) -> Option<&mut dyn Widget> {
        (self.shown && i == 0).then_some(self.child.as_mut())
    }

    /// Delegates: a `Reveal` is a slot, never a pointer target of its own.
    fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
        self.shown.then(|| self.child.hit_test(pos)).flatten()
    }
}

/// Declares hard min/max size bounds on any widget — the way to say "at
/// least this wide, at most this tall" about a child whose own measure
/// can't know it. The bounds work through **both** channels a parent may
/// size by: the wrapper clamps what flows through `measure` (available
/// space going down — so a wrapping label folds at its max width rather
/// than the parent's — and the reported size coming back up), and it
/// republishes the bounds as [`Widget::size_limits`] so a
/// constraint-aware parent ([`Linear`]) folds a flex child's main-axis
/// bounds into its space distribution instead of assigning a share the
/// child can't honor.
///
/// `min` wins over `max`, and an arranged slot outside the bounds leaves
/// the child clamped inside it (top-left anchored): an oversized slot
/// shows the child at its max, an undersized one keeps it at its min —
/// the overflow-is-deliberate policy the layout containers already
/// follow.
#[must_use]
pub struct Constrained {
    child: Box<dyn Widget>,
    limits: Limits,
    rect: Rect,
}

impl Constrained {
    /// Wraps `child` with no bounds; add them with the builder methods.
    pub fn new(child: impl Widget + 'static) -> Self {
        Self {
            child: Box::new(child),
            limits: Limits::NONE,
            rect: Rect::ZERO,
        }
    }

    pub fn min_width(mut self, v: f32) -> Self {
        self.limits.min.w = v.max(0.0);
        self
    }

    pub fn max_width(mut self, v: f32) -> Self {
        self.limits.max.w = v.max(0.0);
        self
    }

    pub fn min_height(mut self, v: f32) -> Self {
        self.limits.min.h = v.max(0.0);
        self
    }

    pub fn max_height(mut self, v: f32) -> Self {
        self.limits.max.h = v.max(0.0);
        self
    }
}

impl Widget for Constrained {
    fn measure(&mut self, avail: Size, ctx: &mut LayoutCtx) -> Size {
        // Clamp the space the child sees (wrap at max, not at avail),
        // then the answer it gives.
        let inner = self.limits.clamp(avail);
        self.limits.clamp(self.child.measure(inner, ctx))
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        // The occupied rect is the clamped one, so hit tests and painting
        // agree with what the bounds allow.
        let size = self.limits.clamp(rect.size());
        self.rect = Rect::from_min_size(rect.min(), size);
        self.child.arrange(self.rect, ctx);
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        self.child.draw(dl, ctx);
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        self.child.event(ev, ctx)
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn size_limits(&self) -> Limits {
        self.limits
    }

    fn child_count(&self) -> usize {
        1
    }

    fn child(&self, i: usize) -> Option<&dyn Widget> {
        (i == 0).then_some(self.child.as_ref())
    }

    fn child_mut(&mut self, i: usize) -> Option<&mut dyn Widget> {
        (i == 0).then_some(self.child.as_mut())
    }

    /// Delegates: a `Constrained` is a slot, never a pointer target of its
    /// own.
    fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
        self.child.hit_test(pos)
    }
}

struct LinearChild {
    widget: Box<dyn Widget>,
    length: Length,
    measured: Size,
}

/// A row or column: lays children along one axis with padding, spacing, and
/// main/cross alignment. Per-child [`Length`] selects fixed / fit / flexible
/// sizing.
#[must_use]
pub struct Linear {
    axis: Axis,
    padding: Insets,
    spacing: f32,
    main_align: MainAlign,
    cross_align: CrossAlign,
    children: Vec<LinearChild>,
    rect: Rect,
}

impl Linear {
    pub fn row() -> Self {
        Self::new(Axis::Horizontal)
    }

    pub fn column() -> Self {
        Self::new(Axis::Vertical)
    }

    fn new(axis: Axis) -> Self {
        Self {
            axis,
            padding: Insets::ZERO,
            spacing: 0.0,
            main_align: MainAlign::Start,
            cross_align: CrossAlign::Start,
            children: Vec::new(),
            rect: Rect::ZERO,
        }
    }

    pub fn padding(mut self, p: Insets) -> Self {
        self.padding = p;
        self
    }

    pub fn spacing(mut self, s: f32) -> Self {
        self.spacing = s;
        self
    }

    pub fn main_align(mut self, a: MainAlign) -> Self {
        self.main_align = a;
        self
    }

    pub fn cross_align(mut self, a: CrossAlign) -> Self {
        self.cross_align = a;
        self
    }

    /// Adds a child with explicit main-axis sizing.
    pub fn child(mut self, widget: impl Widget + 'static, length: Length) -> Self {
        self.children.push(LinearChild {
            widget: Box::new(widget),
            length,
            measured: Size::ZERO,
        });
        self
    }

    /// Appends a child sized to its content ([`Length::Fit`]).
    pub fn push(self, widget: impl Widget + 'static) -> Self {
        self.child(widget, Length::Fit)
    }

    /// The main extent each child occupies, given the inner main space.
    /// Fixed and Fit extents are clamped into the child's
    /// [`Widget::size_limits`]; flex children split the leftover via
    /// [`distribute`], which honors their main-axis bounds.
    fn main_extents(&self, inner_main: f32) -> Vec<f32> {
        let n = self.children.len();
        let gaps = if n > 0 {
            self.spacing * (n - 1) as f32
        } else {
            0.0
        };
        let mut used = gaps;
        let mut flex = Vec::new();
        for c in &self.children {
            let limits = c.widget.size_limits();
            match c.length {
                Length::Fixed(px) => used += clamp_main(self.axis, limits, px),
                Length::Fit => used += self.axis.main(c.measured),
                Length::Flex(w) => flex.push(FlexItem {
                    weight: w,
                    min: self.axis.main(limits.min),
                    max: self.axis.main(limits.max),
                    size: self.axis.main(limits.min),
                }),
            }
        }
        distribute(&mut flex, inner_main - used);
        let mut flex = flex.into_iter();
        self.children
            .iter()
            .map(|c| match c.length {
                Length::Fixed(px) => clamp_main(self.axis, c.widget.size_limits(), px),
                Length::Fit => self.axis.main(c.measured),
                Length::Flex(_) => flex.next().map_or(0.0, |it| it.size),
            })
            .collect()
    }

    fn has_flex(&self) -> bool {
        self.children
            .iter()
            .any(|c| matches!(c.length, Length::Flex(_)))
    }
}

impl Widget for Linear {
    fn measure(&mut self, avail: Size, ctx: &mut LayoutCtx) -> Size {
        let inner_main = (self.axis.main(avail) - main_pad(self.axis, self.padding)).max(0.0);
        let inner_cross = (self.axis.cross(avail) - cross_pad(self.axis, self.padding)).max(0.0);

        // Measure every child to its natural size, clamped into its
        // `size_limits`. Flex children report their content (a `Length::Flex`
        // *spacer* measures to ~0) and grow to fill leftover space only at
        // arrange-time — so a flex child never forces the container to its
        // full available main extent. This lets a flex spacer be used inside
        // a content-sized container (e.g. a dialog) without stretching it to
        // fill the parent.
        let mut used = 0.0;
        for c in &mut self.children {
            let limits = c.widget.size_limits();
            let main_avail = match c.length {
                Length::Fixed(px) => clamp_main(self.axis, limits, px),
                Length::Fit | Length::Flex(_) => inner_main,
            };
            c.measured = limits.clamp(
                c.widget
                    .measure(self.axis.size(main_avail, inner_cross), ctx),
            );
            used += match c.length {
                Length::Fixed(px) => clamp_main(self.axis, limits, px),
                Length::Fit | Length::Flex(_) => self.axis.main(c.measured),
            };
        }
        let n = self.children.len();
        let gaps = if n > 0 {
            self.spacing * (n - 1) as f32
        } else {
            0.0
        };

        let content_cross = self
            .children
            .iter()
            .map(|c| self.axis.cross(c.measured))
            .fold(0.0_f32, f32::max);

        self.axis.size(
            used + gaps + main_pad(self.axis, self.padding),
            content_cross + cross_pad(self.axis, self.padding),
        )
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        let inner = rect.inset(self.padding);
        let origin = inner.min();
        let inner_main = self.axis.main(inner.size());
        let inner_cross = self.axis.cross(inner.size());

        let extents = self.main_extents(inner_main);
        let total_main: f32 = extents.iter().sum();
        let n = self.children.len();
        let gaps = if n > 0 {
            self.spacing * (n - 1) as f32
        } else {
            0.0
        };
        let free = (inner_main - total_main - gaps).max(0.0);

        // Distribute leftover (only meaningful without flex children).
        let (mut cursor, extra_gap) = if self.has_flex() {
            (0.0, 0.0)
        } else {
            match self.main_align {
                MainAlign::Start => (0.0, 0.0),
                MainAlign::Center => (free * 0.5, 0.0),
                MainAlign::End => (free, 0.0),
                MainAlign::SpaceBetween => (0.0, if n > 1 { free / (n - 1) as f32 } else { 0.0 }),
            }
        };

        for (c, &main_ext) in self.children.iter_mut().zip(&extents) {
            // A child whose resolved main extent differs from the one it
            // measured at (a flex share, a clamped Fixed) re-measures at the
            // resolved extent, so cross extents are fresh — a wrapping label
            // in a flex cell folds at the width it actually gets.
            if main_ext != self.axis.main(c.measured) {
                c.measured = c
                    .widget
                    .size_limits()
                    .clamp(c.widget.measure(self.axis.size(main_ext, inner_cross), ctx));
            }
            let child_cross = match self.cross_align {
                CrossAlign::Stretch => inner_cross,
                _ => self.axis.cross(c.measured),
            };
            let cross_pos = match self.cross_align {
                CrossAlign::Start | CrossAlign::Stretch => 0.0,
                CrossAlign::Center => (inner_cross - child_cross) * 0.5,
                CrossAlign::End => inner_cross - child_cross,
            };
            let child_rect = self
                .axis
                .rect(origin, cursor, cross_pos, main_ext, child_cross);
            c.widget.arrange(child_rect, ctx);
            cursor += main_ext + self.spacing + extra_gap;
        }
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        for c in &self.children {
            c.widget.draw(dl, ctx);
        }
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn child_count(&self) -> usize {
        self.children.len()
    }

    fn child(&self, i: usize) -> Option<&dyn Widget> {
        self.children.get(i).map(|c| c.widget.as_ref())
    }

    fn child_mut(&mut self, i: usize) -> Option<&mut dyn Widget> {
        self.children.get_mut(i).map(|c| c.widget.as_mut())
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        let mut handled = false;
        for c in &mut self.children {
            handled |= c.widget.event(ev, ctx);
        }
        handled
    }

    fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
        if !self.rect.contains(pos) {
            return None;
        }
        // Reverse order: later children draw on top, so they hit first.
        self.children
            .iter()
            .rev()
            .find_map(|c| c.widget.hit_test(pos))
    }
}

/// A z-stack: children are layered in order (later draws on top), each filling
/// the stack's content rectangle. Useful for backgrounds + overlays, and — with
/// [`raising`](Stack::raising) — as a floating-window layer.
#[derive(Default)]
#[must_use]
pub struct Stack {
    padding: Insets,
    children: Vec<Box<dyn Widget>>,
    /// Bring the clicked child to the front (window-manager behavior). Off by
    /// default so background/overlay stacks keep their fixed paint order.
    raise: bool,
    rect: Rect,
}

impl Stack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn padding(mut self, p: Insets) -> Self {
        self.padding = p;
        self
    }

    /// Turns this stack into a window layer: a primary press raises the topmost
    /// child under the pointer to the front (drawn/hit last). Leave it off for
    /// fixed-order backgrounds and HUD overlays.
    pub fn raising(mut self) -> Self {
        self.raise = true;
        self
    }

    /// Appends a layer (drawn on top of earlier ones).
    pub fn push(mut self, widget: impl Widget + 'static) -> Self {
        self.children.push(Box::new(widget));
        self
    }
}

impl Widget for Stack {
    fn measure(&mut self, avail: Size, ctx: &mut LayoutCtx) -> Size {
        let inner = Size::new(
            (avail.w - self.padding.horizontal()).max(0.0),
            (avail.h - self.padding.vertical()).max(0.0),
        );
        let mut max = Size::ZERO;
        for c in &mut self.children {
            let m = c.measure(inner, ctx);
            max.w = max.w.max(m.w);
            max.h = max.h.max(m.h);
        }
        Size::new(
            max.w + self.padding.horizontal(),
            max.h + self.padding.vertical(),
        )
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        let inner = rect.inset(self.padding);
        for c in &mut self.children {
            c.arrange(inner, ctx);
        }
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        for c in &self.children {
            c.draw(dl, ctx);
        }
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn child_count(&self) -> usize {
        self.children.len()
    }

    fn child(&self, i: usize) -> Option<&dyn Widget> {
        self.children.get(i).map(|c| c.as_ref())
    }

    fn child_mut(&mut self, i: usize) -> Option<&mut dyn Widget> {
        self.children.get_mut(i).map(|c| c.as_mut())
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        // Window-layer raise: on a primary press, move the topmost child under
        // the pointer to the end of the vector (drawn/hit on top) before
        // dispatching, so the clicked window comes forward.
        if self.raise
            && let Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                ..
            } = ev
            && let Some(i) = self
                .children
                .iter()
                .enumerate()
                .rev()
                .find(|(_, c)| c.hit_test(ctx.pointer).is_some())
                .map(|(i, _)| i)
            && i + 1 != self.children.len()
        {
            let c = self.children.remove(i);
            self.children.push(c);
        }
        let mut handled = false;
        for c in &mut self.children {
            handled |= c.event(ev, ctx);
        }
        handled
    }

    fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
        if !self.rect.contains(pos) {
            return None;
        }
        self.children.iter().rev().find_map(|c| c.hit_test(pos))
    }
}

struct WrapChild {
    widget: Box<dyn Widget>,
    measured: Size,
}

/// A flow layout: lays children along one axis, wrapping to a new **run** (a row
/// for a horizontal wrap, a column for a vertical one) whenever the next child
/// would overflow the available main extent. Each child takes its measured
/// (content) size — there is no per-child [`Length`]. Items within a run are
/// packed from the start; [`line_align`](Wrap::line_align) sets their cross
/// placement *within* the run. Useful for toolbars, tag clouds, button banks,
/// and grid headers that must reflow to the panel width.
#[must_use]
pub struct Wrap {
    axis: Axis,
    padding: Insets,
    /// Gap between items along the main axis.
    spacing: f32,
    /// Gap between runs along the cross axis.
    run_spacing: f32,
    /// Cross placement of items *within* a run.
    line_align: CrossAlign,
    /// Floor on each run's cross extent (see [`Wrap::run_extent`]).
    run_extent: f32,
    children: Vec<WrapChild>,
    rect: Rect,
}

impl Wrap {
    /// A horizontal flow: items fill left-to-right, wrapping downward.
    pub fn row() -> Self {
        Self::new(Axis::Horizontal)
    }

    /// A vertical flow: items fill top-to-bottom, wrapping rightward.
    pub fn column() -> Self {
        Self::new(Axis::Vertical)
    }

    fn new(axis: Axis) -> Self {
        Self {
            axis,
            padding: Insets::ZERO,
            spacing: 0.0,
            run_spacing: 0.0,
            line_align: CrossAlign::Start,
            run_extent: 0.0,
            children: Vec::new(),
            rect: Rect::ZERO,
        }
    }

    pub fn padding(mut self, p: Insets) -> Self {
        self.padding = p;
        self
    }

    /// Gap between items along the main axis.
    pub fn spacing(mut self, s: f32) -> Self {
        self.spacing = s;
        self
    }

    /// Gap between runs along the cross axis (defaults to `0`).
    pub fn run_spacing(mut self, s: f32) -> Self {
        self.run_spacing = s;
        self
    }

    /// Cross placement of items within a run (`Start` by default; `Stretch`
    /// grows each item to the run's height).
    pub fn line_align(mut self, a: CrossAlign) -> Self {
        self.line_align = a;
        self
    }

    /// A floor on every run's cross extent — the **uniform row height** of a
    /// flowed control bar (defaults to `0`: each run is as tall as its tallest
    /// child).
    ///
    /// Without it a run's height is whatever happens to land on it, so a
    /// toolbar of 18px controls that wraps a short caption onto its own run
    /// ends up with one row 1px shorter than the rest — and a panel header
    /// whose band height is an *invariant* (the content below is arranged, or
    /// scissored, against it) wobbles by that much as the dock is resized.
    /// Declaring the row height fixes the band as `rows × extent` at every
    /// width.
    pub fn run_extent(mut self, extent: f32) -> Self {
        self.run_extent = extent;
        self
    }

    /// Appends a child (sized to its content).
    pub fn push(mut self, widget: impl Widget + 'static) -> Self {
        self.children.push(WrapChild {
            widget: Box::new(widget),
            measured: Size::ZERO,
        });
        self
    }

    /// Greedily groups children into runs that each fit `inner_main`. Returns
    /// `(start, end_exclusive, run_main_used, run_cross_max)` per run. A single
    /// child wider than `inner_main` still gets its own run (it overflows rather
    /// than vanishing).
    fn build_runs(&self, inner_main: f32) -> Vec<(usize, usize, f32, f32)> {
        let mut runs = Vec::new();
        let n = self.children.len();
        let mut i = 0;
        while i < n {
            let start = i;
            let mut used = self.axis.main(self.children[i].measured);
            let mut cross = self.axis.cross(self.children[i].measured);
            i += 1;
            while i < n {
                let m = self.axis.main(self.children[i].measured);
                if used + self.spacing + m > inner_main {
                    break;
                }
                used += self.spacing + m;
                cross = cross.max(self.axis.cross(self.children[i].measured));
                i += 1;
            }
            runs.push((start, i, used, cross.max(self.run_extent)));
        }
        runs
    }
}

impl Widget for Wrap {
    fn measure(&mut self, avail: Size, ctx: &mut LayoutCtx) -> Size {
        let inner_main = (self.axis.main(avail) - main_pad(self.axis, self.padding)).max(0.0);
        let inner_cross = (self.axis.cross(avail) - cross_pad(self.axis, self.padding)).max(0.0);
        for c in &mut self.children {
            c.measured = c
                .widget
                .measure(self.axis.size(inner_main, inner_cross), ctx);
        }
        let runs = self.build_runs(inner_main);
        let content_main = runs.iter().map(|r| r.2).fold(0.0_f32, f32::max);
        let runs_cross: f32 = runs.iter().map(|r| r.3).sum();
        let run_gaps = if runs.len() > 1 {
            self.run_spacing * (runs.len() - 1) as f32
        } else {
            0.0
        };
        self.axis.size(
            content_main + main_pad(self.axis, self.padding),
            runs_cross + run_gaps + cross_pad(self.axis, self.padding),
        )
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        let inner = rect.inset(self.padding);
        let origin = inner.min();
        let inner_main = self.axis.main(inner.size());
        let runs = self.build_runs(inner_main);

        let mut cross_cursor = 0.0;
        for (start, end, _used, run_cross) in runs {
            let mut main_cursor = 0.0;
            for idx in start..end {
                let m = self.children[idx].measured;
                let child_main = self.axis.main(m);
                let item_cross = self.axis.cross(m);
                let child_cross = match self.line_align {
                    CrossAlign::Stretch => run_cross,
                    _ => item_cross,
                };
                let cross_off = match self.line_align {
                    CrossAlign::Start | CrossAlign::Stretch => 0.0,
                    CrossAlign::Center => (run_cross - item_cross) * 0.5,
                    CrossAlign::End => run_cross - item_cross,
                };
                let r = self.axis.rect(
                    origin,
                    main_cursor,
                    cross_cursor + cross_off,
                    child_main,
                    child_cross,
                );
                self.children[idx].widget.arrange(r, ctx);
                main_cursor += child_main + self.spacing;
            }
            cross_cursor += run_cross + self.run_spacing;
        }
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        for c in &self.children {
            c.widget.draw(dl, ctx);
        }
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn child_count(&self) -> usize {
        self.children.len()
    }

    fn child(&self, i: usize) -> Option<&dyn Widget> {
        self.children.get(i).map(|c| c.widget.as_ref())
    }

    fn child_mut(&mut self, i: usize) -> Option<&mut dyn Widget> {
        self.children.get_mut(i).map(|c| c.widget.as_mut())
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        let mut handled = false;
        for c in &mut self.children {
            handled |= c.widget.event(ev, ctx);
        }
        handled
    }

    fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
        if !self.rect.contains(pos) {
            return None;
        }
        self.children
            .iter()
            .rev()
            .find_map(|c| c.widget.hit_test(pos))
    }
}

/// A uniform grid: arranges children row-major into a fixed number of
/// `columns`, every cell the same size. The cell width is derived from the
/// arranged width — `(w - (columns-1)·gap) / columns` — so the columns always
/// fill the grid; the cell height matches the cell width (square cells) unless a
/// fixed [`row_height`](Grid::row_height) is set. The grid measures to the height
/// its rows need at the available width, so wrap it in a
/// [`ScrollArea`](crate::scroll::ScrollArea) for a scrollable picker (a palette,
/// a tile or template tray). Children are typically uniform cells —
/// [`ColorButton`](crate::widgets::ColorButton)s or
/// [`ImageButton`](crate::widgets::ImageButton)s — but any widget works; each is
/// arranged into (and measured against) its cell.
#[must_use]
pub struct Grid {
    columns: usize,
    gap: f32,
    row_height: Option<f32>,
    children: Vec<Box<dyn Widget>>,
    rect: Rect,
}

impl Grid {
    /// A grid of `columns` equal columns (clamped to at least 1).
    pub fn new(columns: usize) -> Self {
        Self {
            columns: columns.max(1),
            gap: 0.0,
            row_height: None,
            children: Vec::new(),
            rect: Rect::ZERO,
        }
    }

    /// Gap (logical px) between cells — between columns and between rows alike.
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(0.0);
        self
    }

    /// Fixes the row height (logical px). Without this, cells are square (the row
    /// height tracks the width-derived cell width).
    pub fn row_height(mut self, h: f32) -> Self {
        self.row_height = Some(h.max(0.0));
        self
    }

    /// Appends a cell.
    pub fn push(mut self, widget: impl Widget + 'static) -> Self {
        self.children.push(Box::new(widget));
        self
    }

    /// The number of columns (clamped ≥ 1).
    pub fn columns(&self) -> usize {
        self.columns
    }

    /// The cell width for an inner width across `columns` gapped columns.
    fn cell_w(&self, inner_w: f32) -> f32 {
        let gaps = self.gap * self.columns.saturating_sub(1) as f32;
        ((inner_w - gaps) / self.columns as f32).max(0.0)
    }

    /// The number of rows the children occupy.
    fn row_count(&self) -> usize {
        self.children.len().div_ceil(self.columns)
    }

    /// The cell size at a given inner width (square unless `row_height` is set).
    fn cell_size(&self, inner_w: f32) -> Size {
        let w = self.cell_w(inner_w);
        Size::new(w, self.row_height.unwrap_or(w))
    }

    /// Total content height for the current rows at `cell_h`, with `gap` between.
    fn content_h(&self, cell_h: f32) -> f32 {
        let n = self.row_count();
        if n == 0 {
            0.0
        } else {
            n as f32 * cell_h + (n - 1) as f32 * self.gap
        }
    }
}

impl Widget for Grid {
    fn measure(&mut self, avail: Size, ctx: &mut LayoutCtx) -> Size {
        let cell = self.cell_size(avail.w);
        for c in &mut self.children {
            c.measure(cell, ctx);
        }
        // The grid fills the available width; its height is what the rows need.
        Size::new(avail.w, self.content_h(cell.h))
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        let cell = self.cell_size(rect.w);
        let origin = rect.min();
        for (i, c) in self.children.iter_mut().enumerate() {
            let col = i % self.columns;
            let row = i / self.columns;
            let x = origin.x + col as f32 * (cell.w + self.gap);
            let y = origin.y + row as f32 * (cell.h + self.gap);
            c.arrange(Rect::new(x, y, cell.w, cell.h), ctx);
        }
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        for c in &self.children {
            c.draw(dl, ctx);
        }
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn child_count(&self) -> usize {
        self.children.len()
    }

    fn child(&self, i: usize) -> Option<&dyn Widget> {
        self.children.get(i).map(|c| c.as_ref())
    }

    fn child_mut(&mut self, i: usize) -> Option<&mut dyn Widget> {
        self.children.get_mut(i).map(|c| c.as_mut())
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        let mut handled = false;
        for c in &mut self.children {
            handled |= c.event(ev, ctx);
        }
        handled
    }

    fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
        if !self.rect.contains(pos) {
            return None;
        }
        self.children.iter().rev().find_map(|c| c.hit_test(pos))
    }
}

/// Clamps a main-axis extent into `limits`, min winning over max.
fn clamp_main(axis: Axis, limits: Limits, v: f32) -> f32 {
    v.min(axis.main(limits.max)).max(axis.main(limits.min))
}

/// One flex child during main-axis distribution. `size` is the output and
/// must start at `min` — [`distribute`] leaves it there when no share can
/// be assigned (all weights zero).
struct FlexItem {
    weight: f32,
    min: f32,
    max: f32,
    size: f32,
}

/// Splits `available` pixels between flex items proportionally to weight,
/// honoring each item's min/max: an item whose proportional share falls
/// outside its bounds freezes at the bound and the remainder
/// redistributes among the rest (the classic flex resolve loop; at most
/// `items.len()` rounds). Non-positive `available` leaves everyone at
/// their minimum.
fn distribute(items: &mut [FlexItem], available: f32) {
    let mut frozen = vec![false; items.len()];
    let mut remaining = available;
    loop {
        let total: f32 = items
            .iter()
            .zip(&frozen)
            .filter(|&(_, f)| !f)
            .map(|(it, _)| it.weight.max(0.0))
            .sum();
        if total <= 0.0 {
            return; // unfrozen items keep their minimum
        }
        let pool = remaining.max(0.0);
        // Tentative proportional shares; collect bound violations.
        let mut min_violators = Vec::new();
        let mut max_violators = Vec::new();
        let mut violation = 0.0f32;
        for (i, it) in items.iter().enumerate() {
            if frozen[i] {
                continue;
            }
            let share = pool * it.weight.max(0.0) / total;
            if share < it.min {
                min_violators.push(i);
                violation += it.min - share;
            } else if share > it.max {
                max_violators.push(i);
                violation -= share - it.max;
            }
        }
        if min_violators.is_empty() && max_violators.is_empty() {
            for (i, it) in items.iter_mut().enumerate() {
                if !frozen[i] {
                    it.size = pool * it.weight.max(0.0) / total;
                }
            }
            return;
        }
        // Freeze only the violator kind the round is short on — a
        // min-violator may become satisfiable once a max-violator frees
        // its excess (the flexbox resolve rule). Exact zero (violations
        // cancel) freezes both sides.
        if violation >= 0.0 {
            for &i in &min_violators {
                items[i].size = items[i].min;
                frozen[i] = true;
                remaining -= items[i].size;
            }
        }
        if violation <= 0.0 {
            for &i in &max_violators {
                items[i].size = items[i].max;
                frozen[i] = true;
                remaining -= items[i].size;
            }
        }
    }
}

fn main_pad(axis: Axis, p: Insets) -> f32 {
    match axis {
        Axis::Horizontal => p.horizontal(),
        Axis::Vertical => p.vertical(),
    }
}

fn cross_pad(axis: Axis, p: Insets) -> f32 {
    match axis {
        Axis::Horizontal => p.vertical(),
        Axis::Vertical => p.horizontal(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{FontId, Fonts};
    use crate::theme::Gunmetal;

    /// Measures then arranges `w` at `size` and returns it for inspection.
    fn lay(mut w: impl Widget + 'static, size: Size) -> Box<dyn Widget> {
        let fonts = Fonts::new();
        let theme = Gunmetal::new(FontId(0));
        let mut ctx = LayoutCtx {
            fonts: &fonts,
            theme: &theme,
            scale: 1.0,
            viewport: Rect::ZERO,
        };
        w.measure(size, &mut ctx);
        w.arrange(Rect::from_min_size(Vec2::ZERO, size), &mut ctx);
        Box::new(w)
    }

    fn child_rects(w: &dyn Widget) -> Vec<Rect> {
        (0..w.child_count())
            .filter_map(|i| w.child(i))
            .map(|c| c.rect())
            .collect()
    }

    fn fill(w: f32, h: f32) -> Fill {
        Fill::new(Rgba::WHITE, Size::new(w, h))
    }

    #[test]
    fn row_distributes_fixed_and_flex() {
        let row = Linear::row()
            .child(fill(20.0, 40.0), Length::Fixed(20.0))
            .child(Spacer::new(), Length::Flex(1.0))
            .child(fill(30.0, 40.0), Length::Fixed(30.0));
        let row = lay(row, Size::new(100.0, 40.0));
        let r = child_rects(row.as_ref());
        assert_eq!(r[0], Rect::new(0.0, 0.0, 20.0, 40.0));
        assert_eq!(r[1], Rect::new(20.0, 0.0, 50.0, 0.0)); // flex spacer fills 100-50
        assert_eq!(r[2], Rect::new(70.0, 0.0, 30.0, 40.0));
    }

    #[test]
    fn column_stacks_vertically() {
        let col = Linear::column()
            .child(fill(40.0, 10.0), Length::Fixed(10.0))
            .child(fill(40.0, 30.0), Length::Fixed(30.0));
        let col = lay(col, Size::new(40.0, 40.0));
        let r = child_rects(col.as_ref());
        assert_eq!(r[0], Rect::new(0.0, 0.0, 40.0, 10.0));
        assert_eq!(r[1], Rect::new(0.0, 10.0, 40.0, 30.0));
    }

    #[test]
    fn padding_and_spacing_offset_children() {
        let row = Linear::row()
            .padding(Insets::all(10.0))
            .spacing(5.0)
            .push(fill(20.0, 10.0))
            .push(fill(20.0, 10.0));
        let row = lay(row, Size::new(100.0, 40.0));
        let r = child_rects(row.as_ref());
        assert_eq!(r[0].x, 10.0);
        assert_eq!(r[1].x, 35.0); // 10 pad + 20 + 5 spacing
    }

    #[test]
    fn cross_align_center_and_stretch() {
        let center = lay(
            Linear::row()
                .cross_align(CrossAlign::Center)
                .push(fill(20.0, 10.0)),
            Size::new(40.0, 40.0),
        );
        assert_eq!(child_rects(center.as_ref())[0].y, 15.0); // (40-10)/2

        let stretch = lay(
            Linear::row()
                .cross_align(CrossAlign::Stretch)
                .push(fill(20.0, 10.0)),
            Size::new(40.0, 40.0),
        );
        assert_eq!(child_rects(stretch.as_ref())[0].h, 40.0);
    }

    #[test]
    fn main_align_distributes_free_space() {
        let end = lay(
            Linear::row()
                .main_align(MainAlign::End)
                .push(fill(20.0, 10.0)),
            Size::new(100.0, 40.0),
        );
        assert_eq!(child_rects(end.as_ref())[0].x, 80.0);

        let between = lay(
            Linear::row()
                .main_align(MainAlign::SpaceBetween)
                .push(fill(20.0, 10.0))
                .push(fill(20.0, 10.0)),
            Size::new(100.0, 40.0),
        );
        let r = child_rects(between.as_ref());
        assert_eq!(r[0].x, 0.0);
        assert_eq!(r[1].x, 80.0); // pushed to the far end (free = 60 between)
    }

    #[test]
    fn nested_column_in_row() {
        let row = Linear::row().child(
            Linear::column()
                .push(fill(40.0, 10.0))
                .push(fill(40.0, 10.0)),
            Length::Fixed(40.0),
        );
        let row = lay(row, Size::new(40.0, 40.0));
        // The row's child (the column) is 40 wide (Fixed) and 20 tall (its two
        // 10px rows), top-aligned because the default cross-align is Start.
        assert_eq!(
            child_rects(row.as_ref())[0],
            Rect::new(0.0, 0.0, 40.0, 20.0)
        );
    }

    #[test]
    fn wrap_flows_into_runs_on_overflow() {
        // Three 40-wide items in a 100-wide wrap (no spacing): two fit on the
        // first run (0,40), the third overflows to a second run.
        let wrap = Wrap::row()
            .push(fill(40.0, 20.0))
            .push(fill(40.0, 20.0))
            .push(fill(40.0, 20.0));
        let wrap = lay(wrap, Size::new(100.0, 100.0));
        let r = child_rects(wrap.as_ref());
        assert_eq!(r[0], Rect::new(0.0, 0.0, 40.0, 20.0));
        assert_eq!(r[1], Rect::new(40.0, 0.0, 40.0, 20.0));
        // Second run starts below the first run's height (20).
        assert_eq!(r[2], Rect::new(0.0, 20.0, 40.0, 20.0));
    }

    #[test]
    fn wrap_spacing_counts_against_the_run_width() {
        // 40 + 10 spacing + 40 = 90 fits in 100; a third + spacing (140) wraps.
        // Runs are separated by run_spacing (6).
        let wrap = Wrap::row()
            .spacing(10.0)
            .run_spacing(6.0)
            .push(fill(40.0, 20.0))
            .push(fill(40.0, 20.0))
            .push(fill(40.0, 20.0));
        let wrap = lay(wrap, Size::new(100.0, 100.0));
        let r = child_rects(wrap.as_ref());
        assert_eq!(r[0].x, 0.0);
        assert_eq!(r[1].x, 50.0); // 40 + 10 spacing
        assert_eq!(r[2], Rect::new(0.0, 26.0, 40.0, 20.0)); // 20 run + 6 run gap
    }

    #[test]
    fn wrap_measures_to_content_height() {
        // Two runs of 20px tall items with 6px run gap → 46 tall; widest run 80.
        let mut wrap = Wrap::row()
            .run_spacing(6.0)
            .push(fill(40.0, 20.0))
            .push(fill(40.0, 20.0))
            .push(fill(40.0, 20.0));
        let fonts = Fonts::new();
        let theme = Gunmetal::new(FontId(0));
        let mut ctx = LayoutCtx {
            fonts: &fonts,
            theme: &theme,
            scale: 1.0,
            viewport: Rect::ZERO,
        };
        let size = wrap.measure(Size::new(100.0, 1000.0), &mut ctx);
        assert_eq!(size, Size::new(80.0, 46.0));
    }

    #[test]
    fn wrap_line_align_centers_within_a_run() {
        // A short item next to a tall one in the same run is vertically centered
        // to the run's height (30): offset (30-10)/2 = 10.
        let wrap = Wrap::row()
            .line_align(CrossAlign::Center)
            .push(fill(20.0, 30.0))
            .push(fill(20.0, 10.0));
        let wrap = lay(wrap, Size::new(100.0, 100.0));
        let r = child_rects(wrap.as_ref());
        assert_eq!(r[0].y, 0.0);
        assert_eq!(r[1].y, 10.0);
    }

    /// A declared `run_extent` is the flow's **uniform row height**: a run
    /// carrying only a short child is still a full row, so the band a header
    /// produces is `rows × extent` at every width instead of wobbling by
    /// whatever happened to land on the last run.
    #[test]
    fn wrap_run_extent_keeps_every_run_a_full_row() {
        // Two runs: the first holds a 20px-tall item, the second only a 10px one.
        let wrap = Wrap::row()
            .run_extent(20.0)
            .run_spacing(6.0)
            .line_align(CrossAlign::Center)
            .push(fill(60.0, 20.0))
            .push(fill(60.0, 10.0));
        let wrap = lay(wrap, Size::new(100.0, 100.0));
        let r = child_rects(wrap.as_ref());
        assert_eq!(r[0].y, 0.0);
        // The short child centres in its full-height run: 20 + 6 gap + (20-10)/2.
        assert_eq!(r[1].y, 31.0, "the second run is a full row, not a 10px one");

        // …and the flow measures to that, so a host laying the band out gets
        // the same height it will draw.
        let mut wrap = Wrap::row()
            .run_extent(20.0)
            .run_spacing(6.0)
            .push(fill(60.0, 20.0))
            .push(fill(60.0, 10.0));
        let fonts = Fonts::new();
        let theme = Gunmetal::new(FontId(0));
        let mut ctx = LayoutCtx {
            fonts: &fonts,
            theme: &theme,
            scale: 1.0,
            viewport: Rect::ZERO,
        };
        assert_eq!(
            wrap.measure(Size::new(100.0, 1000.0), &mut ctx),
            Size::new(60.0, 46.0)
        );
    }

    #[test]
    fn stack_layers_fill_content() {
        let stack = lay(
            Stack::new()
                .padding(Insets::all(4.0))
                .push(fill(10.0, 10.0))
                .push(fill(10.0, 10.0)),
            Size::new(40.0, 40.0),
        );
        let r = child_rects(stack.as_ref());
        assert_eq!(r[0], Rect::new(4.0, 4.0, 32.0, 32.0));
        assert_eq!(r[1], Rect::new(4.0, 4.0, 32.0, 32.0));
    }

    #[test]
    fn grid_arranges_row_major_square_cells() {
        // 3 cells, 2 columns, no gap, 100 wide → 50px square cells; the third
        // wraps to the second row. Cells fill the width, so the child's own
        // preferred size is ignored.
        let grid = Grid::new(2)
            .push(fill(999.0, 1.0))
            .push(fill(1.0, 1.0))
            .push(fill(1.0, 1.0));
        let grid = lay(grid, Size::new(100.0, 500.0));
        let r = child_rects(grid.as_ref());
        assert_eq!(r[0], Rect::new(0.0, 0.0, 50.0, 50.0));
        assert_eq!(r[1], Rect::new(50.0, 0.0, 50.0, 50.0));
        assert_eq!(r[2], Rect::new(0.0, 50.0, 50.0, 50.0));
    }

    #[test]
    fn grid_gap_counts_against_the_cell_width() {
        // 2 columns, 10px gap, 100 wide → (100-10)/2 = 45px cells; column 2 sits
        // a cell + gap over.
        let grid = Grid::new(2)
            .gap(10.0)
            .push(fill(1.0, 1.0))
            .push(fill(1.0, 1.0));
        let grid = lay(grid, Size::new(100.0, 100.0));
        let r = child_rects(grid.as_ref());
        assert_eq!(r[0], Rect::new(0.0, 0.0, 45.0, 45.0));
        assert_eq!(r[1], Rect::new(55.0, 0.0, 45.0, 45.0));
    }

    #[test]
    fn grid_fixed_row_height_and_content_measure() {
        // 4 cells, 2 columns, fixed 20px rows, 6px gap, 100 wide.
        let mut grid = Grid::new(2)
            .gap(6.0)
            .row_height(20.0)
            .push(fill(1.0, 1.0))
            .push(fill(1.0, 1.0))
            .push(fill(1.0, 1.0))
            .push(fill(1.0, 1.0));
        let fonts = Fonts::new();
        let theme = Gunmetal::new(FontId(0));
        let mut ctx = LayoutCtx {
            fonts: &fonts,
            theme: &theme,
            scale: 1.0,
            viewport: Rect::ZERO,
        };
        // Two rows of 20 with a 6px gap → 46 tall; fills the 100px width.
        let size = grid.measure(Size::new(100.0, 1000.0), &mut ctx);
        assert_eq!(size, Size::new(100.0, 46.0));
        grid.arrange(
            Rect::from_min_size(Vec2::ZERO, Size::new(100.0, 46.0)),
            &mut ctx,
        );
        let r = child_rects(&grid);
        assert_eq!(r[2], Rect::new(0.0, 26.0, 47.0, 20.0)); // row 2: 20 + 6 gap
    }

    /// A collapsed [`Reveal`] takes no space in a `Length::Fit` column: the rows
    /// below close over it, and re-showing it puts them back.
    #[test]
    fn a_collapsed_reveal_takes_no_room_and_its_siblings_close_over_it() {
        let column = Linear::column()
            .child(Reveal::new(fill(10.0, 20.0)).height(20.0), Length::Fit)
            .child(
                Reveal::new(fill(10.0, 20.0)).height(20.0).with_shown(false),
                Length::Fit,
            )
            .child(Reveal::new(fill(10.0, 20.0)).height(20.0), Length::Fit);
        let column = lay(column, Size::new(100.0, 200.0));
        let r = child_rects(column.as_ref());
        assert_eq!(r[0], Rect::new(0.0, 0.0, 10.0, 20.0), "the first row");
        assert_eq!(
            r[1],
            Rect::new(0.0, 20.0, 0.0, 0.0),
            "the hidden row keeps its place but no extent"
        );
        assert_eq!(
            r[2],
            Rect::new(0.0, 20.0, 10.0, 20.0),
            "the third row sits where the second would have"
        );
    }

    /// Hidden, a `Reveal` is out of every tree walk — no children, no hit, no
    /// draw — which is what keeps a collapsed field out of Tab order.
    #[test]
    fn a_hidden_reveal_hides_its_whole_subtree() {
        let mut r = Reveal::new(fill(10.0, 10.0));
        assert_eq!(r.child_count(), 1, "shown: the child is reachable");
        r.set_shown(false);
        assert_eq!(r.child_count(), 0, "hidden: the subtree is gone");
        assert!(r.child(0).is_none());
        assert!(
            r.hit_test(Vec2::ZERO).is_none(),
            "and nothing hits inside it"
        );
    }

    fn measure_at(w: &mut impl Widget, size: Size) -> Size {
        let fonts = Fonts::new();
        let theme = Gunmetal::new(FontId(0));
        let mut ctx = LayoutCtx {
            fonts: &fonts,
            theme: &theme,
            scale: 1.0,
            viewport: Rect::ZERO,
        };
        w.measure(size, &mut ctx)
    }

    fn items(specs: &[(f32, f32, f32)]) -> Vec<FlexItem> {
        specs
            .iter()
            .map(|&(weight, min, max)| FlexItem {
                weight,
                min,
                max,
                size: min,
            })
            .collect()
    }

    fn sizes(items: &[FlexItem]) -> Vec<f32> {
        items.iter().map(|it| it.size).collect()
    }

    #[test]
    fn distribute_splits_proportionally() {
        let mut it = items(&[(1.0, 0.0, f32::INFINITY), (2.0, 0.0, f32::INFINITY)]);
        distribute(&mut it, 300.0);
        assert_eq!(sizes(&it), vec![100.0, 200.0]);
    }

    #[test]
    fn distribute_min_freezes_and_redistributes() {
        let mut it = items(&[(1.0, 120.0, f32::INFINITY), (1.0, 0.0, f32::INFINITY)]);
        distribute(&mut it, 200.0);
        assert_eq!(sizes(&it), vec![120.0, 80.0]);
    }

    #[test]
    fn distribute_max_freezes_and_redistributes() {
        let mut it = items(&[(1.0, 0.0, 40.0), (1.0, 0.0, f32::INFINITY)]);
        distribute(&mut it, 200.0);
        assert_eq!(sizes(&it), vec![40.0, 160.0]);
    }

    #[test]
    fn distribute_nothing_available_lands_on_minimums() {
        let mut it = items(&[(1.0, 30.0, f32::INFINITY), (3.0, 10.0, 100.0)]);
        distribute(&mut it, -50.0);
        assert_eq!(sizes(&it), vec![30.0, 10.0]);
    }

    #[test]
    fn distribute_zero_weights_keep_minimums() {
        let mut it = items(&[(0.0, 25.0, f32::INFINITY), (0.0, 0.0, 100.0)]);
        distribute(&mut it, 200.0);
        assert_eq!(sizes(&it), vec![25.0, 0.0]);
    }

    /// `Constrained` clamps both what the child reports and — min winning
    /// over max — never lets it collapse below the minimum.
    #[test]
    fn constrained_clamps_measure_min_wins_over_max() {
        let mut c = Constrained::new(fill(10.0, 10.0))
            .min_width(30.0)
            .max_height(5.0);
        assert_eq!(
            measure_at(&mut c, Size::new(100.0, 100.0)),
            Size::new(30.0, 5.0),
            "width raised to min, height capped at max"
        );

        let mut conflicted = Constrained::new(fill(10.0, 10.0))
            .min_width(50.0)
            .max_width(20.0);
        assert_eq!(
            measure_at(&mut conflicted, Size::new(100.0, 100.0)).w,
            50.0,
            "min wins over max"
        );
    }

    /// An arranged slot outside the bounds leaves the child clamped inside
    /// them, top-left anchored — occupied rect and child rect agree.
    #[test]
    fn constrained_clamps_the_arranged_rect() {
        let c = lay(
            Constrained::new(fill(10.0, 10.0))
                .max_width(20.0)
                .min_height(30.0),
            Size::new(100.0, 10.0),
        );
        assert_eq!(c.rect(), Rect::new(0.0, 0.0, 20.0, 30.0));
        assert_eq!(child_rects(c.as_ref())[0], Rect::new(0.0, 0.0, 20.0, 30.0));
    }

    /// A flex child with a min bound freezes at it and the leftover
    /// redistributes to its flex siblings (berry's `distribute` semantics).
    #[test]
    fn flex_min_freezes_and_redistributes_in_a_row() {
        let row = Linear::row()
            .child(
                Constrained::new(fill(0.0, 10.0)).min_width(120.0),
                Length::Flex(1.0),
            )
            .child(fill(0.0, 10.0), Length::Flex(1.0));
        let row = lay(row, Size::new(200.0, 10.0));
        let r = child_rects(row.as_ref());
        assert_eq!(r[0], Rect::new(0.0, 0.0, 120.0, 10.0), "frozen at min");
        assert_eq!(r[1], Rect::new(120.0, 0.0, 80.0, 10.0), "takes the rest");
    }

    /// A flex child with a max bound frees its excess share to the rest.
    #[test]
    fn flex_max_freezes_and_redistributes_in_a_row() {
        let row = Linear::row()
            .child(
                Constrained::new(fill(0.0, 10.0)).max_width(40.0),
                Length::Flex(1.0),
            )
            .child(fill(0.0, 10.0), Length::Flex(1.0));
        let row = lay(row, Size::new(200.0, 10.0));
        let r = child_rects(row.as_ref());
        assert_eq!(r[0], Rect::new(0.0, 0.0, 40.0, 10.0), "capped at max");
        assert_eq!(r[1], Rect::new(40.0, 0.0, 160.0, 10.0), "takes the excess");
    }

    /// A min-bound flex child contributes its minimum to a fit-content
    /// parent — a dialog with a bounded flex column measures at least that
    /// wide, while an unbounded flex spacer still contributes ~0.
    #[test]
    fn a_min_bound_flex_child_props_a_fit_row_open() {
        let mut row = Linear::row().child(
            Constrained::new(fill(0.0, 10.0)).min_width(120.0),
            Length::Flex(1.0),
        );
        assert_eq!(
            measure_at(&mut row, Size::new(1000.0, 100.0)).w,
            120.0,
            "content width is the flex child's minimum, not the 1000 available"
        );
    }

    /// A test stand-in for a wrapping label: fixed "ink" that folds into
    /// 10px lines at the available width.
    struct Folding {
        ink: f32,
        rect: Rect,
    }

    impl Widget for Folding {
        fn measure(&mut self, avail: Size, _ctx: &mut LayoutCtx) -> Size {
            if avail.w >= self.ink {
                Size::new(self.ink, 10.0)
            } else {
                let lines = (self.ink / avail.w.max(1.0)).ceil();
                Size::new(avail.w, lines * 10.0)
            }
        }

        fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
            self.rect = rect;
        }

        fn draw(&self, _dl: &mut DrawList, _ctx: &DrawCtx) {}

        fn rect(&self) -> Rect {
            self.rect
        }
    }

    /// The audit's stale-cross-extent defect: a wrapping child in a flex
    /// cell used to keep the height it measured at the *full* available
    /// width. Arrange now re-measures it at the resolved extent, so its
    /// cross extent matches the width it actually gets.
    #[test]
    fn flex_cells_remeasure_at_their_resolved_extent() {
        let row = Linear::row()
            .child(fill(60.0, 10.0), Length::Fixed(60.0))
            .child(
                Folding {
                    ink: 80.0,
                    rect: Rect::ZERO,
                },
                Length::Flex(1.0),
            );
        let row = lay(row, Size::new(100.0, 100.0));
        let r = child_rects(row.as_ref());
        assert_eq!(
            r[1],
            Rect::new(60.0, 0.0, 40.0, 20.0),
            "80px of ink folds into two 10px lines at the resolved 40px width"
        );
    }

    /// `height` names the row a `Length::Fit` parent cannot: the child still
    /// measures its own width, but the slot claims the height the list wants.
    #[test]
    fn reveal_height_overrides_the_childs_measured_one() {
        let fonts = Fonts::new();
        let theme = Gunmetal::new(FontId(0));
        let mut ctx = LayoutCtx {
            fonts: &fonts,
            theme: &theme,
            scale: 1.0,
            viewport: Rect::ZERO,
        };
        let mut plain = Reveal::new(fill(12.0, 30.0));
        assert_eq!(
            plain.measure(Size::new(100.0, 100.0), &mut ctx),
            Size::new(12.0, 30.0)
        );
        let mut pinned = Reveal::new(fill(12.0, 30.0)).height(18.0);
        assert_eq!(
            pinned.measure(Size::new(100.0, 100.0), &mut ctx),
            Size::new(12.0, 18.0)
        );
    }
}
