//! A single-line field for secrets ([`SecretInput`], the `secret` feature).
//!
//! [`TextInput`](crate::TextInput) must not hold a password: its buffer is a
//! plain `String` (dropped unwiped, cloned whole per typed char by the charset
//! filter), its selection copies to the OS clipboard, and an outline dump
//! could print it. This field closes every one of those paths *by
//! construction* rather than by caller discipline:
//!
//! - the buffer is a [`Zeroizing<String>`] — wiped when dropped, and edits are
//!   in-place (no transient whole-buffer clones). The one residue accepted:
//!   a `String` that reallocates while growing may leave fragments of the old
//!   allocation behind; wiping those needs allocator cooperation no safe-Rust
//!   widget can provide.
//! - while **masked** (the initial state) it renders one bullet per char; the
//!   value never reaches a draw list, x-table, or shaping cache.
//!   [`set_masked`](SecretInput::set_masked) is the host's reveal gate — it
//!   changes how the buffer renders, never the buffer.
//! - there is **no selection**, and Ctrl+C/X are consumed as no-ops: nothing
//!   the field does can export the value to the clipboard. Paste *in* works —
//!   the clipboard already holds that text.
//! - [`accepts_text_input`](crate::widget::Widget::accepts_text_input) is
//!   `false`, so a host driving the OS IME from
//!   [`Ui::wants_text_input`](crate::ui::Ui::wants_text_input) keeps it off
//!   here: a composition would echo the secret in the IME's own UI. Plain
//!   typing (including dead-key compose) still arrives as [`Event::Text`];
//!   a stray [`Event::ImePreedit`] is consumed and dropped.
//! - `Debug` is hand-written and always redacts the value — revealed or not.
//!
//! Everything else is [`TextInput`](crate::TextInput)'s contract: focus on
//! click, caret editing, and the same [`TextCommit`] Enter/focus-out story, so
//! a dialog polls it exactly like its other fields.

/// Re-export so hosts can name [`value`](SecretInput::value)'s wiped-on-drop
/// return type without their own zeroize dependency (and stay pinned to the
/// exact version this crate was built against).
pub use zeroize::Zeroizing;

use crate::draw::DrawList;
use crate::event::{BlurCause, Event, Key, PointerButton};
use crate::geom::{Rect, Size, Vec2};
use crate::interact::{CursorIcon, WidgetId, WidgetState, next_id};
use crate::textedit::{CARET_W, LineCache, PAD_X, PAD_Y, TextCommit, nth_char_byte};
use crate::theme::TextRole;
use crate::widget::{DrawCtx, EventCtx, LayoutCtx, Semantics, Widget, kind_of};

/// What a masked char renders as (one per char, so length still reads).
const BULLET: &str = "\u{2022}";

/// A single-line input for passwords and other secrets: wiped-on-drop buffer,
/// bullet rendering while masked, no copy-out, no IME — see the module docs
/// for the full contract. Starts masked; the value is read back with
/// [`value`](Self::value) (itself a wiped-on-drop copy).
#[must_use]
pub struct SecretInput {
    id: WidgetId,
    value: Zeroizing<String>,
    /// Caret position in **chars** (`0..=chars`). Char-indexed rather than
    /// byte-indexed so masked and revealed geometry share one caret: char `n`
    /// is bullet `n` is value-char `n`.
    cursor: usize,
    masked: bool,
    placeholder: String,
    disabled: bool,
    scroll: f32,
    rect: Rect,
    /// x-table over the *display* string — bullets while masked, so the value
    /// stays out of the shaping path until the host reveals it.
    cache: LineCache,
    /// The pending [`TextCommit`], drained by
    /// [`take_commit`](SecretInput::take_commit).
    commit: Option<TextCommit>,
}

impl SecretInput {
    /// An empty field, masked.
    pub fn new() -> Self {
        Self {
            id: next_id(),
            value: Zeroizing::new(String::new()),
            cursor: 0,
            masked: true,
            placeholder: String::new(),
            disabled: false,
            scroll: 0.0,
            rect: Rect::ZERO,
            cache: LineCache::default(),
            commit: None,
        }
    }

    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = text.into();
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn id(&self) -> WidgetId {
        self.id
    }

    /// Whether the field renders bullets (`true`, the initial state) or the
    /// value.
    pub fn masked(&self) -> bool {
        self.masked
    }

