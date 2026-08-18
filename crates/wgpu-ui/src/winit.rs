//! winit integration (behind the `winit` feature): translate winit window
//! events into [`crate::event::Event`]s.
//!
//! [`WinitInput`] tracks the cursor, modifiers, and scale factor needed to map
//! winit's physical coordinates into the library's logical UI pixels.

use crate::event::{Event, Key, Modifiers, PointerButton, ScrollDelta};
use crate::geom::Vec2;
use crate::interact::CursorIcon;

use ::winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use ::winit::keyboard::{Key as WKey, NamedKey};

/// Maps a [`CursorIcon`] to its winit counterpart — apply the result of
/// [`Ui::cursor_icon`](crate::ui::Ui::cursor_icon) with
/// `window.set_cursor(map_cursor(icon))` after dispatching pointer events
/// (cache the last value; setting the OS cursor every event is wasteful).
pub fn map_cursor(icon: CursorIcon) -> ::winit::window::CursorIcon {
    use ::winit::window::CursorIcon as W;
    match icon {
        CursorIcon::Text => W::Text,
        CursorIcon::Pointer => W::Pointer,
        CursorIcon::Grabbing => W::Grabbing,
        CursorIcon::ResizeEW => W::EwResize,
        CursorIcon::ResizeNS => W::NsResize,
        CursorIcon::ResizeNWSE => W::NwseResize,
        _ => W::Default,
    }
}

/// Translates winit window events into `Event`s.
pub struct WinitInput {
    cursor: Vec2,
    mods: Modifiers,
    scale: f64,
}

impl WinitInput {
    /// `scale` is logical→physical (e.g. `window.scale_factor()`). Use `1.0` to
    /// work directly in physical pixels.
    pub fn new(scale: f64) -> Self {
        Self {
            cursor: Vec2::ZERO,
            mods: Modifiers::NONE,
            scale: scale.max(1e-4),
        }
    }

    pub fn set_scale(&mut self, scale: f64) {
        self.scale = scale.max(1e-4);
    }

    /// The current logical cursor position.
    pub fn cursor(&self) -> Vec2 {
        self.cursor
    }

    /// Translates one window event, appending 0 or more `Event`s to `out`.
    pub fn handle(&mut self, ev: &WindowEvent, out: &mut Vec<Event>) {
        match ev {
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = Vec2::new(
                    (position.x / self.scale) as f32,
                    (position.y / self.scale) as f32,
                );
                out.push(Event::PointerMoved { pos: self.cursor });
            }
            WindowEvent::CursorLeft { .. } => out.push(Event::PointerLeft),
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(b) = map_button(*button) {
                    out.push(Event::PointerButton {
                        button: b,
                        pressed: *state == ElementState::Pressed,
                        pos: self.cursor,
                        mods: self.mods,
                    });
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let delta = match delta {
                    MouseScrollDelta::LineDelta(x, y) => ScrollDelta::Lines(Vec2::new(*x, -*y)),
                    MouseScrollDelta::PixelDelta(p) => ScrollDelta::Pixels(Vec2::new(
                        (p.x / self.scale) as f32,
                        -(p.y / self.scale) as f32,
                    )),
                };
                out.push(Event::Scroll {
                    delta,
                    pos: self.cursor,
                    mods: self.mods,
                });
            }
            WindowEvent::ModifiersChanged(m) => {
                let s = m.state();
                self.mods = Modifiers {
                    shift: s.shift_key(),
                    ctrl: s.control_key(),
                    alt: s.alt_key(),
                    logo: s.super_key(),
                };
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state == ElementState::Pressed;
                if let Some(k) = map_key(&event.logical_key) {
                    out.push(Event::Key {
                        key: k,
                        pressed,
                        repeat: event.repeat,
                        mods: self.mods,
                    });
                }
                if pressed
                    && text_inserts(self.mods)
                    && let Some(t) = &event.text
                {
                    out.push(Event::Text(t.as_str().to_string()));
                }
            }
            // IME (the host opts in with `window.set_ime_allowed`, driven by
            // `Ui::wants_text_input`): composition updates map to
            // `ImePreedit`, committed text to plain `Text`; disabling clears
            // any live composition.
            WindowEvent::Ime(ime) => match ime {
                Ime::Preedit(s, cursor) => out.push(Event::ImePreedit {
                    text: s.clone(),
                    cursor: *cursor,
                }),
                Ime::Commit(s) => out.push(Event::Text(s.clone())),
                Ime::Disabled => out.push(Event::ImePreedit {
                    text: String::new(),
                    cursor: None,
                }),
                Ime::Enabled => {}
            },
            WindowEvent::Focused(f) => out.push(Event::Focus(*f)),
            _ => {}
        }
    }
}

