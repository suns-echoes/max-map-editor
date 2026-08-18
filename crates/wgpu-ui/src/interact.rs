//! Interaction primitives shared by every widget: identity, commit policy, the
//! visual state a theme paints from, the [`Response`] the host uses to route
//! un-consumed input to its own world (see `docs/DESIGN-FROM-USAGE.md`), and
//! the [`ArmFire`] machine a custom multi-target widget embeds.

use crate::event::{Event, PointerButton};
use crate::geom::Vec2;
use crate::widget::EventCtx;

/// A stable per-widget identity used for focus, pointer capture, and polling
/// outcomes. Interactive widgets allocate one at construction via [`next_id`]
/// (hosts can also mint ids from an [`IdGen`]); the id is stable for the
/// widget's lifetime, which is what lets the host poll outcomes by id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WidgetId(pub u64);

impl WidgetId {
    /// A reserved "no widget" id.
    pub const NONE: WidgetId = WidgetId(0);
}

/// Monotonic [`WidgetId`] generator (ids start at 1; 0 is [`WidgetId::NONE`]).
///
/// **Every generator counts from scratch**, so two of them hand out the same
/// ids — and those collide with [`next_id`]'s, which also starts at 1. Use one
/// only where every id in play comes from the *same* generator; a widget that
/// will sit in a tree beside widgets it did not mint (anything hosting a child)
/// must take its id from [`next_id`].
#[derive(Debug, Default)]
pub struct IdGen(u64);

impl IdGen {
    pub fn new() -> Self {
        Self(0)
    }

    /// Allocates the next unique id.
    pub fn alloc(&mut self) -> WidgetId {
        self.0 += 1;
        WidgetId(self.0)
    }
}

/// Allocates a process-unique [`WidgetId`]. Interactive widgets call this once at
/// construction; the id is stable for the widget's lifetime (the tree is
/// retained), which is what lets the host poll outcomes by id.
pub fn next_id() -> WidgetId {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    WidgetId(NEXT.fetch_add(1, Ordering::Relaxed))
}

/// The mouse cursor a widget wants shown while it is hovered (or, mid-drag,
/// while it captures the pointer) — resolved per frame by
/// [`Ui::cursor_icon`](crate::ui::Ui::cursor_icon) and applied by the host
/// (the `winit` feature maps it via [`crate::winit::map_cursor`]). Widgets
/// report it from [`Widget::cursor`](crate::widget::Widget::cursor); the
/// default everywhere is `Default` (the plain arrow — desktop convention:
/// buttons don't change the cursor, text fields and resize affordances do).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum CursorIcon {
    /// The plain arrow.
    #[default]
    Default,
    /// The text I-beam (over editable text).
    Text,
    /// The pointing hand (a link or other navigate-on-click surface —
    /// NOT buttons, which keep the arrow per desktop convention).
    Pointer,
    /// The hand of an in-progress grab (a titlebar drag).
    Grabbing,
    /// Horizontal resize (a vertical splitter / left-right edge).
    ResizeEW,
    /// Vertical resize (a horizontal splitter / top-bottom edge).
    ResizeNS,
    /// Diagonal resize, NW-SE (a bottom-right grip).
    ResizeNWSE,
}

/// When a clickable widget commits its action.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CommitPolicy {
    /// Fire on press (menus, immediate selections like a palette swatch).
    PressFire,
    /// Arm on press, fire on release while still over the widget (buttons). A
    /// press that releases elsewhere is cancelled. This is the GNW/`max-*` model.
    #[default]
    ReleaseInside,
}

/// The arm/fire pointer machine for a custom widget with many internal hit
/// targets — a tab strip, a header toolbar, a grid of tool keys. The widget
/// keeps a *pure* hit oracle (`Vec2 -> Option<A>`, shared with its tests);
/// this machine owns the interaction state:
///
/// - a primary press over a hit **arms** it and consumes the press; a press
///   over empty chrome is NOT consumed, so it falls through to the host;
/// - a release over the **same** hit fires it into the outcome (poll with
///   [`take_outcome`](Self::take_outcome)); a release elsewhere cancels
///   (the release-inside rule every stock clickable follows);
/// - window focus loss disarms (the release may never arrive).
///
/// [`event_with`](Self::event_with) is the same machine with a **per-hit**
/// [`CommitPolicy`], for a widget whose targets are not all buttons.
///
/// This is the state machine every embedded panel widget otherwise re-writes
/// by hand. Paint the [`armed`](Self::armed) hit as pressed.
#[derive(Debug)]
pub struct ArmFire<A: Copy + PartialEq> {
    armed: Option<A>,
    outcome: Option<A>,
}

impl<A: Copy + PartialEq> Default for ArmFire<A> {
    fn default() -> Self {
        Self {
            armed: None,
            outcome: None,
        }
    }
}

