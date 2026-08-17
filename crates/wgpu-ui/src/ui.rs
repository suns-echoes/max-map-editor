//! The [`Ui`] root: owns the retained widget tree and the cross-cutting hover /
//! focus / pointer-capture state, and drives the three passes — `layout`,
//! `dispatch`, `draw`.
//!
//! Behavior lives in the widgets; the `Ui` only resolves *which* widget an event
//! targets (topmost hit, or the pointer-capturing widget, or the focused widget)
//! and exposes that via the event context. Outcomes flow back as a [`Response`]
//! (for host input passthrough) plus pollable fired-widget ids and action tags.

use std::any::Any;
use std::time::{Duration, Instant};

use crate::draw::DrawList;
use crate::event::{BlurCause, Event, Key, Modifiers, PointerButton};
use crate::geom::{Rect, Size, Vec2};
use crate::interact::{CursorIcon, Response, WidgetId};
use crate::text::Fonts;
use crate::theme::Theme;
use crate::widget::{DrawCtx, DrawPass, EventCtx, LayoutCtx, PopupOp, Widget};

/// How close together in time two primary presses must land to count as a
/// double click, and how far apart on screen they may be (logical px). Both
/// generous enough for a trackpad, tight enough that a deliberate second click
/// somewhere else in the same field still just moves the caret.
const MULTI_CLICK_GAP: std::time::Duration = std::time::Duration::from_millis(400);
const MULTI_CLICK_SLOP: f32 = 4.0;

/// How long the pointer must rest on one widget before its tooltip shows —
/// long enough that sweeping across a key bank flashes nothing, short enough
/// that a genuine "what is this" pause is answered. Read against the same
/// clock as the multi-click streak, so a host driving time by hand
/// ([`Ui::set_now`]) controls tooltips with it.
const TOOLTIP_DELAY: Duration = Duration::from_millis(500);

/// Whether Tab / Shift-Tab may *enter* the focus cycle from nothing, or only
/// advance focus that a press or [`Ui::focus`] already placed — the
/// tree-level analogue of [`PageKeys`](crate::scroll::PageKeys)'s
/// widget-level rule. Set via [`Ui::set_tab_entry`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabEntry {
    /// Tab with nothing focused seeds the cycle — the first focusable widget
    /// (Shift-Tab: the last). The standalone-window rule, and the default:
    /// the tree owns the keyboard, so Tab is how a user reaches its fields
    /// without a mouse.
    #[default]
    Seed,
    /// Tab with nothing focused **falls through to the host**
    /// ([`Response::keyboard`] stays false; focus stays NONE). The
    /// embedded-chrome rule: a host compositing this tree over its own
    /// content keeps Tab's content meaning (an editor's indent) until the
    /// user is actually *in* the chrome — by a click or [`Ui::focus`] — and
    /// gets it back the moment the chrome blurs. While a widget holds focus,
    /// Tab cycles exactly as under [`Seed`](Self::Seed).
    WhileFocused,
}

/// The retained UI: a widget tree plus interaction state.
pub struct Ui {
    root: Box<dyn Widget>,
    hovered: WidgetId,
    focused: WidgetId,
    captured: WidgetId,
    /// The pointer button that initiated the current capture — only its release
    /// ends the capture, so a stray other-button release can't drop a live drag.
    capture_button: PointerButton,
    pointer: Vec2,
    modifiers: Modifiers,
    fired: Vec<WidgetId>,
    actions: Vec<u64>,
    /// Text a widget copied/cut during the last dispatch, for the host to
    /// write to the OS clipboard (see [`take_clipboard`](Self::take_clipboard)).
    clipboard: Option<String>,
    blocking: bool,
    /// Logical→physical scale, passed to the layout/draw contexts so scale-aware
    /// widgets can align to the device pixel grid.
    scale: f32,
    /// The open popup's owner, if any (one popup at a time; while open, all
    /// pointer events target the owner, and the grab is reported to the host
    /// as [`Response::capturing`] so a multi-`Ui` host routes outside presses
    /// here). Cleared with the owner on window focus loss, and when the owner
    /// loses the keyboard ([`set_focus`](Self::set_focus)).
    popup: Option<WidgetId>,
    /// Where and when the last primary press landed, and how long the streak of
    /// rapid presses at that spot is — 1 for a single click, 2 for a double, 3
    /// for a triple, and on up. Time is the only way to tell a double click from
    /// two deliberate ones, so this is the one place the toolkit reads a clock;
    /// widgets see the answer as [`EventCtx::clicks`] and stay deterministic.
    /// Timestamps count from `clock_base` — unless the host has taken the
    /// clock over ([`set_now`](Self::set_now)), then they are its own.
    last_click: Option<(Vec2, Duration)>,
    click_streak: u8,
    /// Which widget the pointer has been resting on and since when — what
    /// arms a tooltip. Re-stamped whenever hover moves to a different id, and
    /// on every primary press (a click dismisses the tip until the pointer
    /// rests again).
    hover_mark: (WidgetId, Duration),
    /// Whether the due tooltip's frame has been pumped — see
    /// [`take_dirty`](Self::take_dirty).
    tooltip_settled: bool,
    /// Epoch for the default (ambient) multi-click clock.
    clock_base: Instant,
    /// The host-driven time, once [`set_now`](Self::set_now) has been called;
    /// `None` reads the ambient clock.
    manual_now: Option<Duration>,
    /// Whether Tab may seed focus from nothing, or only advance it — see
    /// [`TabEntry`] and [`set_tab_entry`](Self::set_tab_entry).
    tab_entry: TabEntry,
    /// The surface popups must stay inside; empty (the default) is
    /// unconstrained. See [`set_viewport`](Ui::set_viewport).
    viewport: Rect,
    /// True when something since the last [`take_dirty`](Self::take_dirty)
    /// could have changed pixels: a dispatched event, a focus move, a
    /// state-changing setter, a `&mut` handed out to a widget. Starts true —
    /// a fresh tree has never been drawn.
    dirty: bool,
}

