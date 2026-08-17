//! Backend-agnostic input events.
//!
//! The host translates its window/input events (e.g. winit) into these and feeds
//! them to the UI. Pointer coordinates are in **logical UI pixels** — the host
//! divides physical pixels by the total scale (`ui_scale × dpi`) first, so
//! widgets never see physical pixels (see `docs/DESIGN-FROM-USAGE.md`).

use crate::geom::Vec2;

/// A pointer (mouse) button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PointerButton {
    Primary,
    Secondary,
    Middle,
}

/// Keyboard modifier state at the time of an event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// The "super"/Windows/Command key.
    pub logo: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        shift: false,
        ctrl: false,
        alt: false,
        logo: false,
    };

    /// The platform "command" modifier: `logo` on macOS, `ctrl` elsewhere.
    /// (macOS detection is the host's job; this is the cross-platform default.)
    pub fn command(&self) -> bool {
        self.ctrl
    }

    pub fn is_none(&self) -> bool {
        *self == Self::NONE
    }
}

/// A logical key. Character keys (for shortcuts) arrive as [`Key::Character`];
/// committed text for editing arrives separately as [`Event::Text`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Key {
    Character(char),
    Enter,
    Escape,
    Tab,
    Backspace,
    Delete,
    Insert,
    Space,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

/// Scroll/wheel delta. Positive `y` scrolls toward the end of the content
/// (down); positive `x` scrolls right. The host normalizes its native units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollDelta {
    /// Discrete wheel notches.
    Lines(Vec2),
    /// Precise pixel deltas (trackpads, high-resolution wheels).
    Pixels(Vec2),
}

/// One input event, in logical UI pixels.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// The pointer moved to `pos`.
    PointerMoved { pos: Vec2 },
    /// A pointer button was pressed or released at `pos`.
    PointerButton {
        button: PointerButton,
        pressed: bool,
        pos: Vec2,
        mods: Modifiers,
    },
    /// The wheel/trackpad scrolled while the pointer was at `pos`.
    Scroll {
        delta: ScrollDelta,
        pos: Vec2,
        mods: Modifiers,
    },
    /// A key was pressed or released (named keys + character keys for shortcuts).
    Key {
        key: Key,
        pressed: bool,
        repeat: bool,
        mods: Modifiers,
    },
    /// Committed text to insert (what a focused text field consumes).
    Text(String),
    /// Text pasted from the OS clipboard. The toolkit has no clipboard access
    /// (dependency-free): the host intercepts its paste chord (Ctrl+V), reads
    /// the OS clipboard, and dispatches this. The focused text field inserts
    /// it — a single-line field drops newlines via its control-char filter.
    /// The copy/cut half of the channel is
    /// [`Ui::take_clipboard`](crate::ui::Ui::take_clipboard).
    Paste(String),
    /// An IME composition update: the in-progress (preedit) text the focused
    /// field shows inline at its caret, with an optional caret/highlight byte
    /// range within it. Empty `text` clears the composition (the IME
    /// committed, cancelled, or was disabled); committed text then arrives as
    /// a plain [`Event::Text`]. Hosts drive the OS side from
    /// [`Ui::wants_text_input`](crate::ui::Ui::wants_text_input) (enable) and
    /// [`Ui::ime_rect`](crate::ui::Ui::ime_rect) (candidate-window anchor).
    ImePreedit {
        text: String,
        cursor: Option<(usize, usize)>,
    },
    /// The pointer left the window (clears hover).
    PointerLeft,
    /// The window gained (`true`) or lost (`false`) focus.
    Focus(bool),
    /// Keyboard focus **left** the widget this event targets, for `cause`. The
    /// one event the [`Ui`](crate::ui::Ui) derives rather than the host: focus
    /// moves as a side effect of a click, of Tab, or of a host call, so a widget
    /// cannot see it in the input stream. A text field turns it into its
    /// commit-on-focus-out ([`TextInput::take_commit`](crate::TextInput::take_commit)).
    Blur(BlurCause),
}

/// Why a widget lost keyboard focus — the difference between "I'm done with
/// this box" and "forget what I typed".
///
/// A field **commits** on [`Moved`](BlurCause::Moved) and **discards** on
/// [`Cancelled`](BlurCause::Cancelled). Without the distinction the toolkit
/// would have to pick one meaning for both, and either Escape starts applying
/// edits or clicking away keeps losing them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlurCause {
    /// Focus went somewhere else: another widget was clicked or Tabbed to, or
    /// the host handed the keyboard to another tree
    /// ([`Ui::blur`](crate::ui::Ui::blur)). The edit stands.
    Moved,
    /// The user backed out — Escape. The edit is abandoned.
    Cancelled,
}

impl Event {
    /// The pointer position carried by this event, if any.
    pub fn pos(&self) -> Option<Vec2> {
        match self {
            Event::PointerMoved { pos }
            | Event::PointerButton { pos, .. }
            | Event::Scroll { pos, .. } => Some(*pos),
            _ => None,
        }
    }

    /// True for pointer (mouse) events; false for keyboard/text/focus events.
    pub fn is_pointer(&self) -> bool {
        matches!(
            self,
            Event::PointerMoved { .. }
                | Event::PointerButton { .. }
                | Event::Scroll { .. }
                | Event::PointerLeft
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_none_is_default() {
        assert_eq!(Modifiers::default(), Modifiers::NONE);
        assert!(Modifiers::NONE.is_none());
    }

    /// The cross-platform "command" chord modifier maps to Ctrl (remapping to
    /// the logo key on macOS is the host's job).
    #[test]
    fn command_is_ctrl_on_the_default_mapping() {
        let ctrl = Modifiers {
            ctrl: true,
            ..Modifiers::NONE
        };
        assert!(ctrl.command());
        assert!(!Modifiers::NONE.command());
        let shift = Modifiers {
            shift: true,
            ..Modifiers::NONE
        };
        assert!(!shift.command(), "shift alone is not the command chord");
    }

    #[test]
    fn pos_only_for_pointer_events() {
        let p = Event::PointerMoved {
            pos: Vec2::new(3.0, 4.0),
        };
        assert_eq!(p.pos(), Some(Vec2::new(3.0, 4.0)));
        assert!(p.is_pointer());

        let k = Event::Key {
            key: Key::Enter,
            pressed: true,
            repeat: false,
            mods: Modifiers::NONE,
        };
        assert_eq!(k.pos(), None);
        assert!(!k.is_pointer());
    }
}
