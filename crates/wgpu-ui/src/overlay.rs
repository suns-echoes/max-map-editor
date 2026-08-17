//! Overlay widgets that draw above the rest of the tree: [`Select`] (a dropdown)
//! uses the popup mechanism; [`Tabs`] switches content under a header bar.

use crate::color::Rgba;
use crate::draw::DrawList;
use crate::event::{Event, Key, PointerButton};
use crate::geom::{Insets, Rect, Size, Vec2};
use crate::interact::{WidgetId, WidgetState, next_id};
use crate::scroll::Scroller;
use crate::text::Fonts;
use crate::theme::{
    Emboss, POPUP_FRAME, ROW_FLOOR_ACTIVE, ROW_FLOOR_ACTIVE_HOVER, ROW_FLOOR_HOVER, Role, TextRole,
    Theme,
};
use crate::widget::{DrawCtx, EventCtx, LayoutCtx, Widget};

/// Left padding (px) of the label / option text in a select box or option row.
const SELECT_PAD: f32 = 6.0;
/// Height (px) of a separator gap between option-row groups — the theme's
/// line centered with ~2px margins (the menu separator's proportions).
const SELECT_SEP_H: f32 = 6.0;

/// How far rows below `i` are pushed down by the separators above them.
fn sep_shift(separators: &[usize], i: usize) -> f32 {
    separators.iter().filter(|&&s| s < i).count() as f32 * SELECT_SEP_H
}
/// Right-gutter width (px) reserved for the caret in a closed select box.
const CARET_GUTTER: f32 = 16.0;
/// The caret triangle's height (px).
const CARET_SIZE: f32 = 7.0;

/// The size variant of a [`Select`] — and of the shared [`draw_select_box`] /
/// [`draw_select_popup`] helpers every host draws through. `Standard` is the
/// dialog control (body text, full control-height rows); `Small` is the compact
/// panel-toolbar control (small text, tighter rows). Both render the *same* style
/// — button-face box, caret, panel popup, accent wash, 1px border — differing
/// only in font size and row height, so every dropdown in the app reads alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SelectSize {
    #[default]
    Standard,
    Small,
}

impl SelectSize {
    /// Option-row height (px) of the compact ([`SelectSize::Small`]) popup —
    /// public so a host that owns its own popup geometry/hit-testing derives
    /// its row height from the same source the shared renderer draws with.
    pub const SMALL_ROW_H: f32 = 18.0;

    /// The text role (font size) for this variant.
    pub fn text_role(self) -> TextRole {
        match self {
            SelectSize::Standard => TextRole::Body,
            SelectSize::Small => TextRole::Small,
        }
    }

    /// The option-row height (px) for this variant.
    pub fn row_height(self, theme: &dyn Theme) -> f32 {
        match self {
            SelectSize::Standard => theme.metrics().control_height,
            SelectSize::Small => Self::SMALL_ROW_H,
        }
    }
}

/// Draws a caret triangle centered on `c`, `size` px tall, pointing up (`up`) or
/// down. The [`DrawList`] has no triangle primitive, so it's stepped 1px rows.
fn caret_triangle(dl: &mut DrawList, c: Vec2, size: f32, up: bool, color: Rgba) {
    let rows = size.round() as i32;
    for i in 0..rows {
        let w = size - i as f32 * 2.0;
        if w <= 0.0 {
            break;
        }
        // `i` steps in from the apex; for an up-caret place the row from the far
        // edge so the wide base lands at the bottom (points up), else from the top.
        let row = if up { rows - 1 - i } else { i };
        dl.fill_rect(
            Rect::new(c.x - w * 0.5, c.y - size * 0.5 + row as f32, w, 1.0),
            color,
        );
    }
}

/// Draws a closed select value box: a secondary button face, the `label` (raised
/// ink, clipped clear of the caret gutter), and a caret (pointing up when `open`).
/// `size` selects the font. Appearance only — the host owns geometry and
/// hit-testing (the retained [`Select`] widget, or a bespoke immediate-mode host).
#[allow(clippy::too_many_arguments)]
pub fn draw_select_box(
    dl: &mut DrawList,
    theme: &dyn Theme,
    fonts: &Fonts,
    rect: Rect,
    label: &str,
    open: bool,
    state: WidgetState,
    size: SelectSize,
) {
    theme.button(dl, rect, Role::Neutral, state);
    let role = size.text_role();
    let px = theme.font_px(role);
    let label_area = Rect::new(rect.x, rect.y, (rect.w - CARET_GUTTER).max(0.0), rect.h);
    let baseline = Vec2::new(rect.x + SELECT_PAD, rect.center().y + px * 0.34);
    dl.push_clip(label_area);
    theme.text_colored(
        dl,
        fonts,
        baseline,
        label,
        role,
        Emboss::Raised,
        theme.ink(),
    );
    dl.pop_clip();
    caret_triangle(
        dl,
        Vec2::new(rect.right() - 10.0, rect.center().y),
        CARET_SIZE,
        open,
        theme.ink(),
    );
}