impl Ui {
    pub fn new(root: impl Widget + 'static) -> Self {
        Self {
            root: Box::new(root),
            hovered: WidgetId::NONE,
            focused: WidgetId::NONE,
            captured: WidgetId::NONE,
            capture_button: PointerButton::Primary,
            pointer: Vec2::ZERO,
            modifiers: Modifiers::NONE,
            fired: Vec::new(),
            actions: Vec::new(),
            clipboard: None,
            blocking: false,
            scale: 1.0,
            last_click: None,
            click_streak: 0,
            hover_mark: (WidgetId::NONE, Duration::ZERO),
            tooltip_settled: false,
            clock_base: Instant::now(),
            manual_now: None,
            popup: None,
            tab_entry: TabEntry::Seed,
            viewport: Rect::ZERO,
            dirty: true,
        }
    }

    /// Sets when Tab / Shift-Tab may take focus — see [`TabEntry`]. An
    /// embedded host ([`TabEntry::WhileFocused`]) keeps Tab's content meaning
    /// while none of this tree's widgets hold the keyboard.
    pub fn set_tab_entry(&mut self, entry: TabEntry) {
        self.tab_entry = entry;
    }

    /// Names the surface **popups** must stay inside (logical px) — the window,
    /// which for a `Ui` hosted in a sub-region (a docked panel) is emphatically
    /// not the rect it lays out into. A [`Select`](crate::Select) uses it to flip
    /// its list above the box rather than off the bottom edge, and to shift it
    /// clear of the left/right edges; that difference is exactly what every
    /// hand-rolled dropdown host used to encode for itself, each one differently.
    ///
    /// Unset (the default) is **unconstrained**: a popup drops straight down and
    /// nothing clamps it, which is what a full-screen host wants anyway.
    /// Reaches widgets as [`LayoutCtx::viewport`].
    pub fn set_viewport(&mut self, viewport: Rect) {
        if self.viewport != viewport {
            self.viewport = viewport;
            self.dirty = true;
        }
    }

    /// Sets the logical→physical scale used during layout and draw (e.g. the
    /// window's DPI factor, matching the renderer's). Scale-aware widgets use it
    /// to land on whole device pixels — notably a floating [`Window`], which
    /// snaps its position/size so it drags in rigid pixel steps. Default `1.0`.
    pub fn set_scale(&mut self, scale: f32) {
        let scale = scale.max(1e-4);
        if self.scale != scale {
            self.scale = scale;
            self.dirty = true;
        }
    }

    /// Whether a frame is owed, read-and-cleared: true when something since
    /// the last call could have changed pixels — a dispatched event, a focus
    /// move, a state-changing setter, a `&mut` widget borrow
    /// ([`get_mut`](Self::get_mut)/[`root_mut`](Self::root_mut)), or a
    /// [`mark_dirty`](Self::mark_dirty). A clean tree lets the host skip
    /// layout **and** draw, not just the present; the precise inner filter
    /// (events flowed but the pixels came out identical) stays
    /// [`IdleGate`](crate::draw::IdleGate), which compares the drawn list.
    /// If the frame then fails to present, call `mark_dirty` so the retry
    /// isn't short-circuited away.
    pub fn take_dirty(&mut self) -> bool {
        // A tooltip arms by *time passing*, which no event marks dirty: while a
        // hover rests on a widget that carries one, keep frames coming —
        // through the one frame where the tip first comes due (which draws it)
        // — then go quiet. A hover over tooltip-less chrome pumps nothing.
        let carries = self.tooltip_resting()
            && find(self.root.as_ref(), self.hovered).is_some_and(|w| w.tooltip().is_some());
        let due = carries && self.tooltip_due();
        let pump = carries && (!due || !self.tooltip_settled);
        self.tooltip_settled = due;
        std::mem::take(&mut self.dirty) || pump
    }