    /// The host's reveal gate (a "show password" toggle). The buffer is
    /// untouched — only how it renders. Revealing does put the value through
    /// the ordinary text draw path (and the shaping cache) like any label.
    pub fn set_masked(&mut self, masked: bool) {
        self.masked = masked;
    }

    /// A copy of the value, itself wiped on drop. The only read-back — there
    /// is deliberately no borrowed `&str` accessor inviting a plain
    /// `to_string`.
    #[must_use]
    pub fn value(&self) -> Zeroizing<String> {
        Zeroizing::new(self.value.as_str().to_owned())
    }

    /// Replaces the value (caret to the end). The old value is overwritten in
    /// place, not dropped unwiped — though a longer replacement can still
    /// grow the `String` (the module-doc realloc caveat).
    pub fn set_value(&mut self, text: &str) {
        self.value.clear();
        self.value.push_str(text);
        self.cursor = self.value.chars().count();
    }

    /// Takes the pending [`TextCommit`] — Enter, or focus leaving the field —
    /// reporting it **once**; the [`TextInput::take_commit`] contract.
    ///
    /// [`TextInput::take_commit`]: crate::TextInput::take_commit
    pub fn take_commit(&mut self) -> Option<TextCommit> {
        self.commit.take()
    }

    fn chars(&self) -> usize {
        self.value.chars().count()
    }

    /// The caret's byte offset in the display string the cache was built on.
    fn display_byte(&self) -> usize {
        if self.masked {
            self.cursor * BULLET.len()
        } else {
            nth_char_byte(&self.value, self.cursor)
        }
    }

    /// Display-string byte (a cache boundary) back to a char index — the
    /// inverse of [`display_byte`](Self::display_byte) for mouse hits.
    fn char_at_display_byte(&self, byte: usize) -> usize {
        if self.masked {
            byte / BULLET.len()
        } else {
            self.value[..byte.min(self.value.len())].chars().count()
        }
    }

    fn insert_char(&mut self, c: char) {
        let at = nth_char_byte(&self.value, self.cursor);
        self.value.insert(at, c);
        self.cursor += 1;
    }

    fn backspace(&mut self) {
        if self.cursor > 0 {
            let at = nth_char_byte(&self.value, self.cursor - 1);
            self.value.remove(at);
            self.cursor -= 1;
        }
    }

    fn delete_forward(&mut self) {
        if self.cursor < self.chars() {
            let at = nth_char_byte(&self.value, self.cursor);
            self.value.remove(at);
        }
    }

    fn inner(&self) -> Rect {
        Rect::new(
            self.rect.x + PAD_X,
            self.rect.y,
            (self.rect.w - 2.0 * PAD_X).max(0.0),
            self.rect.h,
        )
    }

    fn ensure_caret_visible(&mut self) {
        let inner_w = self.inner().w;
        let cx = self.cache.x_of(self.display_byte());
        if cx - self.scroll > inner_w {
            self.scroll = cx - inner_w;
        }
        if cx - self.scroll < 0.0 {
            self.scroll = cx;
        }
        self.scroll = self
            .scroll
            .clamp(0.0, (self.cache.width() - inner_w).max(0.0));
    }
}

impl Default for SecretInput {
    fn default() -> Self {
        Self::new()
    }
}

/// Hand-written so the value can never leak into logs, panics, or test output
/// — redacted even while revealed (revealing is a *render* choice).
impl std::fmt::Debug for SecretInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretInput")
            .field("value", &"<secret>")
            .field("cursor", &self.cursor)
            .field("masked", &self.masked)
            .finish()
    }
}