/// Draws the open option list for a select: a [`popup`](Theme::popup) surface
/// (a 1px outset frame), an accent wash on the hovered *and* currently-`selected`
/// rows (inset so it never overflows the frame), and engraved ink option text.
/// `popup_rect` and `row_h` are the host's geometry (matching its own
/// hit-testing); `separators` lists option indices after which a slim
/// separator gap is drawn (the popup grows [`SELECT_SEP_H`] per entry — pass
/// `&[]` for a plain list); `size` selects the font.
///
/// `scroll` is the [`Scroller`] the host laid out **over `popup_rect`** for a
/// list taller than its window (a [`Select::max_visible`] popup, or one clamped
/// to the viewport): the rows shift up by its offset and it paints its own bar
/// down the right-hand column, which the rows make room for. A list that always
/// fits passes `None`.
#[allow(clippy::too_many_arguments)]
pub fn draw_select_popup<S: AsRef<str>>(
    dl: &mut DrawList,
    theme: &dyn Theme,
    fonts: &Fonts,
    popup_rect: Rect,
    row_h: f32,
    labels: &[S],
    separators: &[usize],
    selected: Option<usize>,
    hover: Option<usize>,
    size: SelectSize,
    scroll: Option<&Scroller>,
) {
    let bar = scroll.filter(|s| s.has_bar());
    let (scroll, bar_w) = (
        scroll.map_or(0.0, Scroller::offset),
        bar.map_or(0.0, |s| s.track_rect().w),
    );
    // Rows stop short of the bar's column, so no wash or label runs under it.
    let row_w = (popup_rect.w - bar_w).max(0.0);
    theme.popup(dl, popup_rect);
    // Clip the rows to the inside of the popup's 1px outset frame: a scrolled
    // list's partially-visible edge rows must not paint over the frame (the
    // clip is a no-op for a full-height list — highlights and text already
    // keep inside it).
    dl.push_clip(popup_rect.inset(Insets::all(POPUP_FRAME)));
    let role = size.text_role();
    let px = theme.font_px(role);
    let n = labels.len();
    for (i, opt) in labels.iter().enumerate() {
        let row = Rect::new(
            popup_rect.x,
            popup_rect.y + i as f32 * row_h + sep_shift(separators, i) - scroll,
            row_w,
            row_h,
        );
        // A scrolled-out row (and its separator, which hangs just below it)
        // draws nothing.
        if row.bottom() + SELECT_SEP_H < popup_rect.y || row.y > popup_rect.bottom() {
            continue;
        }
        if separators.contains(&i) {
            // The gap below this row: the theme's rule, inset like option text.
            theme.separator(
                dl,
                Rect::new(
                    row.x + SELECT_PAD,
                    row.bottom(),
                    row.w - 2.0 * SELECT_PAD,
                    SELECT_SEP_H,
                ),
            );
        }
        let (is_sel, is_hov) = (Some(i) == selected, Some(i) == hover);
        if is_sel || is_hov {
            // Inset the highlight off the popup's 1px outset frame — a row
            // highlight stays inside the frame (only a button covers its bevel).
            let top = if i == 0 { POPUP_FRAME } else { 0.0 };
            let bot = if i + 1 == n { POPUP_FRAME } else { 0.0 };
            let inner = Rect::new(
                row.x + POPUP_FRAME,
                row.y + top,
                (row.w - 2.0 * POPUP_FRAME).max(0.0),
                (row.h - top - bot).max(0.0),
            );
            let floor = match (is_sel, is_hov) {
                (true, true) => ROW_FLOOR_ACTIVE_HOVER,
                (true, false) => ROW_FLOOR_ACTIVE,
                _ => ROW_FLOOR_HOVER,
            };
            theme.accent_row(dl, inner, floor);
        }
        let baseline = Vec2::new(row.x + SELECT_PAD, row.center().y + px * 0.34);
        dl.push_clip(row);
        theme.text_colored(
            dl,
            fonts,
            baseline,
            opt.as_ref(),
            role,
            Emboss::Engraved,
            theme.ink(),
        );
        dl.pop_clip();
    }
    // Inside the frame clip like the rows: the bar's own column runs the full
    // height of the window, and the popup's 1px outset frame trims it.
    if let Some(s) = bar {
        s.draw_bar(dl, theme);
    }
    dl.pop_clip();
}