    /// Marks the tree as needing a frame for a change the toolkit cannot see:
    /// the target was resized, a registered host texture's pixels moved, the
    /// theme object changed behind its `&dyn`. The counterpart of
    /// [`take_dirty`](Self::take_dirty).
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Hands the multi-click clock to the host. `now` is the host's own
    /// monotonic timestamp (from any epoch it likes); every primary press from
    /// here on is stamped with the latest value given — call it whenever your
    /// time advances, before dispatching. Never calling it keeps the default,
    /// the ambient system clock, which is what a live window wants. Driving it
    /// by hand is how a replayed input script expresses an *intended* double
    /// click deterministically: real elapsed time between two replayed presses
    /// races `MULTI_CLICK_GAP` (a paused process turns a double into two
    /// singles), a scripted timestamp cannot. Time handed backwards starts a
    /// fresh streak rather than extending one.
    pub fn set_now(&mut self, now: Duration) {
        self.manual_now = Some(now);
    }

    /// True while a dropdown/menu popup is open.
    pub fn popup_open(&self) -> bool {
        self.popup.is_some()
    }

    /// The mouse cursor the UI wants at the current pointer position: asks the
    /// capturing widget mid-drag (so a resize keeps its arrows even when the
    /// pointer outruns the grip), the hovered widget otherwise. The host
    /// applies it to the OS window after dispatching pointer events (the
    /// `winit` feature maps it via [`crate::winit::map_cursor`]).
    pub fn cursor_icon(&self) -> CursorIcon {
        let id = if self.captured != WidgetId::NONE {
            self.captured
        } else {
            self.hovered
        };
        if id == WidgetId::NONE {
            return CursorIcon::Default;
        }
        find(self.root.as_ref(), id)
            .map(|w| w.cursor(self.pointer))
            .unwrap_or_default()
    }

    pub fn root(&self) -> &dyn Widget {
        self.root.as_ref()
    }

    pub fn root_mut(&mut self) -> &mut dyn Widget {
        // A `&mut` tree is presumed mutated — over-marking costs one gated
        // frame, under-marking costs stale pixels.
        self.dirty = true;
        self.root.as_mut()
    }

    pub fn hovered(&self) -> WidgetId {
        self.hovered
    }

    /// The last pointer position this `Ui` saw (logical px) — the coordinate
    /// hover was resolved against. For a host deriving pointer-anchored chrome
    /// (a hover hint's row bands, a popover anchor) right after
    /// [`dispatch`](Self::dispatch), instead of shadowing the event stream it
    /// already handed over to track the same number.
    pub fn pointer(&self) -> Vec2 {
        self.pointer
    }

    pub fn focused(&self) -> WidgetId {
        self.focused
    }

    /// Drops keyboard focus, so no widget holds it, telling the widget that had
    /// it **why** — a field commits on [`BlurCause::Moved`] and discards on
    /// [`BlurCause::Cancelled`].
    ///
    /// The host's counterpart to [`focus_first`](Self::focus_first), for the two
    /// things only a host can decide. **Escape leaves a field:** `TextInput` does
    /// not handle Escape — a single-line field has no editing meaning for it, and
    /// what Escape *should* do (blur, close a dialog, cancel a tool) belongs to
    /// the host, so the host blurs (`Cancelled`) and then runs its own cascade.
    /// **One keyboard owner:** a host running several `Ui`s (one per panel) must
    /// blur the one losing focus when a press moves it (`Moved`), or two trees
    /// each believe they hold the keyboard.
    pub fn blur(&mut self, cause: BlurCause) {
        self.set_focus(WidgetId::NONE, cause);
    }

    /// Moves keyboard focus to the first focusable widget (in tree order) — e.g.
    /// to focus a form's first field when a dialog opens. No-op if none accept
    /// focus.
    pub fn focus_first(&mut self) {
        let mut order = Vec::new();
        collect_focusables(self.root.as_ref(), &mut order);
        self.set_focus(
            order.first().copied().unwrap_or(WidgetId::NONE),
            BlurCause::Moved,
        );
    }

