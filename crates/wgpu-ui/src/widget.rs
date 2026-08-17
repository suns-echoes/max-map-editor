//! The retained `Widget` trait and the layout vocabulary.
//!
//! Layout is two-pass (measure → arrange), Flutter/Compose-style: a widget first
//! reports its desired size for some available space, then is assigned a final
//! rectangle and positions its children. Everything is in **logical UI pixels**;
//! the library converts to physical pixels at the render boundary.
//!
//! Behavior (event handling) lives on the widget; visuals go through the
//! [`crate::theme`] (added in a later step). This file defines the trait and the
//! contexts; concrete containers/leaves live in [`crate::layout`].

use std::any::Any;

use crate::draw::DrawList;
use crate::event::{Event, Modifiers};
use crate::geom::{Rect, Size, Vec2};
use crate::interact::{CursorIcon, Response, WidgetId};
use crate::text::Fonts;
use crate::theme::Theme;

/// Context for the layout passes (`measure`/`arrange`).
pub struct LayoutCtx<'a> {
    /// Fonts, for measuring text.
    pub fonts: &'a Fonts,
    /// The theme, for metrics and font selection during sizing.
    pub theme: &'a dyn Theme,
    /// The UI scale in effect (logical→physical factor), for any scale-aware
    /// metrics. Layout itself stays in logical pixels.
    pub scale: f32,
    /// The surface a **popup** must stay inside (logical px) — the window, not
    /// necessarily the rect this tree is laid out into. A [`Select`] uses it to
    /// decide whether its option list drops below or flips above, and to shift
    /// the list clear of the left/right edges; a `Ui` laid out into a sub-region
    /// (a docked panel) is told the whole window through
    /// [`Ui::set_viewport`](crate::Ui::set_viewport), so a dropdown near the
    /// panel's bottom edge is not cropped by it.
    ///
    /// **Empty (the default) means unconstrained** — nothing clamps, and a popup
    /// simply drops down. A host that lays out into the whole window needs no
    /// viewport; one that hosts a `Ui` in a sub-region names it.
    ///
    /// [`Select`]: crate::Select
    pub viewport: Rect,
}

/// Which draw pass is running. The `Ui` walks the whole tree twice: `Base` for
/// normal content, then `Overlay` (composited on top) for popups/menus/dropdowns.
///
/// **The z-stacking rule:** a widget must paint its normal content **only in the
/// `Base` pass**, and emit content in the `Overlay` pass *only* for things that
/// must float above everything (an open dropdown, a menu cascade, a drag
/// indicator). Otherwise a leaf drawn later in the tree would repaint over an
/// earlier widget's popup in the overlay pass. Leaves typically start their draw
/// with `if !ctx.is_base() { return; }`; containers forward `draw` to children in
/// both passes (so a child popup can reach the overlay) but gate their own chrome
/// behind [`DrawCtx::is_base`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawPass {
    Base,
    Overlay,
}

/// Context for the `draw` pass.
pub struct DrawCtx<'a> {
    pub fonts: &'a Fonts,
    pub theme: &'a dyn Theme,
    pub scale: f32,
    /// The currently hovered widget (so widgets paint the right state).
    pub hovered: WidgetId,
    /// The currently focused widget.
    pub focused: WidgetId,
    /// The pass currently running.
    pub pass: DrawPass,
}

impl DrawCtx<'_> {
    pub fn is_hovered(&self, id: WidgetId) -> bool {
        id != WidgetId::NONE && self.hovered == id
    }

    pub fn is_focused(&self, id: WidgetId) -> bool {
        id != WidgetId::NONE && self.focused == id
    }

    /// True during the overlay pass (draw popups here, on top of everything).
    pub fn is_overlay(&self) -> bool {
        self.pass == DrawPass::Overlay
    }

    pub fn is_base(&self) -> bool {
        self.pass == DrawPass::Base
    }
}

/// A request from a widget to open or close a top-level popup (dropdown list,
/// menu, context menu). The `Ui` routes pointer input to the popup owner while a
/// popup is open and dismisses it on an outside click.
pub(crate) enum PopupOp {
    Open(WidgetId),
    Close,
}