impl<A: Copy + PartialEq> ArmFire<A> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one event through the machine. `id` is the owning widget's id
    /// (presses are only taken when it is the dispatch target); `hit` is the
    /// widget's pure hit oracle. Returns whether the event was consumed.
    ///
    /// Every hit commits [`CommitPolicy::ReleaseInside`] — the button rule. Use
    /// [`event_with`](Self::event_with) when some of the widget's targets are
    /// not buttons.
    pub fn event(
        &mut self,
        ev: &Event,
        ctx: &mut EventCtx,
        id: WidgetId,
        hit: impl Fn(Vec2) -> Option<A>,
    ) -> bool {
        self.event_with(ev, ctx, id, hit, |_| CommitPolicy::ReleaseInside)
    }

    /// [`event`](Self::event) with a per-hit [`CommitPolicy`].
    ///
    /// A widget with mixed targets needs the distinction, and one hit oracle
    /// must still answer for all of them: on a palette panel a swatch selects
    /// on **press** (as does an HSL slider, which would otherwise lose every
    /// move between the press and the release that started its drag), while the
    /// toolbar button beside it arms and fires on release-inside. Splitting
    /// those across two machines would mean two oracles disagreeing about the
    /// same pixel.
    ///
    /// Both kinds land in the same outcome slot, so a host that polls
    /// [`take_outcome`](Self::take_outcome) after *each* dispatch — press as
    /// well as release — sees them in the order they happened.
    pub fn event_with(
        &mut self,
        ev: &Event,
        ctx: &mut EventCtx,
        id: WidgetId,
        hit: impl Fn(Vec2) -> Option<A>,
        commit: impl Fn(A) -> CommitPolicy,
    ) -> bool {
        match ev {
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                pos,
                ..
            } if ctx.is_target(id) => {
                if let Some(h) = hit(*pos) {
                    match commit(h) {
                        CommitPolicy::PressFire => self.outcome = Some(h),
                        CommitPolicy::ReleaseInside => self.armed = Some(h),
                    }
                    ctx.consume_pointer();
                    return true;
                }
                false
            }
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: false,
                pos,
                ..
            } if self.armed.is_some() => {
                if let Some(h) = self.armed.take()
                    && hit(*pos) == Some(h)
                {
                    self.outcome = Some(h);
                }
                ctx.consume_pointer();
                true
            }
            Event::Focus(false) => {
                self.armed = None;
                false
            }
            _ => false,
        }
    }

    /// The currently armed hit (draw it pressed), if any.
    pub fn armed(&self) -> Option<A> {
        self.armed
    }

    /// Takes the fired hit, if a press→release-inside completed since the
    /// last poll.
    pub fn take_outcome(&mut self) -> Option<A> {
        self.outcome.take()
    }
}

/// The interaction/visual state of a control, resolved by the dispatcher and
/// read by the [`crate::theme`] when painting — so every theme paints the same
/// states the same way. This is the *only* channel from behavior to look.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WidgetState {
    pub hovered: bool,
    /// Armed/held (pointer down on it, or actively pressed).
    pub pressed: bool,
    pub focused: bool,
    pub disabled: bool,
    /// Checked/toggled/selected (checkbox, toggle, radio, list row, tab).
    pub selected: bool,
}

impl WidgetState {
    pub const DISABLED: Self = Self {
        hovered: false,
        pressed: false,
        focused: false,
        disabled: true,
        selected: false,
    };
}

/// What the UI did with a dispatched batch of events. The host reads this to
/// decide whether to also route input to its world/map.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Response {
    /// A pointer event landed on the UI.
    pub pointer: bool,
    /// A keyboard/text event was consumed (e.g. typing into a focused field).
    pub keyboard: bool,
    /// The UI holds the pointer stream — a drag is in progress, or a popup
    /// (dropdown list, menu cascade) is open. Route **all** pointer input to
    /// this `Ui` and withhold it from the world until this clears: for a drag
    /// that is what keeps the stroke alive outside the widget, and for a popup
    /// it is what delivers the outside press that dismisses it (instead of
    /// letting that press act on whatever lies underneath).
    pub capturing: bool,
    /// A modal/blocking layer is open; the world should ignore all input.
    pub blocking: bool,
}

impl Response {
    /// True when the host should withhold pointer input from its world.
    pub fn wants_pointer(&self) -> bool {
        self.pointer || self.capturing || self.blocking
    }

    /// True when the host should withhold keyboard input from its world.
    pub fn wants_keyboard(&self) -> bool {
        self.keyboard || self.blocking
    }

    /// Combines two responses (logical OR of every flag).
    pub fn or(self, o: Response) -> Response {
        Response {
            pointer: self.pointer || o.pointer,
            keyboard: self.keyboard || o.keyboard,
            capturing: self.capturing || o.capturing,
            blocking: self.blocking || o.blocking,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_nonzero() {
        let mut ids = IdGen::new();
        let a = ids.alloc();
        let b = ids.alloc();
        assert_ne!(a, b);
        assert_ne!(a, WidgetId::NONE);
    }

    #[test]
    fn default_commit_is_release_inside() {
        assert_eq!(CommitPolicy::default(), CommitPolicy::ReleaseInside);
    }

    #[test]
    fn response_routing() {
        let none = Response::default();
        assert!(!none.wants_pointer() && !none.wants_keyboard());

        let capturing = Response {
            capturing: true,
            ..Default::default()
        };
        assert!(capturing.wants_pointer());
        assert!(!capturing.wants_keyboard());

        let blocking = Response {
            blocking: true,
            ..Default::default()
        };
        assert!(blocking.wants_pointer() && blocking.wants_keyboard());

        let merged = Response {
            pointer: true,
            ..Default::default()
        }
        .or(Response {
            keyboard: true,
            ..Default::default()
        });
        assert!(merged.pointer && merged.keyboard);
    }
}