    /// Moves keyboard focus to `id` — [`focus_first`](Self::focus_first)'s
    /// targeted sibling, for the host shortcut that lands *in* a specific
    /// field (Ctrl+F refocusing a find bar that is already open). Whoever held
    /// focus is told [`BlurCause::Moved`] (a field commits), exactly as if the
    /// user had clicked into the target.
    ///
    /// Only a widget that is in the tree **and accepts focus** can take it:
    /// anything else leaves focus where it was and returns `false` — handing
    /// the keyboard to a widget that never asked for it would strand every
    /// key event on a widget with no keyboard handling. To *drop* focus, call
    /// [`blur`](Self::blur) (passing [`WidgetId::NONE`] here is a refusal,
    /// not a blur).
    pub fn focus(&mut self, id: WidgetId) -> bool {
        let mut order = Vec::new();
        collect_focusables(self.root.as_ref(), &mut order);
        if !order.contains(&id) {
            return false;
        }
        self.set_focus(id, BlurCause::Moved);
        true
    }

    /// Advances focus to the next (or previous, if `backward`) focusable widget,
    /// wrapping around. Returns `false` when nothing accepts focus (so Tab can
    /// fall through to the host).
    fn focus_step(&mut self, backward: bool) -> bool {
        let mut order = Vec::new();
        collect_focusables(self.root.as_ref(), &mut order);
        let Some(&first) = order.first() else {
            return false;
        };
        let n = order.len();
        let next = match order.iter().position(|&id| id == self.focused) {
            Some(i) if backward => order[(i + n - 1) % n],
            Some(i) => order[(i + 1) % n],
            None if backward => order[n - 1],
            None => first,
        };
        self.set_focus(next, BlurCause::Moved);
        true
    }

    /// Moves keyboard focus to `id`, delivering [`Event::Blur`] to whoever held
    /// it. **The one place `self.focused` is written**, so no path can move focus
    /// without the outgoing widget hearing about it — which is the whole of G2:
    /// focus moves as a side effect (a click elsewhere, Tab, a host handing the
    /// keyboard to another tree), never as an event a widget could watch for
    /// itself.
    fn set_focus(&mut self, id: WidgetId, cause: BlurCause) {
        let lost = self.focused;
        self.focused = id;
        if lost != id {
            // Focus rings and caret visibility are pixels.
            self.dirty = true;
        }
        if lost == WidgetId::NONE || lost == id {
            return;
        }
        let mut resp = Response::default();
        let (mut capture_request, mut focus_request, mut popup_request) = (None, None, None);
        let mut ctx = EventCtx {
            modifiers: self.modifiers,
            pointer: self.pointer,
            target: lost,
            hovered: self.hovered,
            // The widget is told it *had* focus: `is_focused` still answers for
            // it, so a blur handler reads the same pre-event focus state that a
            // press-driven one does.
            focused: lost,
            // A blur is not a press: no widget may read a click streak off it.
            clicks: 0,
            response: &mut resp,
            fired: &mut self.fired,
            actions: &mut self.actions,
            capture_request: &mut capture_request,
            focus_request: &mut focus_request,
            popup_request: &mut popup_request,
            popup_owner: self.popup.unwrap_or(WidgetId::NONE),
            clipboard: &mut self.clipboard,
        };
        // A blur consumes nothing and grants nothing: the response is dropped and
        // the three requests are ignored, so a widget cannot take focus (or the
        // pointer, or the popup) back on its way out.
        self.root.event(&Event::Blur(cause), &mut ctx);
        // A popup does not survive its owner losing the keyboard: an open list
        // with no focus would eat every pointer event while arrows and Escape
        // fall through to whatever got the keyboard instead. The owner drops its
        // own open flag in its `Blur` arm (its `close_popup` request was ignored
        // above, like every request on the way out); the routing state is the
        // `Ui`'s to drop, here.
        if self.popup == Some(lost) {
            self.popup = None;
            self.hovered = self.root.hit_test(self.pointer).unwrap_or(WidgetId::NONE);
        }
        self.note_hover();
    }

    /// Marks a modal/blocking layer open (the host should withhold all world
    /// input while set — reflected in [`Response::blocking`]).
    pub fn set_blocking(&mut self, blocking: bool) {
        self.blocking = blocking;
    }

    /// True if `id` fired (clicked) during the last [`dispatch`](Self::dispatch).
    pub fn fired(&self, id: WidgetId) -> bool {
        id != WidgetId::NONE && self.fired.contains(&id)
    }