/// Context for the `event` pass. The `Ui` resolves the topmost hit target,
/// hover, and focus centrally and exposes them here; widgets self-select by id.
/// Outcomes flow back out (consumed flags, fired widgets, action tags, capture/
/// focus requests) for the host to poll.
pub struct EventCtx<'a> {
    pub modifiers: Modifiers,
    pub pointer: Vec2,
    /// How long a streak of rapid presses **this** event completes: 1 for a
    /// single click, 2 for a double, 3 for a triple, and on up. `0` on anything
    /// that is not a primary press, so a widget can match on it without first
    /// checking the event.
    ///
    /// The [`Ui`](crate::ui::Ui) resolves it from the press positions and a
    /// clock, once per event — the toolkit's only reading of time, kept there
    /// so every widget agrees on what a double click is instead of each one
    /// timing presses for itself. A text field selects the word under a double
    /// click and the whole line under a triple; anything else is free to ignore
    /// it and treat every press as a press.
    pub clicks: u8,
    pub(crate) target: WidgetId,
    pub(crate) hovered: WidgetId,
    pub(crate) focused: WidgetId,
    pub(crate) response: &'a mut Response,
    pub(crate) fired: &'a mut Vec<WidgetId>,
    pub(crate) actions: &'a mut Vec<u64>,
    pub(crate) capture_request: &'a mut Option<WidgetId>,
    pub(crate) focus_request: &'a mut Option<WidgetId>,
    pub(crate) popup_request: &'a mut Option<PopupOp>,
    pub(crate) popup_owner: WidgetId,
    pub(crate) clipboard: &'a mut Option<String>,
}

impl EventCtx<'_> {
    /// True if `id` is the topmost widget under this pointer event (or the
    /// widget capturing the pointer).
    pub fn is_target(&self, id: WidgetId) -> bool {
        id != WidgetId::NONE && self.target == id
    }

    pub fn is_hovered(&self, id: WidgetId) -> bool {
        id != WidgetId::NONE && self.hovered == id
    }

    pub fn is_focused(&self, id: WidgetId) -> bool {
        id != WidgetId::NONE && self.focused == id
    }

    /// True while *any* widget holds keyboard focus — the guard an embedded
    /// accelerator checks so hover-scoped keys ([`PageKeys::WhenHovered`]
    /// paging) never rob the focused widget of its own Home/End/PgUp (the
    /// sibling of [`any_popup_open`](Self::any_popup_open)).
    ///
    /// [`PageKeys::WhenHovered`]: crate::PageKeys::WhenHovered
    pub fn any_focused(&self) -> bool {
        self.focused != WidgetId::NONE
    }

    /// Requests that `id` capture the pointer until the button is released.
    pub fn capture(&mut self, id: WidgetId) {
        *self.capture_request = Some(id);
    }

    /// Requests keyboard focus for `id`.
    pub fn request_focus(&mut self, id: WidgetId) {
        *self.focus_request = Some(id);
    }

    /// Opens a top-level popup owned by `id`. While open, the `Ui` routes
    /// pointer events to `id`, and reports the grab to the host as
    /// [`Response::capturing`] — which is what routes an outside press into
    /// this tree in a multi-`Ui` host, so the owner can dismiss on it. Guard
    /// the call with [`any_popup_open`](Self::any_popup_open): one owner at a
    /// time, and a widget's own open flag must track ownership
    /// (`self.open == true ⇔ ctx.popup_open(self.id)`) — replacing a live
    /// owner would strand its flag (both widgets would then consume events
    /// and draw overlays).
    ///
    /// **The owner's side of the contract:** the popup does not outlive the
    /// window's focus or the owner's keyboard. On [`Event::Focus`]`(false)`
    /// drop the open flag and call [`close_popup`](Self::close_popup) (the
    /// dismissing click may land in another window and never arrive) — the
    /// `Ui` clears its own routing state on the same event, exactly as it
    /// drops a pointer capture. On [`Event::Blur`] (the owner losing the
    /// keyboard) drop the open flag too; the `close_popup` request is ignored
    /// on that path and the `Ui` clears the routing state itself, so the
    /// widget's flag and the `Ui` stay in step. `Select`, `MenuBar` and
    /// `ContextMenu` are the reference implementations.
    pub fn open_popup(&mut self, id: WidgetId) {
        *self.popup_request = Some(PopupOp::Open(id));
    }

    /// Closes the current popup.
    pub fn close_popup(&mut self) {
        *self.popup_request = Some(PopupOp::Close);
    }

    /// True if a popup owned by `id` is currently open.
    pub fn popup_open(&self, id: WidgetId) -> bool {
        id != WidgetId::NONE && self.popup_owner == id
    }

    /// True while *any* popup is open — the guard an opener checks so it never
    /// replaces a live owner (see [`open_popup`](Self::open_popup)).
    pub fn any_popup_open(&self) -> bool {
        self.popup_owner != WidgetId::NONE
    }

    /// Records that `id` fired this frame (pollable via `Ui::fired`), optionally
    /// with an action tag for the host's command layer.
    pub fn fire(&mut self, id: WidgetId, action: Option<u64>) {
        self.fired.push(id);
        if let Some(a) = action {
            self.actions.push(a);
        }
    }

    /// Declares a modal/blocking layer for this dispatch (a [`crate::Modal`]
    /// with an open dialog). Reflected in [`Response::blocking`] so the host
    /// withholds all world input while it is set.
    pub fn block(&mut self) {
        self.response.blocking = true;
    }

    /// Hands `text` to the host for the OS clipboard (a text field's copy/cut
    /// chord) — the toolkit itself has no clipboard access. The host polls it
    /// after dispatch via [`Ui::take_clipboard`](crate::ui::Ui::take_clipboard).
    pub fn set_clipboard(&mut self, text: String) {
        *self.clipboard = Some(text);
    }

    /// Marks the pointer event as consumed by the UI.
    pub fn consume_pointer(&mut self) {
        self.response.pointer = true;
    }

    /// Marks the keyboard/text event as consumed by the UI.
    pub fn consume_keyboard(&mut self) {
        self.response.keyboard = true;
    }
}