/// Should a key press's `text` payload become an `Event::Text`? A held Ctrl
/// means the press is a chord, not typing — X11/Wayland still report the
/// plain character as `text` (e.g. `"v"` for Ctrl+V), so without this filter
/// a host that answers the chord (paste, shortcuts) sees the stray character
/// inserted right after. Ctrl+Alt stays typeable: that combination is AltGr
/// on many layouts and MUST produce text.
fn text_inserts(mods: Modifiers) -> bool {
    !(mods.ctrl && !mods.alt)
}

fn map_button(b: MouseButton) -> Option<PointerButton> {
    match b {
        MouseButton::Left => Some(PointerButton::Primary),
        MouseButton::Right => Some(PointerButton::Secondary),
        MouseButton::Middle => Some(PointerButton::Middle),
        _ => None,
    }
}

fn map_key(k: &WKey) -> Option<Key> {
    Some(match k {
        WKey::Named(n) => match n {
            NamedKey::Enter => Key::Enter,
            NamedKey::Escape => Key::Escape,
            NamedKey::Tab => Key::Tab,
            NamedKey::Backspace => Key::Backspace,
            NamedKey::Delete => Key::Delete,
            NamedKey::Insert => Key::Insert,
            NamedKey::Space => Key::Space,
            NamedKey::ArrowLeft => Key::Left,
            NamedKey::ArrowRight => Key::Right,
            NamedKey::ArrowUp => Key::Up,
            NamedKey::ArrowDown => Key::Down,
            NamedKey::Home => Key::Home,
            NamedKey::End => Key::End,
            NamedKey::PageUp => Key::PageUp,
            NamedKey::PageDown => Key::PageDown,
            NamedKey::F1 => Key::F1,
            NamedKey::F2 => Key::F2,
            NamedKey::F3 => Key::F3,
            NamedKey::F4 => Key::F4,
            NamedKey::F5 => Key::F5,
            NamedKey::F6 => Key::F6,
            NamedKey::F7 => Key::F7,
            NamedKey::F8 => Key::F8,
            NamedKey::F9 => Key::F9,
            NamedKey::F10 => Key::F10,
            NamedKey::F11 => Key::F11,
            NamedKey::F12 => Key::F12,
            _ => return None,
        },
        WKey::Character(s) => Key::Character(s.chars().next()?),
        _ => return None,
    })
}

// What these tests can and cannot reach: every `WindowEvent` variant `handle`
// matches on is publicly constructible (`DeviceId::dummy()` exists for exactly
// this) — except `KeyboardInput`, whose `winit::event::KeyEvent` has a private
// `platform_specific` field and no public constructor. That one arm
// (state/repeat/text plumbing) is therefore untestable here; its mapping logic
// (`map_key`, `text_inserts`) is unit-tested directly instead.
#[cfg(test)]
mod tests {
    use super::*;

    use ::winit::dpi::PhysicalPosition;
    use ::winit::event::{DeviceId, TouchPhase};
    use ::winit::keyboard::{ModifiersState, NativeKey, SmolStr};