    /// Action tags emitted during the last [`dispatch`](Self::dispatch) (the
    /// bridge to a host command/undo/replay layer).
    pub fn actions(&self) -> &[u64] {
        &self.actions
    }

    /// Text a widget copied/cut during the last [`dispatch`](Self::dispatch) —
    /// the host writes it to the OS clipboard (the toolkit itself has no
    /// clipboard access). The paste half is [`Event::Paste`]: the host reads
    /// the OS clipboard on its paste chord and dispatches that.
    pub fn take_clipboard(&mut self) -> Option<String> {
        self.clipboard.take()
    }

    /// True while the focused widget consumes committed text (a text field) —
    /// the host mirrors this into the OS IME (`window.set_ime_allowed`) and
    /// then feeds [`Event::ImePreedit`] / [`Event::Text`] back in.
    pub fn wants_text_input(&self) -> bool {
        find(self.root.as_ref(), self.focused).is_some_and(|w| w.accepts_text_input())
    }

    /// The focused text field's caret rectangle (logical px), anchoring the
    /// host's IME candidate window (`window.set_ime_cursor_area`).
    pub fn ime_rect(&self) -> Option<Rect> {
        find(self.root.as_ref(), self.focused).and_then(|w| w.ime_rect())
    }

    /// Borrows the widget with `id` as concrete type `T`, if present — the
    /// typed way to read a control's state (e.g. `ui.get::<Checkbox>(id)`).
    pub fn get<T: Widget>(&self, id: WidgetId) -> Option<&T> {
        descendant(self.root.as_ref(), id)
    }

    /// Mutably borrows the widget with `id` as `T`, if present (to set state
    /// programmatically). Presumed mutated: marks the tree as needing a frame
    /// (see [`root_mut`](Self::root_mut)).
    pub fn get_mut<T: Widget>(&mut self, id: WidgetId) -> Option<&mut T> {
        self.dirty = true;
        descendant_mut(self.root.as_mut(), id)
    }

    /// The last-layout rectangle of the widget with `id`, if present — the
    /// type-erased lookup for hosts that hit-test or anchor against a control
    /// (hover hints, popovers) without knowing its concrete type.
    pub fn rect_of(&self, id: WidgetId) -> Option<Rect> {
        find(self.root.as_ref(), id).map(|w| w.rect())
    }

    /// The tree as a [`semantics::outline`](crate::semantics::outline), with
    /// this `Ui`'s hovered/focused ids filled into the state flags (they live
    /// here, not in the tree — a bare-tree outline can't show them).
    #[must_use]
    pub fn outline(&self, opts: crate::semantics::Outline) -> String {
        let opts = crate::semantics::Outline {
            hovered: self.hovered,
            focused: self.focused,
            ..opts
        };
        crate::semantics::outline(self.root.as_ref(), &opts)
    }

    /// Lays out the tree to fill `viewport` (logical px) from the origin.
    pub fn layout(&mut self, viewport: Size, theme: &dyn Theme, fonts: &Fonts) {
        self.layout_in(Rect::from_min_size(Vec2::ZERO, viewport), theme, fonts);
    }

    /// Lays out the tree into `rect` (logical px) — like [`layout`](Self::layout)
    /// but positioned at `rect`'s origin instead of `(0, 0)`, for hosting a `Ui`
    /// inside a sub-region of a larger surface (a docked panel, a status strip).
    pub fn layout_in(&mut self, rect: Rect, theme: &dyn Theme, fonts: &Fonts) {
        let mut ctx = LayoutCtx {
            fonts,
            theme,
            scale: self.scale,
            viewport: self.viewport,
        };
        self.root.measure(rect.size(), &mut ctx);
        self.root.arrange(rect, &mut ctx);
    }

    /// Emits the tree's draw commands into `dl`. Draws the base pass, then the
    /// overlay pass (popups/menus) on top.
    pub fn draw(&self, dl: &mut DrawList, theme: &dyn Theme, fonts: &Fonts) {
        for pass in [DrawPass::Base, DrawPass::Overlay] {
            self.draw_pass(dl, theme, fonts, pass);
        }
    }