/// Places a popup `ph` px tall under (or over) `box_rect`, kept inside
/// `viewport`: it drops below, flips above when it would not fit below but fits
/// above, and — for a list taller than the surface itself — fills the viewport
/// and pins to the top. Horizontally it shifts so neither edge is cropped. An
/// **empty** `viewport` is unconstrained: the list simply drops below.
///
/// A host that wants breathing room at the window edges passes an inset
/// viewport; the geometry itself carries no margin.
fn popup_rect_in(box_rect: Rect, ph: f32, viewport: Rect) -> Rect {
    let below = box_rect.bottom();
    if viewport.w <= 0.0 || viewport.h <= 0.0 {
        return Rect::new(box_rect.x, below, box_rect.w, ph);
    }
    // A list that fits nowhere is *shortened* to the surface rather than
    // cropped by it: the window then scrolls (and shows a bar) over the whole
    // list, instead of stranding every option past the bottom edge.
    let ph = ph.min(viewport.h);
    let above = box_rect.y - ph;
    let y = if below + ph <= viewport.bottom() {
        below
    } else if above >= viewport.y {
        above
    } else {
        (viewport.bottom() - ph).max(viewport.y)
    };
    let x = box_rect
        .x
        .min(viewport.right() - box_rect.w)
        .max(viewport.x);
    Rect::new(x, y, box_rect.w, ph)
}

/// A dropdown that opens a popup list of options.
#[must_use]
pub struct Select {
    id: WidgetId,
    options: Vec<String>,
    /// Option indices after which the popup draws a separator gap.
    sep_after: Vec<usize>,
    selected: usize,
    open: bool,
    disabled: bool,
    hover: Option<usize>,
    /// An option picked since the host last polled [`take_pick`](Select::take_pick).
    pick: Option<usize>,
    size: SelectSize,
    row_h: f32,
    rect: Rect,
    /// The option list's rect, settled at `arrange` against
    /// [`LayoutCtx::viewport`] — so the drawn list and its hit-test are one
    /// geometry even when the list flips up or is shifted off an edge.
    popup: Rect,
    /// Popup row cap (see [`max_visible`](Select::max_visible)); `None` = the
    /// list always shows every option.
    max_visible: Option<usize>,
    /// The option list's scroll machine, laid out over [`popup`](Self::popup) —
    /// the offset, the wheel, the bar and its drag. Inert (no bar, offset `0`)
    /// whenever the whole list fits its window.
    scroller: Scroller,
}