/// What a widget *is* and *says*: a stable kind name plus the user-visible
/// label, if it has one — a button's text, a window's title, a field's
/// **placeholder** (never its value: values are content, and a masked field
/// must not leak through a test dump). Behind [`crate::semantics`]'s queries
/// and outline: logic tests and script drivers address widgets by what a user
/// sees instead of by id plumbing, and an accessibility layer has somewhere
/// to start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Semantics<'a> {
    /// The kind name, normally [`kind_of`] (the type's name).
    pub kind: &'static str,
    /// The user-visible label; `None` for anonymous/unlabeled widgets.
    pub label: Option<&'a str>,
}

impl<'a> Semantics<'a> {
    /// A kind with no label.
    pub fn new(kind: &'static str) -> Self {
        Semantics { kind, label: None }
    }

    /// A kind with a label; an empty label counts as none.
    pub fn labeled(kind: &'static str, label: &'a str) -> Self {
        Semantics {
            kind,
            label: (!label.is_empty()).then_some(label),
        }
    }
}

/// The default semantic kind: the type's own name, path- and generics-trimmed
/// (`wgpu_ui::widgets::Button` → `Button`). Rust documents `type_name` as
/// diagnostic-grade rather than contractual, so a widget whose kind is pinned
/// in committed test expectations may prefer a string literal; in practice the
/// trimmed name is stable and every built-in reads correctly through this.
pub fn kind_of<T: ?Sized>() -> &'static str {
    let full = std::any::type_name::<T>();
    let base = full.split('<').next().unwrap_or(full);
    base.rsplit("::").next().unwrap_or(base)
}

/// A retained UI element: it owns its state, lays itself out, draws, and handles
/// input. Object-safe so trees are `Box<dyn Widget>`. The `Any` bound lets the
/// host read a widget's typed state by id (see [`crate::ui::Ui::get`]).
pub trait Widget: Any {
    /// Reports the desired size given `avail` (logical px). May exceed `avail`
    /// (the parent decides how to handle overflow).
    fn measure(&mut self, avail: Size, ctx: &mut LayoutCtx) -> Size;

    /// The widget's hard size bounds (logical px). Constraint-aware parents
    /// honor them — [`crate::layout::Linear`] clamps each child's measured
    /// size and folds a flex child's main-axis bounds into its space
    /// distribution. Declare bounds on any widget by wrapping it in
    /// [`crate::layout::Constrained`], which also clamps itself for parents
    /// that size purely by `measure`. Default: unbounded.
    fn size_limits(&self) -> Limits {
        Limits::NONE
    }