    /// Emits **one** pass's draw commands into `dl` — for a host that composites
    /// the two into different layers. A `Ui` hosted in a sub-region (a docked
    /// panel) is one of several drawn in z-order, so its base chrome belongs at
    /// its own depth while its popups have to land above *every* sibling: draw
    /// [`DrawPass::Base`] into that panel's list and [`DrawPass::Overlay`] into
    /// a shared one composited last. [`draw`](Self::draw) is this twice, in
    /// order, for the single-layer case.
    pub fn draw_pass(&self, dl: &mut DrawList, theme: &dyn Theme, fonts: &Fonts, pass: DrawPass) {
        let ctx = DrawCtx {
            fonts,
            theme,
            scale: self.scale,
            hovered: self.hovered,
            focused: self.focused,
            pass,
        };
        self.root.draw(dl, &ctx);
        // The hovered widget's tooltip, painted centrally at the end of the
        // overlay pass — above every popup a sibling drew into this list, and
        // without any widget keeping a clock: the `Ui` owns the hover and the
        // time, the widget only *carries* the text (`Widget::tooltip`).
        if pass == DrawPass::Overlay
            && self.tooltip_due()
            && let Some(w) = find(self.root.as_ref(), self.hovered)
            && let Some(tip) = w.tooltip()
        {
            theme.tooltip(dl, fonts, w.rect(), self.viewport, tip);
        }
    }

    /// Routes `events` into the tree and returns what the UI consumed. Hit-tests
    /// against the most recent [`layout`](Self::layout).
    pub fn dispatch(&mut self, events: &[Event]) -> Response {
        self.fired.clear();
        self.actions.clear();
        self.clipboard = None;
        // Any event can move widget state (even a bare pointer move changes
        // hover); an empty dispatch cannot. The draw-list gate downstream is
        // the precise filter — this flag only has to never under-mark.
        if !events.is_empty() {
            self.dirty = true;
        }
        let mut resp = Response::default();

        for ev in events {
            if let Some(pos) = ev.pos() {
                self.pointer = pos;
            }
            if matches!(ev, Event::PointerLeft) {
                self.hovered = WidgetId::NONE;
            }
            // Refresh hover on move/press while no drag is in progress. While a
            // popup is open, hover collapses to its owner or nothing: the popup
            // occludes an unknown region (the Ui tracks no popup geometry), and
            // no other widget can receive pointer events anyway — hit-testing
            // through the popup would paint hover on widgets underneath it.
            if self.captured == WidgetId::NONE
                && matches!(ev, Event::PointerMoved { .. } | Event::PointerButton { .. })
            {
                let hit = self.root.hit_test(self.pointer).unwrap_or(WidgetId::NONE);
                self.hovered = match self.popup {
                    Some(owner) if hit != owner => WidgetId::NONE,
                    _ => hit,
                };
            }

            self.modifiers = match ev {
                Event::PointerButton { mods, .. }
                | Event::Scroll { mods, .. }
                | Event::Key { mods, .. } => *mods,
                _ => self.modifiers,
            };

            // Tab / Shift-Tab cycle keyboard focus through the focusable widgets
            // (text fields, dropdowns) in tree order, before the event reaches any
            // widget — so a focused field never inserts a tab. Inert while a popup
            // is open: moving focus away would strand the popup (Escape could no
            // longer reach its owner while it still grabs all pointer input).
            // Under `TabEntry::WhileFocused`, also inert while nothing is
            // focused — the host keeps Tab until the user is in the chrome.
            if let Event::Key {
                key: Key::Tab,
                pressed: true,
                mods,
                ..
            } = ev
                && self.popup.is_none()
                && (self.tab_entry == TabEntry::Seed || self.focused != WidgetId::NONE)
                && self.focus_step(mods.shift)
            {
                resp.keyboard = true;
                continue;
            }

            // While a popup is open, all pointer events target its owner (so the
            // owner handles its options and dismisses on an outside click).
            // Otherwise pointer events target the capturing widget (mid-drag) or
            // the topmost hit; keyboard/text events target the focused widget.
            let popup_owner = self.popup.unwrap_or(WidgetId::NONE);
            let target = if ev.is_pointer() {
                if self.captured != WidgetId::NONE {
                    self.captured
                } else if popup_owner != WidgetId::NONE {
                    popup_owner
                } else {
                    self.hovered
                }
            } else {
                self.focused
            };

            // How long a streak of rapid presses this one completes: a second
            // press near the first within `MULTI_CLICK_GAP` is a double click, a
            // third a triple. Resolved here, once, so every widget agrees on what
            // a double click is and none of them has to keep its own clock.
            let clicks = match ev {
                Event::PointerButton {
                    button: PointerButton::Primary,
                    pressed: true,
                    pos,
                    ..
                } => {
                    let now = self.now();
                    // A press dismisses a resting tooltip: the rest starts over.
                    self.hover_mark.1 = now;
                    let near = |at: Vec2| {
                        (pos.x - at.x).abs() <= MULTI_CLICK_SLOP
                            && (pos.y - at.y).abs() <= MULTI_CLICK_SLOP
                    };
                    let within_gap =
                        |t: Duration| now.checked_sub(t).is_some_and(|gap| gap <= MULTI_CLICK_GAP);
                    self.click_streak = match self.last_click {
                        Some((at, t)) if near(at) && within_gap(t) => {
                            self.click_streak.saturating_add(1)
                        }
                        _ => 1,
                    };
                    self.last_click = Some((*pos, now));
                    self.click_streak
                }
                _ => 0,
            };

            let mut capture_request = None;
            let mut focus_request = None;
            let mut popup_request = None;
            {
                let mut ctx = EventCtx {
                    modifiers: self.modifiers,
                    pointer: self.pointer,
                    clicks,
                    target,
                    hovered: self.hovered,
                    focused: self.focused,
                    response: &mut resp,
                    fired: &mut self.fired,
                    actions: &mut self.actions,
                    capture_request: &mut capture_request,
                    focus_request: &mut focus_request,
                    popup_request: &mut popup_request,
                    popup_owner,
                    clipboard: &mut self.clipboard,
                };
                self.root.event(ev, &mut ctx);
            }
            if let Some(id) = focus_request {
                self.set_focus(id, BlurCause::Moved);
            }
            if let Some(id) = capture_request {
                self.captured = id;
                // Remember which button started the capture (a capture taken
                // outside a press — e.g. from a move — behaves as primary).
                self.capture_button = match ev {
                    Event::PointerButton {
                        button,
                        pressed: true,
                        ..
                    } => *button,
                    _ => PointerButton::Primary,
                };
            }
            match popup_request {
                // Opening occludes whatever was hovered (unless that is the
                // owner itself); closing re-exposes it — recompute right away
                // rather than leave stale hover until the next pointer move.
                Some(PopupOp::Open(id)) => {
                    self.popup = Some(id);
                    if self.hovered != id {
                        self.hovered = WidgetId::NONE;
                    }
                }
                Some(PopupOp::Close) => {
                    self.popup = None;
                    self.hovered = self.root.hit_test(self.pointer).unwrap_or(WidgetId::NONE);
                }
                None => {}
            }
            // Releasing the button that initiated the capture ends it (after the
            // captured widget has handled the release above); a stray release of
            // any *other* button must not drop a live drag.
            if let Event::PointerButton {
                button,
                pressed: false,
                ..
            } = ev
                && *button == self.capture_button
            {
                self.captured = WidgetId::NONE;
            }
            // Window focus lost: hover is stale, any capture's release will
            // never arrive, and an open popup's dismissing click may land in
            // another window entirely (widgets disarm themselves on the same
            // event — a popup owner drops its open flag exactly as a dragging
            // widget drops its `pressed`).
            if matches!(ev, Event::Focus(false)) {
                self.hovered = WidgetId::NONE;
                self.captured = WidgetId::NONE;
                self.popup = None;
            }
        }

        // An open popup is a pointer grab, exactly like a drag: all pointer
        // input belongs to this tree until the owner dismisses. Reporting it
        // here is what makes a *multi-`Ui`* host route an outside press into
        // the owner's tree (where the popup machinery dismisses it) instead of
        // to whatever lies under the press — a host that honors `capturing`
        // gets popup press-modality without knowing popups exist.
        resp.capturing = self.captured != WidgetId::NONE || self.popup.is_some();
        // OR in the Ui-level flag so a widget that called `ctx.block()` this
        // dispatch (an open `Modal`) keeps its blocking request.
        resp.blocking |= self.blocking;
        self.note_hover();
        resp
    }