impl Select {
    pub fn new(options: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            id: next_id(),
            options: options.into_iter().map(Into::into).collect(),
            sep_after: Vec::new(),
            selected: 0,
            open: false,
            disabled: false,
            hover: None,
            pick: None,
            size: SelectSize::Standard,
            row_h: 24.0,
            rect: Rect::ZERO,
            popup: Rect::ZERO,
            max_visible: None,
            scroller: Scroller::new(),
        }
    }

    /// Replaces the option list, **keeping the popup open** if it is — the
    /// per-frame refresh a host needs when the options are derived from live
    /// state (the tile packs of the open map). The selection is clamped into
    /// the new list; the caller re-points it with
    /// [`set_selected`](Select::set_selected) when the labels moved.
    pub fn set_options(&mut self, options: impl IntoIterator<Item = impl Into<String>>) {
        self.options = options.into_iter().map(Into::into).collect();
        let last = self.options.len().saturating_sub(1);
        self.selected = self.selected.min(last);
        self.hover = self.hover.map(|h| h.min(last));
    }

    /// Replaces the separator positions (see
    /// [`separator_after`](Select::separator_after)).
    pub fn set_separators(&mut self, after: impl IntoIterator<Item = usize>) {
        self.sep_after = after.into_iter().collect();
    }

    /// Takes the option picked since this was last called — the host-facing
    /// half of a pick, for an owner that cannot see `Ui::fired` because it *is*
    /// a widget (a panel root hosting this one). Firing is unchanged: a pick
    /// still fires the widget for tree-level hosts. Poll after dispatch, like
    /// [`Tabs::take_close_request`].
    pub fn take_pick(&mut self) -> Option<usize> {
        self.pick.take()
    }

    /// Draws a separator gap in the popup after option `i` (groups related
    /// options, e.g. built-ins above user entries). Option indices are
    /// unaffected — a separator is a gap, not a row.
    pub fn separator_after(mut self, i: usize) -> Self {
        self.sep_after.push(i);
        self
    }

    /// Makes this the compact ([`SelectSize::Small`]) variant — small font, tight
    /// rows — the size used in panel toolbars (the default is `Standard`).
    pub fn small(mut self) -> Self {
        self.size = SelectSize::Small;
        self
    }

    pub fn with_selected(mut self, i: usize) -> Self {
        self.selected = i.min(self.options.len().saturating_sub(1));
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Caps the popup at `n` visible option rows; a longer list scrolls — a
    /// scrollbar down its right-hand column (drag the thumb, page the track),
    /// the wheel, PgUp/PgDn/Home/End, and the arrow keys keeping their hover
    /// row in view. Opening scrolls the current selection into view.
    ///
    /// The default shows every option — a long list (a unit-type or font
    /// picker) is what this exists for, so the popup is a tidy window rather
    /// than a full-height slab. An *uncapped* list still scrolls when it does
    /// not fit: one taller than the viewport is shortened to it, and gets the
    /// same bar.
    pub fn max_visible(mut self, n: usize) -> Self {
        self.max_visible = Some(n.max(1));
        self
    }

    pub fn id(&self) -> WidgetId {
        self.id
    }

    /// True while the option list is dropped down.
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn set_selected(&mut self, i: usize) {
        self.selected = i.min(self.options.len().saturating_sub(1));
    }

    pub fn selected_text(&self) -> &str {
        self.options.get(self.selected).map_or("", String::as_str)
    }

    /// The open option list's rect (logical px), settled at `arrange` — below
    /// the box, or above it when it would not fit below. Valid whether or not
    /// the list is open; a host hit-testing on this widget's behalf (a panel
    /// root routing a press to its child) asks this and
    /// [`is_open`](Select::is_open).
    pub fn popup_rect(&self) -> Rect {
        self.popup
    }

    /// The full option list's height: one row per option, plus the separator
    /// gaps — what the popup window scrolls over when capped.
    fn content_h(&self) -> f32 {
        self.row_h * self.options.len() as f32 + sep_shift(&self.sep_after, self.options.len())
    }

    /// Whether [`max_visible`](Select::max_visible) shortens this list — the
    /// one scroll case `measure` can see coming (the other, a list taller than
    /// the viewport, is only known once `arrange` settles the popup).
    fn capped(&self) -> bool {
        matches!(self.max_visible, Some(n) if self.options.len() > n)
    }

    /// The popup *window's* height: the whole list, capped to
    /// [`max_visible`](Select::max_visible) rows when set.
    fn popup_h(&self) -> f32 {
        match self.max_visible {
            Some(n) if (self.options.len() > n) => self.row_h * n as f32,
            _ => self.content_h(),
        }
    }

    /// Scrolls so option `i`'s row sits fully inside the popup window — the
    /// open-on-selection and arrow-key follow behavior of a scrolling list.
    fn scroll_to(&mut self, i: usize) {
        let top = i as f32 * self.row_h + sep_shift(&self.sep_after, i);
        let bottom = top + self.row_h;
        let at = self.scroller.offset();
        if top < at {
            self.scroller.set_offset(top);
        } else if bottom > at + self.popup.h {
            self.scroller.set_offset(bottom - self.popup.h);
        }
    }

    /// The option row under `p` — `None` outside the popup, on the scrollbar's
    /// column, *and* in a separator gap.
    fn option_at(&self, p: Vec2) -> Option<usize> {
        let pr = self.popup_rect();
        if !pr.contains(p) || (self.scroller.has_bar() && p.x >= self.scroller.track_rect().x) {
            return None;
        }
        let y_in = p.y - pr.y + self.scroller.offset();
        (0..self.options.len()).find(|&i| {
            let y = i as f32 * self.row_h + sep_shift(&self.sep_after, i);
            y_in >= y && y_in < y + self.row_h
        })
    }
}