impl Widget for SecretInput {
    // The label is the placeholder, never the value — the TextInput rule,
    // doubly so here.
    fn semantics(&self) -> Semantics<'_> {
        Semantics::labeled(kind_of::<Self>(), &self.placeholder)
    }

    fn cursor(&self, _pos: Vec2) -> CursorIcon {
        if self.disabled {
            CursorIcon::Default
        } else {
            CursorIcon::Text
        }
    }

    fn measure(&mut self, _avail: Size, ctx: &mut LayoutCtx) -> Size {
        let m = ctx.theme.metrics();
        Size::new(m.button_min_width * 2.0, m.control_height)
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        if self.masked {
            let display = BULLET.repeat(self.chars());
            self.cache = LineCache::build(
                &display,
                0,
                display.len(),
                ctx.fonts,
                ctx.theme,
                TextRole::Body,
            );
        } else {
            let len = self.value.len();
            self.cache =
                LineCache::build(&self.value, 0, len, ctx.fonts, ctx.theme, TextRole::Body);
        }
        self.ensure_caret_visible();
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if !ctx.is_base() {
            return;
        }
        let focused = ctx.is_focused(self.id);
        ctx.theme.well(
            dl,
            self.rect,
            WidgetState {
                focused,
                disabled: self.disabled,
                ..Default::default()
            },
        );
        let inner = self.inner();
        dl.push_clip(inner);
        let base_x = inner.x - self.scroll;
        let px = ctx.theme.font_px(TextRole::Body);
        let baseline = Vec2::new(base_x, inner.center().y + px * 0.34);

        if self.value.is_empty() && !focused && !self.placeholder.is_empty() {
            ctx.theme
                .text_placeholder(dl, ctx.fonts, baseline, &self.placeholder, TextRole::Body);
        } else if self.masked {
            let display = BULLET.repeat(self.chars());
            ctx.theme
                .text(dl, ctx.fonts, baseline, &display, TextRole::Body);
        } else {
            ctx.theme
                .text(dl, ctx.fonts, baseline, &self.value, TextRole::Body);
        }

        if focused {
            let cx = base_x + self.cache.x_of(self.display_byte());
            dl.fill_rect(
                Rect::new(cx, inner.y + PAD_Y, CARET_W, inner.h - 2.0 * PAD_Y),
                ctx.theme.accent(),
            );
        }
        dl.pop_clip();
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        if self.disabled {
            return false;
        }
        match ev {
            // A click focuses and places the caret. No drag, no double-click
            // word-take: there is no selection to make (see the module docs).
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                ..
            } if ctx.is_target(self.id) => {
                ctx.consume_pointer();
                ctx.request_focus(self.id);
                let local = ctx.pointer.x - self.inner().x + self.scroll;
                self.cursor = self.char_at_display_byte(self.cache.byte_at(local));
                true
            }
            Event::Key {
                key,
                pressed: true,
                mods,
                ..
            } if ctx.is_target(self.id) => {
                match key {
                    Key::Left => self.cursor = self.cursor.saturating_sub(1),
                    Key::Right => self.cursor = (self.cursor + 1).min(self.chars()),
                    Key::Home => self.cursor = 0,
                    Key::End => self.cursor = self.chars(),
                    Key::Backspace => self.backspace(),
                    Key::Delete => self.delete_forward(),
                    // Select-all/copy/cut are deliberate no-ops — consumed so
                    // the chords never leak to a host binding while focused,
                    // and so nothing can put the value on the clipboard.
                    // Ctrl+V is NOT consumed: the host must see it to answer
                    // with an [`Event::Paste`].
                    Key::Character('a' | 'A' | 'c' | 'C' | 'x' | 'X') if mods.ctrl => {}
                    // Enter commits and fires, the TextInput contract.
                    Key::Enter => {
                        self.commit = Some(TextCommit::Enter);
                        ctx.fire(self.id, None);
                    }
                    _ => return false,
                }
                ctx.consume_keyboard();
                true
            }
            // Typed and pasted text insert at the caret; control characters
            // are skipped (which also strips a paste's newlines). No charset,
            // no length cap — passwords are arbitrary.
            Event::Text(s) | Event::Paste(s) if ctx.is_target(self.id) => {
                for ch in s.chars().filter(|c| !c.is_control()) {
                    self.insert_char(ch);
                }
                ctx.consume_keyboard();
                true
            }
            // A composition must not run over a secret (module docs); if a
            // host enables the IME anyway, its preedit is dropped here —
            // committed text still arrives as `Event::Text` above.
            Event::ImePreedit { .. } if ctx.is_target(self.id) => {
                ctx.consume_keyboard();
                true
            }
            // Focus left: the edit stands unless the user backed out with
            // Escape — the TextInput contract.
            Event::Blur(cause) if ctx.is_target(self.id) => {
                if *cause == BlurCause::Moved {
                    self.commit = Some(TextCommit::FocusOut);
                }
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

    fn accepts_focus(&self) -> bool {
        true
    }

    // `false` — not because the field refuses text (it takes `Event::Text`
    // like any field) but because this is what a host mirrors into
    // `set_ime_allowed`, and the OS IME must stay off over a secret. Plain
    // typing and dead-key compose don't need the IME to reach `Event::Text`.
    fn accepts_text_input(&self) -> bool {
        false
    }
}