    /// The toolkit's clock: the host-driven time once [`set_now`](Self::set_now)
    /// has been called, the ambient monotonic clock otherwise. Multi-click
    /// streaks and tooltip rests both read it, so a scripted host controls both.
    fn now(&self) -> Duration {
        self.manual_now.unwrap_or_else(|| self.clock_base.elapsed())
    }

    /// Re-stamps the tooltip rest mark if hover has moved to a different
    /// widget — called wherever a batch of hover changes settles (the end of a
    /// dispatch, a blur that re-resolved hover).
    fn note_hover(&mut self) {
        if self.hover_mark.0 != self.hovered {
            self.hover_mark = (self.hovered, self.now());
        }
    }

    /// Whether a hover is in a state a tooltip could ever arm from: the pointer
    /// is on a widget, with no drag in progress and no popup open (a popup
    /// occludes unknown geometry, and its owner holds every pointer event
    /// anyway).
    fn tooltip_resting(&self) -> bool {
        self.hovered != WidgetId::NONE
            && self.hover_mark.0 == self.hovered
            && self.captured == WidgetId::NONE
            && self.popup.is_none()
    }

    /// True while a tooltip-carrying hover is **arming** — resting toward its
    /// delay but not yet due. The frame-scheduling half of the tooltip
    /// handshake for a host that drives redraws itself instead of polling
    /// [`take_dirty`](Self::take_dirty) (an editor that syncs widget state
    /// every frame keeps the dirty flag permanently set, which would turn it
    /// into a spin): while this is true, schedule another frame — each armed
    /// frame requests the next, and the first *due* frame, requested by the
    /// last arming one, is the frame that draws the tip.
    pub fn tooltip_arming(&self) -> bool {
        self.tooltip_resting()
            && !self.tooltip_due()
            && find(self.root.as_ref(), self.hovered).is_some_and(|w| w.tooltip().is_some())
    }