    fn cursor_moved(x: f64, y: f64) -> WindowEvent {
        WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(x, y),
        }
    }

    fn mouse_input(state: ElementState, button: MouseButton) -> WindowEvent {
        WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button,
        }
    }

    fn wheel(delta: MouseScrollDelta) -> WindowEvent {
        WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta,
            phase: TouchPhase::Moved,
        }
    }

    /// Every toolkit cursor maps to the matching winit cursor; the catch-all
    /// arm (here `Default`) falls back to the plain arrow.
    #[test]
    fn map_cursor_matches_toolkit_variants() {
        use ::winit::window::CursorIcon as W;
        assert_eq!(map_cursor(CursorIcon::Text), W::Text);
        assert_eq!(map_cursor(CursorIcon::Pointer), W::Pointer);
        assert_eq!(map_cursor(CursorIcon::Grabbing), W::Grabbing);
        assert_eq!(map_cursor(CursorIcon::ResizeEW), W::EwResize);
        assert_eq!(map_cursor(CursorIcon::ResizeNS), W::NsResize);
        assert_eq!(map_cursor(CursorIcon::ResizeNWSE), W::NwseResize);
        assert_eq!(map_cursor(CursorIcon::Default), W::Default);
    }

    /// `CursorMoved` divides physical coordinates by the scale factor so the
    /// UI sees logical pixels, and the translator remembers the position
    /// (`cursor()`).
    #[test]
    fn cursor_moved_scales_physical_to_logical() {
        let mut input = WinitInput::new(2.0);
        let mut out = Vec::new();
        input.handle(&cursor_moved(200.0, 100.0), &mut out);
        assert_eq!(
            out,
            vec![Event::PointerMoved {
                pos: Vec2::new(100.0, 50.0),
            }],
            "physical position / scale"
        );
        assert_eq!(
            input.cursor(),
            Vec2::new(100.0, 50.0),
            "the logical position is remembered"
        );
    }

    /// `set_scale` re-maps subsequent positions, and degenerate scales (zero
    /// or negative, e.g. from a bogus compositor) are clamped so coordinates
    /// stay finite instead of dividing by zero.
    #[test]
    fn scale_updates_and_degenerate_scales_stay_finite() {
        let mut input = WinitInput::new(1.0);
        let mut out = Vec::new();
        input.set_scale(2.0);
        input.handle(&cursor_moved(300.0, 100.0), &mut out);
        assert_eq!(
            input.cursor(),
            Vec2::new(150.0, 50.0),
            "set_scale applies to later events"
        );

        input.set_scale(0.0);
        input.handle(&cursor_moved(5.0, 5.0), &mut out);
        assert!(
            input.cursor().x.is_finite() && input.cursor().y.is_finite(),
            "a zero scale is clamped, not divided by"
        );

        let mut neg = WinitInput::new(-3.0);
        neg.handle(&cursor_moved(5.0, 5.0), &mut out);
        assert!(
            neg.cursor().x.is_finite() && neg.cursor().y.is_finite(),
            "a negative construction scale is clamped"
        );
    }

    /// The pointer leaving the window becomes `PointerLeft` (the event that
    /// clears widget hover).
    #[test]
    fn cursor_left_translates() {
        let mut input = WinitInput::new(1.0);
        let mut out = Vec::new();
        input.handle(
            &WindowEvent::CursorLeft {
                device_id: DeviceId::dummy(),
            },
            &mut out,
        );
        assert_eq!(out, vec![Event::PointerLeft]);
    }

    /// Presses and releases of Left/Right/Middle map to
    /// Primary/Secondary/Middle `PointerButton`s stamped with the last-known
    /// cursor position; the extra buttons (Back/Forward/Other) are dropped
    /// rather than mis-mapped.
    #[test]
    fn mouse_input_translates_and_stamps_cursor() {
        let mut input = WinitInput::new(1.0);
        let mut out = Vec::new();
        input.handle(&cursor_moved(40.0, 30.0), &mut out);

        for (winit_btn, ours) in [
            (MouseButton::Left, PointerButton::Primary),
            (MouseButton::Right, PointerButton::Secondary),
            (MouseButton::Middle, PointerButton::Middle),
        ] {
            out.clear();
            input.handle(&mouse_input(ElementState::Pressed, winit_btn), &mut out);
            input.handle(&mouse_input(ElementState::Released, winit_btn), &mut out);
            assert_eq!(
                out,
                vec![
                    Event::PointerButton {
                        button: ours,
                        pressed: true,
                        pos: Vec2::new(40.0, 30.0),
                        mods: Modifiers::NONE,
                    },
                    Event::PointerButton {
                        button: ours,
                        pressed: false,
                        pos: Vec2::new(40.0, 30.0),
                        mods: Modifiers::NONE,
                    },
                ],
                "{winit_btn:?} maps to {ours:?} at the tracked cursor"
            );
        }

        out.clear();
        input.handle(
            &mouse_input(ElementState::Pressed, MouseButton::Back),
            &mut out,
        );
        input.handle(
            &mouse_input(ElementState::Pressed, MouseButton::Forward),
            &mut out,
        );
        input.handle(
            &mouse_input(ElementState::Pressed, MouseButton::Other(5)),
            &mut out,
        );
        assert!(
            out.is_empty(),
            "unmapped buttons are dropped, not mis-mapped"
        );
    }

    /// Wheel notches become `ScrollDelta::Lines` with `y` flipped (winit's
    /// positive `y` is away-from-user; the toolkit's is toward-content-end),
    /// and pixel deltas additionally divide by the scale factor. Line deltas
    /// are unitless notches, so they are never scaled.
    #[test]
    fn mouse_wheel_flips_y_and_scales_pixels() {
        let mut input = WinitInput::new(2.0);
        let mut out = Vec::new();
        input.handle(&wheel(MouseScrollDelta::LineDelta(1.0, 3.0)), &mut out);
        input.handle(
            &wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
                10.0, -30.0,
            ))),
            &mut out,
        );
        assert_eq!(
            out,
            vec![
                Event::Scroll {
                    delta: ScrollDelta::Lines(Vec2::new(1.0, -3.0)),
                    pos: Vec2::ZERO,
                    mods: Modifiers::NONE,
                },
                Event::Scroll {
                    delta: ScrollDelta::Pixels(Vec2::new(5.0, 15.0)),
                    pos: Vec2::ZERO,
                    mods: Modifiers::NONE,
                },
            ]
        );
    }

    /// `ModifiersChanged` emits no UI event of its own but stamps every
    /// subsequent pointer event (all four modifiers tracked) until the next
    /// change clears it.
    #[test]
    fn modifiers_stamp_subsequent_events() {
        let mut input = WinitInput::new(1.0);
        let mut out = Vec::new();
        let all = ModifiersState::SHIFT
            | ModifiersState::CONTROL
            | ModifiersState::ALT
            | ModifiersState::SUPER;
        input.handle(&WindowEvent::ModifiersChanged(all.into()), &mut out);
        assert!(out.is_empty(), "a modifier change alone emits no UI event");

        input.handle(
            &mouse_input(ElementState::Pressed, MouseButton::Left),
            &mut out,
        );
        assert_eq!(
            out,
            vec![Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                pos: Vec2::ZERO,
                mods: Modifiers {
                    shift: true,
                    ctrl: true,
                    alt: true,
                    logo: true,
                },
            }],
            "the held modifiers stamp the click"
        );

        out.clear();
        input.handle(
            &WindowEvent::ModifiersChanged(ModifiersState::empty().into()),
            &mut out,
        );
        input.handle(&wheel(MouseScrollDelta::LineDelta(0.0, 1.0)), &mut out);
        assert_eq!(
            out,
            vec![Event::Scroll {
                delta: ScrollDelta::Lines(Vec2::new(0.0, -1.0)),
                pos: Vec2::ZERO,
                mods: Modifiers::NONE,
            }],
            "releasing the modifiers clears the stamp"
        );
    }

    /// Window focus gain/loss maps to `Event::Focus`; `Ime::Enabled` and
    /// unrelated window chatter (a window move) produce no UI events.
    #[test]
    fn focus_translates_and_chatter_is_ignored() {
        let mut input = WinitInput::new(1.0);
        let mut out = Vec::new();
        input.handle(&WindowEvent::Focused(true), &mut out);
        input.handle(&WindowEvent::Focused(false), &mut out);
        assert_eq!(out, vec![Event::Focus(true), Event::Focus(false)]);

        out.clear();
        input.handle(&WindowEvent::Ime(Ime::Enabled), &mut out);
        input.handle(&WindowEvent::Moved(PhysicalPosition::new(3, 4)), &mut out);
        assert!(
            out.is_empty(),
            "IME-enabled and window-move produce no UI events"
        );
    }

    /// Every supported named key maps to its toolkit counterpart; unsupported
    /// named keys (F13), dead keys, and unidentified keys map to `None` so they
    /// never reach widgets as bogus input.
    #[test]
    fn map_key_named_and_rejections() {
        for (named, ours) in [
            (NamedKey::Enter, Key::Enter),
            (NamedKey::Escape, Key::Escape),
            (NamedKey::Tab, Key::Tab),
            (NamedKey::Backspace, Key::Backspace),
            (NamedKey::Delete, Key::Delete),
            (NamedKey::Insert, Key::Insert),
            (NamedKey::Space, Key::Space),
            (NamedKey::ArrowLeft, Key::Left),
            (NamedKey::ArrowRight, Key::Right),
            (NamedKey::ArrowUp, Key::Up),
            (NamedKey::ArrowDown, Key::Down),
            (NamedKey::Home, Key::Home),
            (NamedKey::End, Key::End),
            (NamedKey::PageUp, Key::PageUp),
            (NamedKey::PageDown, Key::PageDown),
            (NamedKey::F1, Key::F1),
            (NamedKey::F2, Key::F2),
            (NamedKey::F3, Key::F3),
            (NamedKey::F4, Key::F4),
            (NamedKey::F5, Key::F5),
            (NamedKey::F6, Key::F6),
            (NamedKey::F7, Key::F7),
            (NamedKey::F8, Key::F8),
            (NamedKey::F9, Key::F9),
            (NamedKey::F10, Key::F10),
            (NamedKey::F11, Key::F11),
            (NamedKey::F12, Key::F12),
        ] {
            assert_eq!(map_key(&WKey::Named(named)), Some(ours), "{named:?}");
        }
        assert_eq!(
            map_key(&WKey::Named(NamedKey::F13)),
            None,
            "unsupported named key"
        );
        assert_eq!(map_key(&WKey::Dead(None)), None, "dead key");
        assert_eq!(
            map_key(&WKey::Unidentified(NativeKey::Unidentified)),
            None,
            "unidentified key"
        );
    }

    /// Character keys map to `Key::Character` on their first char: non-ASCII
    /// works, a multi-char sequence keeps the lead char, and an empty string
    /// maps to `None` instead of panicking.
    #[test]
    fn map_key_characters() {
        assert_eq!(
            map_key(&WKey::Character(SmolStr::new("a"))),
            Some(Key::Character('a'))
        );
        assert_eq!(
            map_key(&WKey::Character(SmolStr::new("ß"))),
            Some(Key::Character('ß')),
            "non-ASCII character"
        );
        assert_eq!(
            map_key(&WKey::Character(SmolStr::new("ab"))),
            Some(Key::Character('a')),
            "lead char of a multi-char sequence"
        );
        assert_eq!(
            map_key(&WKey::Character(SmolStr::new(""))),
            None,
            "empty character string"
        );
    }

    /// A key press's `text` inserts only when it is typing, not a chord: a
    /// held Ctrl suppresses it (the platform reports the plain character —
    /// Ctrl+V must not insert "v"), while Ctrl+Alt (AltGr on many layouts)
    /// and every other modifier combination keep typing.
    #[test]
    fn text_inserts_filters_ctrl_chords() {
        let m = |shift, ctrl, alt, logo| Modifiers {
            shift,
            ctrl,
            alt,
            logo,
        };
        assert!(text_inserts(Modifiers::NONE), "plain typing");
        assert!(text_inserts(m(true, false, false, false)), "shift types");
        assert!(
            text_inserts(m(false, true, true, false)),
            "AltGr (ctrl+alt) must type"
        );
        assert!(text_inserts(m(false, false, true, false)), "bare alt types");
        assert!(!text_inserts(m(false, true, false, false)), "ctrl chord");
        assert!(
            !text_inserts(m(true, true, false, false)),
            "ctrl+shift chord"
        );
        assert!(
            !text_inserts(m(false, true, false, true)),
            "ctrl+logo chord"
        );
    }

    /// IME window events translate to the toolkit's composition events:
    /// preedit updates pass through, a commit becomes plain committed text,
    /// and disabling clears any live composition.
    #[test]
    fn ime_events_translate() {
        let mut input = WinitInput::new(1.0);
        let mut out = Vec::new();
        input.handle(
            &WindowEvent::Ime(Ime::Preedit("あ".into(), Some((3, 3)))),
            &mut out,
        );
        input.handle(&WindowEvent::Ime(Ime::Commit("あ".into())), &mut out);
        input.handle(&WindowEvent::Ime(Ime::Disabled), &mut out);
        assert_eq!(
            out,
            vec![
                Event::ImePreedit {
                    text: "あ".into(),
                    cursor: Some((3, 3)),
                },
                Event::Text("あ".into()),
                Event::ImePreedit {
                    text: String::new(),
                    cursor: None,
                },
            ]
        );
    }
}