impl Widget for Select {
    /// Wide enough that the widest option renders in full, in the box *and* in
    /// the popup: [`draw_select_box`] insets the label by [`SELECT_PAD`] and
    /// clips it [`CARET_GUTTER`] short of the right edge, so that pair is
    /// exactly the chrome a closed box spends — and it is the binding one, the
    /// popup rows spending only `2 * SELECT_PAD` — plus the scrollbar column
    /// when [`max_visible`](Select::max_visible) shortens the list, which is
    /// what can tip the popup's demand past the box's. Measuring the *drawn*
    /// chrome (rather than a theme padding this widget never uses) is what lets
    /// a compact dropdown share a flow run with its neighbours.
    fn measure(&mut self, _avail: Size, ctx: &mut LayoutCtx) -> Size {
        let m = ctx.theme.metrics();
        self.row_h = self.size.row_height(ctx.theme);
        let font = ctx.theme.font();
        let px = ctx.theme.font_px(self.size.text_role());
        let widest = self
            .options
            .iter()
            .map(|o| ctx.fonts.measure(font, o, px))
            .fold(0.0_f32, f32::max);
        let bar = if self.capped() { m.scrollbar } else { 0.0 };
        Size::new(
            (widest + SELECT_PAD + CARET_GUTTER)
                .max(widest + 2.0 * SELECT_PAD + bar)
                .max(m.button_min_width),
            self.row_h,
        )
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        // Settled here as well as in `measure`: a host that arranges this widget
        // *directly* — a panel placing it over its own cell instead of laying out
        // a tree — never calls `measure`, and a `row_h` left at its constructor
        // default would put the drawn rows and `option_at` on different grids.
        self.row_h = self.size.row_height(ctx.theme);
        // The list's placement is a fact about this layout, not about the next
        // pointer event: settle it once here, so the drawn popup and the rows
        // `option_at` hit-tests are one rect even when it flips or shifts.
        self.popup = popup_rect_in(rect, self.popup_h(), ctx.viewport);
        // The scroll machine rides that same rect — so the bar it paints, the
        // rows it shifts and the column `option_at` excludes are one geometry.
        // (`layout` re-clamps the offset, so a shrunken option list —
        // `set_options` — cannot leave the view scrolled past its end.)
        self.scroller.layout(ctx, self.popup, self.content_h());
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if ctx.is_base() {
            let state = WidgetState {
                hovered: ctx.is_hovered(self.id),
                focused: ctx.is_focused(self.id) || self.open,
                disabled: self.disabled,
                ..Default::default()
            };
            draw_select_box(
                dl,
                ctx.theme,
                ctx.fonts,
                self.rect,
                self.selected_text(),
                self.open,
                state,
                self.size,
            );
        } else if self.open {
            // Overlay pass: the open popup list.
            draw_select_popup(
                dl,
                ctx.theme,
                ctx.fonts,
                self.popup_rect(),
                self.row_h,
                &self.options,
                &self.sep_after,
                Some(self.selected),
                self.hover,
                self.size,
                Some(&self.scroller),
            );
        }
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        if self.disabled {
            return false;
        }
        if !self.open {
            if let Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                ..
            } = ev
                && ctx.is_target(self.id)
            {
                self.open = true;
                self.hover = Some(self.selected);
                self.scroller.set_offset(0.0);
                self.scroll_to(self.selected);
                ctx.consume_pointer();
                ctx.request_focus(self.id);
                ctx.open_popup(self.id);
                return true;
            }
            // Keyboard open on the focused (Tab-reachable) box — guarded so it
            // never replaces another live popup's ownership.
            if let Event::Key {
                key: Key::Enter | Key::Space | Key::Down,
                pressed: true,
                ..
            } = ev
                && ctx.is_target(self.id)
                && !ctx.any_popup_open()
            {
                self.open = true;
                self.hover = Some(self.selected);
                self.scroller.set_offset(0.0);
                self.scroll_to(self.selected);
                ctx.consume_keyboard();
                ctx.open_popup(self.id);
                return true;
            }
            return false;
        }
        // Open: the Ui routes all pointer events here.
        //
        // The scrollbar is chrome *over* the option rows, so it gets first
        // refusal on everything but the wheel (whose step is this widget's, in
        // rows) — a press on its column drags the thumb or pages the track
        // instead of picking, and the drag keeps the pointer while it runs.
        // Offered only while open: a closed dropdown must not swallow the
        // host's PgUp/PgDn/Home/End just because its list would scroll.
        if !matches!(ev, Event::Scroll { .. }) && self.scroller.event(ev, ctx, self.id) {
            return true;
        }
        match ev {
            Event::PointerMoved { .. } => {
                self.hover = self.option_at(ctx.pointer);
                ctx.consume_pointer();
                true
            }
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                ..
            } => {
                if let Some(i) = self.option_at(ctx.pointer) {
                    self.selected = i;
                    self.pick = Some(i);
                    ctx.fire(self.id, None);
                    self.open = false;
                    ctx.close_popup();
                } else if !self.popup_rect().contains(ctx.pointer) {
                    self.open = false;
                    ctx.close_popup();
                }
                // Inside the popup but on no option (a separator gap): swallow
                // the click and keep the list open, like a menu does.
                ctx.consume_pointer();
                true
            }
            // Any other button dismisses without picking, wherever it lands —
            // a right-click is never a pick, and swallowing it keeps it from
            // acting on whatever lies underneath.
            Event::PointerButton { pressed: true, .. } => {
                self.open = false;
                self.hover = None;
                ctx.close_popup();
                ctx.consume_pointer();
                true
            }
            // Wheel over a scrolling list: three rows a notch (the list's own
            // step, not the scroller's pixel one), and re-derive the hover from
            // what now sits under the (stationary) pointer.
            Event::Scroll { delta, .. } => {
                let dy = match delta {
                    crate::event::ScrollDelta::Lines(v) => v.y * self.row_h * 3.0,
                    crate::event::ScrollDelta::Pixels(v) => v.y,
                };
                self.scroller.scroll_by(dy);
                self.hover = self.option_at(ctx.pointer);
                ctx.consume_pointer();
                true
            }
            Event::Key {
                key: Key::Up,
                pressed: true,
                ..
            } if ctx.is_target(self.id) => {
                let cur = self.hover.unwrap_or(self.selected);
                let next = cur.saturating_sub(1);
                self.hover = Some(next);
                self.scroll_to(next);
                ctx.consume_keyboard();
                true
            }
            Event::Key {
                key: Key::Down,
                pressed: true,
                ..
            } if ctx.is_target(self.id) => {
                let cur = self.hover.unwrap_or(self.selected);
                let next = (cur + 1).min(self.options.len().saturating_sub(1));
                self.hover = Some(next);
                self.scroll_to(next);
                ctx.consume_keyboard();
                true
            }
            Event::Key {
                key: Key::Enter | Key::Space,
                pressed: true,
                ..
            } if ctx.is_target(self.id) => {
                if let Some(i) = self.hover
                    && i < self.options.len()
                {
                    self.selected = i;
                    self.pick = Some(i);
                    ctx.fire(self.id, None);
                }
                self.open = false;
                ctx.close_popup();
                ctx.consume_keyboard();
                true
            }
            Event::Key {
                key: Key::Escape,
                pressed: true,
                ..
            } if ctx.is_target(self.id) => {
                self.open = false;
                ctx.close_popup();
                ctx.consume_keyboard();
                true
            }
            // Window focus lost: the dismissing click may land in another
            // window and never arrive here — drop the list now, the same way a
            // dragging widget drops its `pressed` (the popup-owner contract on
            // [`EventCtx::open_popup`]).
            Event::Focus(false) => {
                self.open = false;
                self.hover = None;
                ctx.close_popup();
                false
            }
            // Losing the keyboard closes the list: open-but-unfocused would eat
            // every pointer event while arrows and Escape go elsewhere. On the
            // blur path the `close_popup` request is ignored — the `Ui` drops
            // the routing state itself; this arm keeps the widget's flag true
            // to it (`self.open ⇔ ctx.popup_open(self.id)`).
            Event::Blur(_) if ctx.is_target(self.id) => {
                self.open = false;
                self.hover = None;
                ctx.close_popup();
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

    fn accepts_focus(&self) -> bool {
        true
    }
}

/// Width (logical px) of a closeable tab's `×` button.
const CLOSE_W: f32 = 16.0;

/// A tabbed container: a header bar of tab buttons switching a single visible
/// content widget. Optionally [`closeable`](Tabs::closeable) (each header gets a
/// `×`); headers compress to fit when their natural widths overflow the bar.
#[must_use]
pub struct Tabs {
    id: WidgetId,
    tabs: Vec<(String, Box<dyn Widget>)>,
    active: usize,
    closeable: bool,
    /// Paint the header bar on a raised [`panel`](Theme::panel) band (see
    /// [`framed`](Tabs::framed)).
    framed: bool,
    /// A pending close request (tab index), taken by the host after dispatch.
    close_request: Option<usize>,
    /// The header index under the pointer (only that header paints hovered).
    hover: Option<usize>,
    bar_h: f32,
    headers: Vec<Rect>,
    rect: Rect,
}

impl Tabs {
    pub fn new() -> Self {
        Self {
            id: next_id(),
            tabs: Vec::new(),
            active: 0,
            closeable: false,
            framed: false,
            close_request: None,
            hover: None,
            bar_h: 24.0,
            headers: Vec::new(),
            rect: Rect::ZERO,
        }
    }