    /// The tooltip text the widget under the pointer carries, the moment hover
    /// lands — no rest delay. The floating plate waits for
    /// [`TOOLTIP_DELAY`]; a host that *also* mirrors hints somewhere always
    /// visible (a status bar) wants the words immediately, and to drop them
    /// the moment hover leaves. Gated like the plate on there being no drag
    /// in progress and no popup open, so a held stroke or an open list never
    /// narrates the chrome under it.
    pub fn hovered_tooltip(&self) -> Option<&str> {
        if !self.tooltip_resting() {
            return None;
        }
        find(self.root.as_ref(), self.hovered).and_then(|w| w.tooltip())
    }

    /// Whether the hovered widget's tooltip is due: the rest has lasted the
    /// delay. The widget half of the handshake is
    /// [`Widget::tooltip`](crate::widget::Widget::tooltip).
    fn tooltip_due(&self) -> bool {
        self.tooltip_resting()
            && self
                .now()
                .checked_sub(self.hover_mark.1)
                .is_some_and(|rest| rest >= TOOLTIP_DELAY)
    }
}

/// Collects the ids of focusable widgets in depth-first (tree) order — the Tab
/// traversal order.
fn collect_focusables(w: &dyn Widget, out: &mut Vec<WidgetId>) {
    if w.accepts_focus() && w.id() != WidgetId::NONE {
        out.push(w.id());
    }
    for i in 0..w.child_count() {
        if let Some(c) = w.child(i) {
            collect_focusables(c, out);
        }
    }
}

/// Borrows the widget with `id` from **`w`'s own subtree** as `T` (`w` itself
/// matches if its id is `id`). The tree-level form of [`Ui::get`], which is what
/// a **composite widget** needs: it owns its children but has no `Ui` to ask, so
/// without this it would hand-roll the same depth-first walk plus an `Any`
/// downcast — once per composite.
pub fn descendant<T: Widget>(w: &dyn Widget, id: WidgetId) -> Option<&T> {
    let found = find(w, id)?;
    let any: &dyn Any = found;
    any.downcast_ref::<T>()
}

/// Mutably borrows the widget with `id` from `w`'s own subtree as `T` — see
/// [`descendant`]. This is how a composite pushes host state into its retained
/// children (an editor panel lighting the active tool's key each frame).
pub fn descendant_mut<T: Widget>(w: &mut dyn Widget, id: WidgetId) -> Option<&mut T> {
    let path = find_path(&*w, id)?;
    let found = walk_mut(w, &path)?;
    let any: &mut dyn Any = found;
    any.downcast_mut::<T>()
}

/// Depth-first search for the widget with `id`.
fn find(w: &dyn Widget, id: WidgetId) -> Option<&dyn Widget> {
    if id != WidgetId::NONE && w.id() == id {
        return Some(w);
    }
    for i in 0..w.child_count() {
        if let Some(found) = w.child(i).and_then(|c| find(c, id)) {
            return Some(found);
        }
    }
    None
}

/// The child-index path from `w` to the widget with `id` (empty = `w` itself).
fn find_path(w: &dyn Widget, id: WidgetId) -> Option<Vec<usize>> {
    if id != WidgetId::NONE && w.id() == id {
        return Some(Vec::new());
    }
    for i in 0..w.child_count() {
        if let Some(mut path) = w.child(i).and_then(|c| find_path(c, id)) {
            path.insert(0, i);
            return Some(path);
        }
    }
    None
}

/// Descends `path` via `child_mut` (sequential reborrow — borrow-checker safe).
fn walk_mut<'a>(mut w: &'a mut dyn Widget, path: &[usize]) -> Option<&'a mut dyn Widget> {
    for &i in path {
        w = w.child_mut(i)?;
    }
    Some(w)
}