    /// Assigns the final `rect` (logical px) and lays out children.
    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx);

    /// Emits draw commands for the widget and its children.
    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx);

    /// Handles an event; returns `true` if it was consumed. Default: ignore.
    fn event(&mut self, _ev: &Event, _ctx: &mut EventCtx) -> bool {
        false
    }

    /// The widget's arranged rectangle (valid after `arrange`).
    fn rect(&self) -> Rect;

    /// The widget's identity, or [`WidgetId::NONE`] if non-interactive.
    fn id(&self) -> WidgetId {
        WidgetId::NONE
    }

    /// The widget's semantic identity for [`crate::semantics`] queries and
    /// outlines. Optional — the default is the bare type name, no label;
    /// labeled widgets (buttons, checkboxes, windows) override with what the
    /// user reads on them.
    fn semantics(&self) -> Semantics<'_> {
        Semantics::new(kind_of::<Self>())
    }

    /// Whether this widget takes keyboard focus via Tab traversal (text fields,
    /// dropdowns). Default: no. The [`Ui`](crate::ui::Ui) cycles focus through
    /// the focusable widgets in tree order on Tab / Shift-Tab.
    fn accepts_focus(&self) -> bool {
        false
    }

    /// Whether this widget consumes committed text while focused (text
    /// fields). Hosts enable the OS IME exactly when the focused widget
    /// accepts text — see [`Ui::wants_text_input`](crate::ui::Ui::wants_text_input).
    fn accepts_text_input(&self) -> bool {
        false
    }

    /// The caret rectangle (logical px) anchoring the IME candidate window
    /// while this widget is focused; `None` for non-composing widgets — see
    /// [`Ui::ime_rect`](crate::ui::Ui::ime_rect).
    fn ime_rect(&self) -> Option<Rect> {
        None
    }

    /// Number of direct children. Default: leaf (0).
    fn child_count(&self) -> usize {
        0
    }

    /// Borrows direct child `i`, if any. Containers override.
    fn child(&self, _i: usize) -> Option<&dyn Widget> {
        None
    }

    /// Mutably borrows direct child `i`, if any. Containers override.
    fn child_mut(&mut self, _i: usize) -> Option<&mut dyn Widget> {
        None
    }

    /// The mouse cursor to show at `pos` (logical px). Consulted on the widget
    /// the pointer resolves to — the hovered widget, or the capturing one
    /// mid-drag (see [`Ui::cursor_icon`](crate::ui::Ui::cursor_icon)) — so a
    /// leaf answers for itself; containers don't forward. Default: the arrow.
    fn cursor(&self, _pos: Vec2) -> CursorIcon {
        CursorIcon::Default
    }

    /// The hover tooltip this widget carries, if any. Consulted on the
    /// **hovered** widget once the `Ui` decides the pointer has rested there
    /// ([`Ui`] owns the hover and the clock); the `Ui` then paints it centrally
    /// through [`Theme::tooltip`](crate::theme::Theme::tooltip) at the end of
    /// the overlay pass, above everything. Leaves with a tooltip override this
    /// ([`Button::tooltip`](crate::Button::tooltip) is the standard way to set
    /// one); containers don't forward — the hovered id is already the leaf's.
    fn tooltip(&self) -> Option<&str> {
        None
    }

    /// Returns the topmost interactive widget under `pos` (logical px), or
    /// `None`. Default: this widget if it is interactive and contains `pos`.
    /// Containers override to recurse children in reverse (top-most) order.
    fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
        if self.id() != WidgetId::NONE && self.rect().contains(pos) {
            Some(self.id())
        } else {
            None
        }
    }
}

/// The two layout axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

impl Axis {
    /// The extent along this axis.
    pub fn main(self, s: Size) -> f32 {
        match self {
            Axis::Horizontal => s.w,
            Axis::Vertical => s.h,
        }
    }

    /// The extent across this axis.
    pub fn cross(self, s: Size) -> f32 {
        match self {
            Axis::Horizontal => s.h,
            Axis::Vertical => s.w,
        }
    }

    /// Builds a [`Size`] from main/cross extents.
    pub fn size(self, main: f32, cross: f32) -> Size {
        match self {
            Axis::Horizontal => Size::new(main, cross),
            Axis::Vertical => Size::new(cross, main),
        }
    }

    /// Builds a [`Rect`] from `origin` plus main/cross positions and extents.
    pub fn rect(self, origin: Vec2, main_pos: f32, cross_pos: f32, main: f32, cross: f32) -> Rect {
        match self {
            Axis::Horizontal => Rect::new(origin.x + main_pos, origin.y + cross_pos, main, cross),
            Axis::Vertical => Rect::new(origin.x + cross_pos, origin.y + main_pos, cross, main),
        }
    }
}

/// Hard min/max size bounds a widget declares for itself (see
/// [`Widget::size_limits`]). **`min` wins over `max`** when the two
/// conflict — a widget never collapses below its minimum; children that
/// then don't fit overflow their parent deliberately (overflow is the
/// parent's documented policy, clipping the painter's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    pub min: Size,
    pub max: Size,
}

impl Limits {
    /// No bounds — the default for every widget.
    pub const NONE: Self = Self {
        min: Size::ZERO,
        max: Size::new(f32::INFINITY, f32::INFINITY),
    };

    /// Clamps `s` into the bounds, min winning over max on each axis.
    #[must_use]
    pub fn clamp(&self, s: Size) -> Size {
        Size::new(
            s.w.min(self.max.w).max(self.min.w),
            s.h.min(self.max.h).max(self.min.h),
        )
    }
}

/// A child's sizing along a [`crate::layout::Linear`]'s main axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    /// Exactly this many logical pixels.
    Fixed(f32),
    /// The child's measured (content) size.
    Fit,
    /// A share of the leftover space, by weight.
    Flex(f32),
}

/// Distribution of leftover main-axis space when there are no [`Length::Flex`]
/// children.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MainAlign {
    #[default]
    Start,
    Center,
    End,
    /// Equal gaps between children, filling the leftover.
    SpaceBetween,
}

/// Cross-axis placement of each child.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CrossAlign {
    #[default]
    Start,
    Center,
    End,
    /// Stretch each child to the container's cross extent.
    Stretch,
}