    pub fn tab(mut self, label: impl Into<String>, content: impl Widget + 'static) -> Self {
        self.tabs.push((label.into(), Box::new(content)));
        self
    }

    /// Shows a `×` close button on each tab header. A click on it fires the
    /// `Tabs` and records the tab index for the host to read with
    /// [`take_close_request`](Tabs::take_close_request) (the host owns removal).
    pub fn closeable(mut self) -> Self {
        self.closeable = true;
        self
    }

    /// Paints the header bar's full-width band as a raised
    /// [`panel`](Theme::panel) box — the same face a menu bar sits on — with
    /// the tab buttons over it. For a bar mounted flush against its host's
    /// edges (a dialog whose tab strip runs edge to edge under the titlebar);
    /// the default leaves the bar's remainder transparent.
    pub fn framed(mut self) -> Self {
        self.framed = true;
        self
    }

    pub fn id(&self) -> WidgetId {
        self.id
    }

    pub fn active(&self) -> usize {
        self.active
    }

    pub fn set_active(&mut self, i: usize) {
        if i < self.tabs.len() {
            self.active = i;
        }
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// Takes the pending close request (the tab whose `×` was clicked), clearing
    /// it. Call after dispatch; the host then removes that tab as it sees fit.
    pub fn take_close_request(&mut self) -> Option<usize> {
        self.close_request.take()
    }

    /// The header (tab button) rect for tab `i`, valid after layout.
    pub fn header_rect(&self, i: usize) -> Option<Rect> {
        self.headers.get(i).copied()
    }

    /// The `×` hit rect for header `i` (the right [`CLOSE_W`] of its header),
    /// when closeable.
    fn close_rect(&self, i: usize) -> Option<Rect> {
        if !self.closeable {
            return None;
        }
        let hr = *self.headers.get(i)?;
        Some(Rect::new(hr.right() - CLOSE_W, hr.y, CLOSE_W, hr.h))
    }

    fn content_rect(&self) -> Rect {
        Rect::new(
            self.rect.x,
            self.rect.y + self.bar_h,
            self.rect.w,
            (self.rect.h - self.bar_h).max(0.0),
        )
    }

    /// How far a [`framed`](Tabs::framed) bar insets its buttons off the band
    /// edge: past the band's border stroke *and* its bevel ring, so both stay
    /// visible around the buttons. Zero for an unframed bar.
    fn frame_inset(&self, bevel: f32) -> f32 {
        if self.framed { 2.0 * bevel } else { 0.0 }
    }
}

impl Default for Tabs {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Tabs {
    /// Natural size: the active page's measured size (at least the header
    /// bar's natural width), plus the bar — clamped to `avail` like a scroll
    /// area. Claiming `avail` outright would balloon an auto-sized dialog to
    /// the whole screen.
    fn measure(&mut self, avail: Size, ctx: &mut LayoutCtx) -> Size {
        self.bar_h = ctx.theme.metrics().control_height;
        let inner = Size::new(avail.w, (avail.h - self.bar_h).max(0.0));
        let content = match self.tabs.get_mut(self.active) {
            Some((_, c)) => c.measure(inner, ctx),
            None => Size::ZERO,
        };
        let font = ctx.theme.font();
        let px = ctx.theme.font_px(TextRole::Body);
        let pad = ctx.theme.metrics().pad;
        let close_w = if self.closeable { CLOSE_W } else { 0.0 };
        let bar_w: f32 = self
            .tabs
            .iter()
            .map(|(label, _)| ctx.fonts.measure(font, label, px) + 2.0 * pad + close_w)
            .sum::<f32>()
            + 2.0 * self.frame_inset(ctx.theme.metrics().bevel);
        Size::new(
            content.w.max(bar_w).min(avail.w),
            (content.h + self.bar_h).min(avail.h),
        )
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        // Natural header widths (label + padding + any close button).
        let font = ctx.theme.font();
        let px = ctx.theme.font_px(TextRole::Body);
        let pad = ctx.theme.metrics().pad;
        let close_w = if self.closeable { CLOSE_W } else { 0.0 };
        let widths: Vec<f32> = self
            .tabs
            .iter()
            .map(|(label, _)| ctx.fonts.measure(font, label, px) + 2.0 * pad + close_w)
            .collect();
        // A framed bar keeps its buttons inside the band's border + bevel ring
        // (its outer frame) rather than covering them.
        let inset = self.frame_inset(ctx.theme.metrics().bevel);
        let bar_w = (rect.w - 2.0 * inset).max(0.0);
        // Compress proportionally to fit the bar when the row overflows.
        let total: f32 = widths.iter().sum();
        let scale = if total > bar_w && total > 0.0 {
            bar_w / total
        } else {
            1.0
        };
        self.headers.clear();
        let mut x = rect.x + inset;
        for w in &widths {
            let w = w * scale;
            self.headers
                .push(Rect::new(x, rect.y + inset, w, self.bar_h - 2.0 * inset));
            x += w;
        }
        let cr = self.content_rect();
        if let Some((_, c)) = self.tabs.get_mut(self.active) {
            c.arrange(cr, ctx);
        }
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if ctx.is_base() {
            if self.framed {
                // The full-width raised band under the headers — the menu-bar
                // face. The buttons sit inside its border + bevel ring (see
                // `frame_inset`), so the band's frame stays visible around them.
                ctx.theme.panel(
                    dl,
                    Rect::new(self.rect.x, self.rect.y, self.rect.w, self.bar_h),
                );
            }
            let close_w = if self.closeable { CLOSE_W } else { 0.0 };
            for (i, (label, _)) in self.tabs.iter().enumerate() {
                let hr = self.headers[i];
                ctx.theme.button(
                    dl,
                    hr,
                    Role::Neutral,
                    WidgetState {
                        hovered: self.hover == Some(i),
                        selected: i == self.active,
                        ..Default::default()
                    },
                );
                let px = ctx.theme.font_px(TextRole::Body);
                // Label area excludes the close button; clip so compressed tabs
                // don't bleed into their neighbor.
                let label_w = (hr.w - close_w).max(0.0);
                let label_area = Rect::new(hr.x, hr.y, label_w, hr.h);
                let tw = ctx.fonts.measure(ctx.theme.font(), label, px);
                let baseline = Vec2::new(
                    (hr.x + label_w * 0.5 - tw * 0.5).max(hr.x + 4.0),
                    hr.center().y + px * 0.34,
                );
                dl.push_clip(label_area);
                // Tab headers are raised (button) faces: full emboss.
                ctx.theme.text_em(
                    dl,
                    ctx.fonts,
                    baseline,
                    label,
                    TextRole::Body,
                    Emboss::Raised,
                );
                dl.pop_clip();
                // Close glyph in the reserved right column — ASCII `x`, so it
                // renders in a font that carries only the ASCII range (the same
                // rule as `Theme::ellipsized`'s three-dot marker).
                if let Some(cr) = self.close_rect(i) {
                    let xw = ctx.fonts.measure(ctx.theme.font(), "x", px);
                    let xb = Vec2::new(cr.center().x - xw * 0.5, cr.center().y + px * 0.34);
                    ctx.theme.text(dl, ctx.fonts, xb, "x", TextRole::Body);
                }
            }
        }
        if let Some((_, c)) = self.tabs.get(self.active) {
            c.draw(dl, ctx);
        }
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        // Passive per-header hover tracking (unconsumed) so draw lights only
        // the header under the pointer, not every header at once.
        match ev {
            Event::PointerMoved { .. } => {
                self.hover = self.headers.iter().position(|r| r.contains(ctx.pointer));
            }
            Event::PointerLeft => self.hover = None,
            _ => {}
        }
        if let Some((_, c)) = self.tabs.get_mut(self.active)
            && c.event(ev, ctx)
        {
            return true;
        }
        if let Event::PointerButton {
            button: PointerButton::Primary,
            pressed: true,
            ..
        } = ev
            && ctx.is_target(self.id)
        {
            if let Some(i) = self.headers.iter().position(|r| r.contains(ctx.pointer)) {
                // A hit in the close column requests removal instead of
                // switching — reported ONLY via `take_close_request`, while a
                // switch ONLY fires (and only on an actual change), so the two
                // outcomes are never ambiguous to the host.
                if self
                    .close_rect(i)
                    .is_some_and(|cr| cr.contains(ctx.pointer))
                {
                    self.close_request = Some(i);
                } else if i != self.active {
                    self.active = i;
                    ctx.fire(self.id, Some(i as u64));
                }
                ctx.consume_pointer();
                return true;
            }
            if ctx.pointer.y < self.rect.y + self.bar_h {
                ctx.consume_pointer();
                return true;
            }
        }
        false
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn id(&self) -> WidgetId {
        self.id
    }

    fn child_count(&self) -> usize {
        usize::from(!self.tabs.is_empty())
    }

    fn child(&self, i: usize) -> Option<&dyn Widget> {
        (i == 0)
            .then(|| self.tabs.get(self.active).map(|(_, c)| c.as_ref()))
            .flatten()
    }

    fn child_mut(&mut self, i: usize) -> Option<&mut dyn Widget> {
        (i == 0)
            .then(|| self.tabs.get_mut(self.active).map(|(_, c)| c.as_mut()))
            .flatten()
    }

    fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
        if !self.rect.contains(pos) {
            return None;
        }
        if pos.y < self.rect.y + self.bar_h {
            return Some(self.id);
        }
        self.tabs
            .get(self.active)
            .and_then(|(_, c)| c.hit_test(pos))
            .or(Some(self.id))
    }
}
