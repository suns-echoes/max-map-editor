//! Editable text: a shared [`Edit`] core (UTF-8-safe cursor/selection/editing)
//! plus a single-line [`TextInput`] and a multi-line [`TextArea`].
//!
//! Cursor and anchor are **byte offsets** kept on char boundaries. Mouse hit-
//! testing and caret/selection geometry use a per-line x-table rebuilt during
//! `arrange` (which has fonts/theme); `event` reads that cache, so it needs no
//! fonts of its own.

use crate::color::Rgba;
use crate::draw::DrawList;
use crate::event::{BlurCause, Event, Key, PointerButton};
use crate::geom::{Rect, Size, Vec2};
use crate::interact::{CursorIcon, WidgetId, WidgetState, next_id};
use crate::text::Fonts;
use crate::theme::{TextRole, Theme};
use crate::widget::{DrawCtx, EventCtx, LayoutCtx, Semantics, Widget, kind_of};

pub(crate) const PAD_X: f32 = 6.0;
pub(crate) const PAD_Y: f32 = 3.0;
pub(crate) const CARET_W: f32 = 1.5;
/// Gap between a [`TextArea`]'s wrapped text and its scrollbar column.
const BAR_GAP: f32 = 4.0;

/// The byte offset of the char before `i` (or 0).
fn prev_boundary(s: &str, i: usize) -> usize {
    i - s[..i].chars().next_back().map_or(0, char::len_utf8)
}

/// The byte offset of the char after the one at `i` (or `s.len()`).
fn next_boundary(s: &str, i: usize) -> usize {
    i + s[i..].chars().next().map_or(0, char::len_utf8)
}

/// Which run a character belongs to for word selection: `0` word (alphanumeric
/// or `_`), `1` space, `2` everything else, `3` a line break. Adjacent
/// characters of one class are one "word" — see [`Edit::select_word_at`]. The
/// break gets a class of its own so no run ever spans two lines: trailing spaces
/// belong to the line they are on.
fn char_class(c: char) -> u8 {
    if c == '\n' {
        3
    } else if c.is_alphanumeric() || c == '_' {
        0
    } else if c.is_whitespace() {
        1
    } else {
        2
    }
}

/// Horizontal alignment of text within the box that holds it — a
/// [`TextInput`]'s field or a [`Label`](crate::widgets::Label)'s arranged rect.
/// Applies while the text fits; once it overflows there is no slack to
/// distribute (the field scrolls to the caret, the label ellipsizes or clips).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

impl TextAlign {
    /// The fraction of the free width placed left of the text.
    pub(crate) fn factor(self) -> f32 {
        match self {
            TextAlign::Left => 0.0,
            TextAlign::Center => 0.5,
            TextAlign::Right => 1.0,
        }
    }
}

/// An input filter for [`TextInput`]: restricts which characters may be typed
/// (and pasted). Validation is on the *partial* entry, so intermediate states
/// like `""`, `"-"`, and `"1."` are allowed while typing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Charset {
    /// Any non-control character (the default).
    #[default]
    Text,
    /// ASCII digits only.
    Digits,
    /// An optional leading `-` then digits.
    SignedInt,
    /// An optional leading `-`, digits, and at most one `.`.
    Decimal,
    /// An identifier: alphanumerics and `_`, not starting with a digit.
    Identifier,
    /// A slug: an [`Self::Identifier`] that may also contain `-`. What an asset
    /// id looks like when it comes from a file name (`oak-stand-3`) rather than
    /// from source code.
    Slug,
}

impl Charset {
    /// Whether `s` is a valid (possibly partial) entry for this charset.
    pub fn accepts(self, s: &str) -> bool {
        match self {
            Charset::Text => true,
            Charset::Digits => s.chars().all(|c| c.is_ascii_digit()),
            Charset::SignedInt => {
                let body = s.strip_prefix('-').unwrap_or(s);
                body.chars().all(|c| c.is_ascii_digit())
            }
            Charset::Decimal => {
                let body = s.strip_prefix('-').unwrap_or(s);
                body.matches('.').count() <= 1
                    && body.chars().all(|c| c.is_ascii_digit() || c == '.')
            }
            Charset::Identifier | Charset::Slug => {
                let dash = self == Charset::Slug;
                let body = |c: char| c.is_ascii_alphanumeric() || c == '_' || (dash && c == '-');
                let mut chars = s.chars();
                match chars.next() {
                    None => true,
                    Some(first) if first.is_ascii_alphabetic() || first == '_' => chars.all(body),
                    Some(_) => false,
                }
            }
        }
    }
}

/// A text buffer with a cursor and a selection anchor (both byte offsets).
#[derive(Clone, Debug, Default)]
pub struct Edit {
    text: String,
    cursor: usize,
    anchor: usize,
}

impl Edit {
    fn new(text: String) -> Self {
        let cursor = text.len();
        Self {
            text,
            cursor,
            anchor: cursor,
        }
    }

    fn selection(&self) -> (usize, usize) {
        (self.cursor.min(self.anchor), self.cursor.max(self.anchor))
    }

    fn has_selection(&self) -> bool {
        self.cursor != self.anchor
    }

    fn set_cursor(&mut self, byte: usize, extend: bool) {
        self.cursor = byte.min(self.text.len());
        if !extend {
            self.anchor = self.cursor;
        }
    }

    fn delete_selection(&mut self) {
        let (a, b) = self.selection();
        self.text.replace_range(a..b, "");
        self.cursor = a;
        self.anchor = a;
    }

    fn insert(&mut self, s: &str) {
        if self.has_selection() {
            self.delete_selection();
        }
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
        self.anchor = self.cursor;
    }

    /// Inserts `s` character by character, skipping any char that would make the
    /// resulting text fail `accept` or exceed `max_chars` (counted in chars).
    /// Control characters are always skipped. Each char is validated against the
    /// would-be full text, so context-sensitive charsets (sign placement, a
    /// single decimal point) filter correctly mid-string.
    fn insert_filtered(
        &mut self,
        s: &str,
        accept: impl Fn(&str) -> bool,
        max_chars: Option<usize>,
    ) {
        if self.has_selection() {
            self.delete_selection();
        }
        // Both of the per-character costs here used to be O(len): cloning the
        // whole buffer to build the candidate, and recounting `chars()` against
        // the cap. Over an n-character insert that is O(n^2) — typing never
        // noticed (one char per event), but a paste of a large clipboard payload
        // stalled the frame. The candidate is now built in place and undone on
        // reject, which is the same string the charset was always asked about,
        // and the count is carried.
        let mut chars_now = if max_chars.is_some() {
            self.text.chars().count()
        } else {
            0
        };
        for ch in s.chars() {
            if ch.is_control() {
                continue;
            }
            if max_chars.is_some_and(|max| chars_now >= max) {
                break;
            }
            self.text.insert(self.cursor, ch);
            if accept(&self.text) {
                self.cursor += ch.len_utf8();
                self.anchor = self.cursor;
                chars_now += 1;
            } else {
                self.text.remove(self.cursor);
            }
        }
    }

    fn backspace(&mut self) {
        if self.has_selection() {
            self.delete_selection();
        } else if self.cursor > 0 {
            let p = prev_boundary(&self.text, self.cursor);
            self.text.replace_range(p..self.cursor, "");
            self.cursor = p;
            self.anchor = p;
        }
    }

    fn delete_forward(&mut self) {
        if self.has_selection() {
            self.delete_selection();
        } else if self.cursor < self.text.len() {
            let n = next_boundary(&self.text, self.cursor);
            self.text.replace_range(self.cursor..n, "");
        }
    }

    fn move_left(&mut self, extend: bool) {
        self.set_cursor(prev_boundary(&self.text, self.cursor), extend);
    }

    fn move_right(&mut self, extend: bool) {
        self.set_cursor(next_boundary(&self.text, self.cursor), extend);
    }

    fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.text.len();
    }

    /// The run of like characters around `byte` — the "word" a double click
    /// takes, and the unit a double-click *drag* extends by. "Like" is a
    /// [`char_class`]: word characters (alphanumeric or `_`), spaces, everything
    /// else — so `foo_bar(baz)` gives `foo_bar` from inside the identifier, `(`
    /// from the paren, and the spaces from inside a run of them. Selecting by
    /// *class* rather than by "not whitespace" is what stops a double click on an
    /// identifier from swallowing the punctuation beside it.
    ///
    /// `None` where there is no run to take: empty text, or the very start of a
    /// blank line.
    fn word_range_at(&self, byte: usize) -> Option<(usize, usize)> {
        let byte = byte.min(self.text.len());
        // A click past the last character — or on a line break, which is a
        // boundary rather than something anyone means to select — takes the run
        // that *ends* there. Without this, double-clicking past the end of a
        // line would select the newline, and typing would join the two lines.
        let ends_here = byte == self.text.len() || self.text[byte..].starts_with('\n');
        let at = if ends_here {
            prev_boundary(&self.text, byte)
        } else {
            byte
        };
        if ends_here && at == byte {
            return None; // nothing before the click either
        }
        let class = self.text[at..].chars().next().map(char_class)?;
        let start = self.text[..at]
            .char_indices()
            .rev()
            .take_while(|(_, c)| char_class(*c) == class)
            .last()
            .map_or(at, |(i, _)| i);
        let end = self.text[at..]
            .char_indices()
            .take_while(|(_, c)| char_class(*c) == class)
            .last()
            .map_or(at, |(i, c)| at + i + c.len_utf8());
        Some((start, end))
    }

    /// Selects the word around `byte` (see [`word_range_at`](Self::word_range_at)),
    /// caret at its end so a following Shift+click extends from the far edge the
    /// way a drag would. With no word there, the selection just collapses.
    fn select_word_at(&mut self, byte: usize) {
        match self.word_range_at(byte) {
            Some((start, end)) => {
                self.anchor = start;
                self.cursor = end;
            }
            None => self.set_cursor(byte.min(self.text.len()), false),
        }
    }

    /// Extends a **word-granularity** drag: the selection covers the anchored
    /// word plus the word under `byte`, whole. This is what keeps a double click
    /// followed by a drag from collapsing — the pointer wobbling inside the word
    /// it started on re-selects that same word, where a per-character extend
    /// would cut it back to wherever the pointer happened to land.
    fn extend_word(&mut self, anchor: (usize, usize), byte: usize) {
        let (s, e) = self.word_range_at(byte).unwrap_or((byte, byte));
        if s < anchor.0 {
            // Dragging backwards: the caret leads at the far word's start.
            self.anchor = anchor.1;
            self.cursor = s;
        } else {
            self.anchor = anchor.0;
            self.cursor = e.max(anchor.1);
        }
    }

    /// Selects `start..end` with the caret at `end` — the row a triple click
    /// takes. The *widget* resolves which row that is: a single-line field's is
    /// its whole text, a [`TextArea`]'s is the one its own Home/End move within.
    fn select_range(&mut self, start: usize, end: usize) {
        self.anchor = start.min(self.text.len());
        self.cursor = end.min(self.text.len());
    }

    fn selected_text(&self) -> &str {
        let (a, b) = self.selection();
        &self.text[a..b]
    }
}

/// A cached x-table for one rendered line: byte boundaries and their x offsets
/// (relative to the line's left edge). Built from the shaped line's CLUSTERS —
/// a caret can only land on cluster edges, so a ligature ("fi" as one glyph)
/// contributes no interior boundary. For the hand-rolled backend clusters are
/// per-char and each boundary x is the prefix measure, byte-identical to the
/// pre-seam table.
#[derive(Clone, Debug, Default)]
pub(crate) struct LineCache {
    start: usize,
    end: usize,
    bytes: Vec<usize>,
    xs: Vec<f32>,
}

impl LineCache {
    pub(crate) fn build(
        text: &str,
        start: usize,
        end: usize,
        fonts: &Fonts,
        theme: &dyn Theme,
        role: TextRole,
    ) -> Self {
        let px = theme.font_px(role);
        let line = &text[start..end];
        let shaped = fonts.shape(theme.font_for(role), line, px);
        let mut bytes = vec![start];
        let mut xs = vec![0.0];
        for (byte, x) in shaped.boundaries() {
            // Guard against non-monotone byte order (bidi reordering): the
            // table is a byte-ascending map, so keep only edges that advance.
            if start + byte > *bytes.last().expect("seeded") {
                bytes.push(start + byte);
                xs.push(x);
            }
        }
        Self {
            start,
            end,
            bytes,
            xs,
        }
    }

    pub(crate) fn x_of(&self, byte: usize) -> f32 {
        match self.bytes.binary_search(&byte) {
            Ok(i) => self.xs[i],
            Err(i) => self.xs[i.saturating_sub(1).min(self.xs.len() - 1)],
        }
    }

    /// Byte offset nearest local x (0 at the line's left edge).
    pub(crate) fn byte_at(&self, local: f32) -> usize {
        for i in 0..self.xs.len() {
            let mid = if i + 1 < self.xs.len() {
                (self.xs[i] + self.xs[i + 1]) * 0.5
            } else {
                f32::INFINITY
            };
            if local < mid {
                return self.bytes[i];
            }
        }
        *self.bytes.last().unwrap_or(&self.start)
    }

    pub(crate) fn width(&self) -> f32 {
        *self.xs.last().unwrap_or(&0.0)
    }
}

fn selection_fill(theme: &dyn Theme) -> Rgba {
    theme.accent().with_alpha(70)
}

// --- TextInput (single line) ------------------------------------------------

/// Why a field's value should be read back — the poll a host does instead of
/// re-deriving "the user finished with this box" from raw events.
///
/// Both mean *the edit stands*; they differ only in what the user did. A host
/// that treats them alike (the common case) can ignore the payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextCommit {
    /// Enter was pressed in the field. A single-line field also fires
    /// ([`Ui::fired`](crate::ui::Ui::fired)) for this, so a dialog polling the
    /// fire channel keeps working unchanged.
    Enter,
    /// Keyboard focus left the field for [`BlurCause::Moved`] — a click
    /// elsewhere, Tab, or the host handing the keyboard to another tree. An
    /// Escape blur ([`BlurCause::Cancelled`]) is **not** a commit.
    FocusOut,
}

/// A single-line editable text field.
#[must_use]
pub struct TextInput {
    id: WidgetId,
    edit: Edit,
    placeholder: String,
    disabled: bool,
    charset: Charset,
    max_len: Option<usize>,
    align: TextAlign,
    /// The text face/size this field measures and draws with (see
    /// [`TextInput::role`]).
    role: TextRole,
    /// Skip the [`Theme::well`] backing (see [`TextInput::frameless`]).
    frameless: bool,
    dragging: bool,
    /// While a **double-click** drag is live: the word it started on, so the
    /// extend stays word-granular (see [`Edit::extend_word`]). `None` for a
    /// plain character drag.
    drag_word: Option<(usize, usize)>,
    scroll: f32,
    rect: Rect,
    cache: LineCache,
    /// The pending [`TextCommit`], drained by
    /// [`take_commit`](TextInput::take_commit).
    commit: Option<TextCommit>,
    /// In-progress IME composition, shown inline at the caret (not committed).
    preedit: String,
    /// Caret byte offset within `preedit`.
    preedit_cursor: usize,
    /// Rendered width of `preedit` / of its text before the composition caret
    /// (both 0 while not composing) — measured at arrange like the x-table.
    preedit_w: f32,
    preedit_caret_w: f32,
}

impl TextInput {
    pub fn new() -> Self {
        Self::with_text(String::new())
    }

    pub fn with_text(text: impl Into<String>) -> Self {
        Self {
            id: next_id(),
            edit: Edit::new(text.into()),
            placeholder: String::new(),
            disabled: false,
            charset: Charset::Text,
            max_len: None,
            align: TextAlign::Left,
            role: TextRole::Body,
            frameless: false,
            dragging: false,
            drag_word: None,
            scroll: 0.0,
            rect: Rect::ZERO,
            cache: LineCache::default(),
            commit: None,
            preedit: String::new(),
            preedit_cursor: 0,
            preedit_w: 0.0,
            preedit_caret_w: 0.0,
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

    /// Restricts which characters may be typed/pasted (digits, signed, decimal,
    /// identifier, or unrestricted text). Validation is on the partial entry, so
    /// the field can still be typed into one key at a time.
    pub fn charset(mut self, charset: Charset) -> Self {
        self.charset = charset;
        self
    }

    /// Caps the field at `max` characters (typed or pasted; programmatic
    /// `set_text` is not truncated).
    pub fn max_len(mut self, max: usize) -> Self {
        self.max_len = Some(max);
        self
    }

    /// Aligns the text within the field (left, center, or right) while it
    /// fits; an overflowing field scrolls to the caret as usual.
    pub fn align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    /// The text face/size to measure and draw with. [`TextRole::Mono`] makes an
    /// ordinary field a **terminal** field — fixed pitch, so columns line up
    /// with mono text around it — while every behavior (caret, drag-select,
    /// charset, IME) stays the field's. Default [`TextRole::Body`].
    pub fn role(mut self, role: TextRole) -> Self {
        self.role = role;
        self
    }

    /// Drops the [`Theme::well`] backing **and its inner padding**: the text,
    /// selection and caret draw straight onto whatever is behind, starting at
    /// the field's own left edge. For a field that is not visually a box — a
    /// console prompt line, an in-place rename over a list row — where the
    /// padding would push the text out of alignment with the plain text around
    /// it (a terminal's columns have to line up).
    pub fn frameless(mut self, frameless: bool) -> Self {
        self.frameless = frameless;
        self
    }

    pub fn id(&self) -> WidgetId {
        self.id
    }

    pub fn text(&self) -> &str {
        &self.edit.text
    }

    /// Byte offset of the text caret into [`text`](Self::text) (always on a char
    /// boundary). For a host that draws its own caret/overlay over the field
    /// instead of the field's own render — e.g. a monospace console reusing its
    /// own glyph atlas. (Distinct from the [`Widget::cursor`] mouse icon.)
    pub fn caret(&self) -> usize {
        self.edit.cursor
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.edit = Edit::new(text.into());
    }

    /// Takes the pending [`TextCommit`] — Enter, or focus leaving the field —
    /// reporting it **once**. Read the value itself with [`text`](Self::text).
    ///
    /// This is the whole "the user finished editing" contract, and it is the
    /// widget's, not the host's: a host cannot see focus move away from one of
    /// its fields (a click in another tree never reaches this one), so anything
    /// re-derived from raw events misses cases. Poll it after every dispatch,
    /// the way a [`Select`](crate::Select)'s pick is polled.
    pub fn take_commit(&mut self) -> Option<TextCommit> {
        self.commit.take()
    }

    fn inner(&self) -> Rect {
        let pad = if self.frameless { 0.0 } else { PAD_X };
        Rect::new(
            self.rect.x + pad,
            self.rect.y,
            (self.rect.w - 2.0 * pad).max(0.0),
            self.rect.h,
        )
    }

    fn ensure_caret_visible(&mut self) {
        let inner_w = self.inner().w;
        let cx = self.cache.x_of(self.edit.cursor) + self.preedit_caret_w;
        if cx - self.scroll > inner_w {
            self.scroll = cx - inner_w;
        }
        if cx - self.scroll < 0.0 {
            self.scroll = cx;
        }
        self.scroll = self.scroll.clamp(
            0.0,
            (self.cache.width() + self.preedit_w - inner_w).max(0.0),
        );
    }

    /// The slack placed left of the text by the alignment — zero when the text
    /// overflows (scrolling owns the x-origin then).
    fn align_pad(&self) -> f32 {
        let free = self.inner().w - self.cache.width() - self.preedit_w;
        free.max(0.0) * self.align.factor()
    }
}

impl Default for TextInput {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for TextInput {
    // The label is the placeholder, never the value: a field's content is
    // data (and possibly secret), not what the field is called.
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
        let len = self.edit.text.len();
        self.cache = LineCache::build(&self.edit.text, 0, len, ctx.fonts, ctx.theme, self.role);
        if self.preedit.is_empty() {
            self.preedit_w = 0.0;
            self.preedit_caret_w = 0.0;
        } else {
            let font = ctx.theme.font_for(self.role);
            let px = ctx.theme.font_px(self.role);
            self.preedit_w = ctx.fonts.measure(font, &self.preedit, px);
            self.preedit_caret_w =
                ctx.fonts
                    .measure(font, &self.preedit[..self.preedit_cursor], px);
        }
        self.ensure_caret_visible();
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if !ctx.is_base() {
            return;
        }
        let focused = ctx.is_focused(self.id);
        if !self.frameless {
            ctx.theme.well(
                dl,
                self.rect,
                WidgetState {
                    focused,
                    disabled: self.disabled,
                    ..Default::default()
                },
            );
        }
        let inner = self.inner();
        dl.push_clip(inner);
        let base_x = inner.x + self.align_pad() - self.scroll;
        let px = ctx.theme.font_px(self.role);
        let baseline = Vec2::new(base_x, inner.center().y + px * 0.34);

        if self.edit.has_selection() {
            let (a, b) = self.edit.selection();
            let ax = self.cache.x_of(a);
            let bx = self.cache.x_of(b);
            dl.fill_rect(
                Rect::new(base_x + ax, inner.y + PAD_Y, bx - ax, inner.h - 2.0 * PAD_Y),
                selection_fill(ctx.theme),
            );
        }

        if focused && !self.preedit.is_empty() {
            // Composing: the committed text splits at the caret and the
            // preedit run sits between, underlined (the IME convention).
            let pre_x = base_x + self.cache.x_of(self.edit.cursor);
            let before = &self.edit.text[..self.edit.cursor];
            let after = &self.edit.text[self.edit.cursor..];
            ctx.theme.text(dl, ctx.fonts, baseline, before, self.role);
            ctx.theme.text(
                dl,
                ctx.fonts,
                Vec2::new(pre_x, baseline.y),
                &self.preedit,
                self.role,
            );
            ctx.theme.text(
                dl,
                ctx.fonts,
                Vec2::new(pre_x + self.preedit_w, baseline.y),
                after,
                self.role,
            );
            dl.fill_rect(
                Rect::new(pre_x, baseline.y + 2.0, self.preedit_w, 1.0),
                ctx.theme.ink_dim(),
            );
        } else if self.edit.text.is_empty() && !focused && !self.placeholder.is_empty() {
            ctx.theme
                .text_placeholder(dl, ctx.fonts, baseline, &self.placeholder, self.role);
        } else {
            ctx.theme
                .text(dl, ctx.fonts, baseline, &self.edit.text, self.role);
        }

        if focused {
            // The caret tracks the composition cursor while composing
            // (`preedit_caret_w` is 0 otherwise).
            let cx = base_x + self.cache.x_of(self.edit.cursor) + self.preedit_caret_w;
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
        let local_x = |x: f32, this: &Self| x - this.inner().x - this.align_pad() + this.scroll;
        match ev {
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                mods,
                ..
            } if ctx.is_target(self.id) => {
                ctx.consume_pointer();
                ctx.request_focus(self.id);
                let byte = self.cache.byte_at(local_x(ctx.pointer.x, self));
                // A double click takes the word under it, a triple the whole
                // field (a single-line field's "line" is all of it). Neither
                // starts a drag: the pointer is about to move a little while the
                // user lets go, and re-tracking it would collapse the selection
                // that was just made. Shift is an explicit "extend from here",
                // so it stays a plain caret move however fast it was clicked.
                match ctx.clicks {
                    2 if !mods.shift => {
                        self.drag_word = self.edit.word_range_at(byte);
                        self.edit.select_word_at(byte);
                        self.dragging = true;
                        ctx.capture(self.id);
                    }
                    n if n >= 3 && !mods.shift => self.edit.select_all(),
                    _ => {
                        self.edit.set_cursor(byte, mods.shift);
                        self.dragging = true;
                        ctx.capture(self.id);
                    }
                }
                true
            }
            Event::PointerMoved { .. } if self.dragging && ctx.is_target(self.id) => {
                let byte = self.cache.byte_at(local_x(ctx.pointer.x, self));
                match self.drag_word {
                    Some(word) => self.edit.extend_word(word, byte),
                    None => self.edit.set_cursor(byte, true),
                }
                ctx.consume_pointer();
                true
            }
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: false,
                ..
            } if self.dragging => {
                self.dragging = false;
                self.drag_word = None;
                ctx.consume_pointer();
                true
            }
            Event::Key {
                key,
                pressed: true,
                mods,
                ..
            } if ctx.is_target(self.id) => {
                let ext = mods.shift;
                match key {
                    Key::Left => self.edit.move_left(ext),
                    Key::Right => self.edit.move_right(ext),
                    Key::Home => self.edit.set_cursor(0, ext),
                    Key::End => {
                        let len = self.edit.text.len();
                        self.edit.set_cursor(len, ext);
                    }
                    Key::Backspace => self.edit.backspace(),
                    Key::Delete => self.edit.delete_forward(),
                    Key::Character('a' | 'A') if mods.ctrl => self.edit.select_all(),
                    // Copy/cut hand the selection to the host's OS clipboard
                    // (`Ui::take_clipboard`); consumed even selectionless so
                    // the chord never leaks to a host binding while focused.
                    Key::Character('c' | 'C') if mods.ctrl => {
                        if self.edit.has_selection() {
                            ctx.set_clipboard(self.edit.selected_text().to_string());
                        }
                    }
                    Key::Character('x' | 'X') if mods.ctrl => {
                        if self.edit.has_selection() {
                            ctx.set_clipboard(self.edit.selected_text().to_string());
                            self.edit.delete_selection();
                        }
                    }
                    // Enter commits: a single-line field has nowhere to put the
                    // newline, so it fires (like a button activating) — the host
                    // polls `Ui::fired(id)` to read the value / submit the form,
                    // or `take_commit` to treat it as one of the two ways an edit
                    // ends. Consumed so the chord never leaks to a host binding
                    // while a field is focused.
                    Key::Enter => {
                        self.commit = Some(TextCommit::Enter);
                        ctx.fire(self.id, None);
                    }
                    _ => return false,
                }
                ctx.consume_keyboard();
                true
            }
            // Pasted text goes through the same charset/length filter as typed
            // text (which also drops a paste's newlines — control characters).
            // Committed text ends any IME composition (the IME's clearing
            // empty preedit may arrive after the commit).
            Event::Text(s) | Event::Paste(s) if ctx.is_target(self.id) => {
                self.clear_preedit();
                let charset = self.charset;
                self.edit
                    .insert_filtered(s, |t| charset.accepts(t), self.max_len);
                ctx.consume_keyboard();
                true
            }
            Event::ImePreedit { text, cursor } if ctx.is_target(self.id) => {
                self.set_preedit(text, *cursor);
                ctx.consume_keyboard();
                true
            }
            // Focus left: the edit stands (the user moved on) unless they backed
            // out with Escape. An in-progress IME composition is not committed
            // text, so it goes with the focus.
            Event::Blur(cause) if ctx.is_target(self.id) => {
                self.clear_preedit();
                self.dragging = false;
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

    fn accepts_text_input(&self) -> bool {
        !self.disabled
    }

    fn ime_rect(&self) -> Option<Rect> {
        if self.disabled {
            return None;
        }
        let inner = self.inner();
        let cx = inner.x + self.align_pad() - self.scroll
            + self.cache.x_of(self.edit.cursor)
            + self.preedit_caret_w;
        Some(Rect::new(
            cx,
            inner.y + PAD_Y,
            CARET_W,
            inner.h - 2.0 * PAD_Y,
        ))
    }
}

impl TextInput {
    fn clear_preedit(&mut self) {
        self.preedit.clear();
        self.preedit_cursor = 0;
        self.preedit_w = 0.0;
        self.preedit_caret_w = 0.0;
    }

    fn set_preedit(&mut self, text: &str, cursor: Option<(usize, usize)>) {
        self.preedit = text.to_string();
        self.preedit_cursor = preedit_caret(text, cursor);
    }
}

/// The caret byte offset within a preedit string: the highlight range's end,
/// clamped onto a char boundary (defensive — hosts pass byte offsets).
fn preedit_caret(text: &str, cursor: Option<(usize, usize)>) -> usize {
    let mut c = cursor.map_or(text.len(), |(_, end)| end).min(text.len());
    while c > 0 && !text.is_char_boundary(c) {
        c -= 1;
    }
    c
}

// --- TextArea (multi-line) --------------------------------------------------

/// The `(start, end)` byte ranges of each `\n`-delimited line (end excludes the
/// newline).
fn line_bounds(text: &str) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    let mut start = 0;
    for (i, ch) in text.char_indices() {
        if ch == '\n' {
            v.push((start, i));
            start = i + 1;
        }
    }
    v.push((start, text.len()));
    v
}

fn char_count(s: &str) -> usize {
    s.chars().count()
}

/// Byte offset of the `n`-th char of `s` (clamped to `s.len()`).
pub(crate) fn nth_char_byte(s: &str, n: usize) -> usize {
    s.char_indices().nth(n).map_or(s.len(), |(i, _)| i)
}

/// Soft-wraps `text` into visual rows of at most `max_w` px: each logical
/// (`\n`-delimited) line breaks after the last whitespace that fits, or
/// mid-word at char granularity when a single word overflows the width — so
/// nothing is ever cropped at the right edge. Whitespace at a break rides
/// along at the end of the row (clipped invisibly) rather than starting the
/// next row.
fn wrap_bounds(text: &str, max_w: f32, fonts: &Fonts, theme: &dyn Theme) -> Vec<(usize, usize)> {
    let mut rows = Vec::new();
    for (ls, le) in line_bounds(text) {
        if ls == le {
            rows.push((ls, le));
            continue;
        }
        let lc = LineCache::build(text, ls, le, fonts, theme, TextRole::Body);
        let mut rs = ls;
        while rs < le {
            // Row start is always a cached boundary (a prior break or `ls`).
            let i0 = lc.bytes.binary_search(&rs).unwrap_or(0);
            let x0 = lc.xs[i0];
            // Furthest char that still fits — at least one, to guarantee
            // progress at any width.
            let mut j = i0 + 1;
            while j + 1 < lc.bytes.len() && lc.xs[j + 1] - x0 <= max_w {
                j += 1;
            }
            // Let a whitespace run overflow the row instead of leading the
            // next one.
            while j + 1 < lc.bytes.len()
                && text[lc.bytes[j]..]
                    .chars()
                    .next()
                    .is_some_and(char::is_whitespace)
            {
                j += 1;
            }
            let end = lc.bytes[j];
            if end >= le {
                rows.push((rs, le));
                break;
            }
            // Back up to the last break opportunity; none → hard char break.
            let cut = text[rs..end]
                .char_indices()
                .rev()
                .find(|&(_, c)| c.is_whitespace())
                .map(|(i, c)| rs + i + c.len_utf8());
            let next = match cut {
                Some(c) if c > rs => c,
                _ => end,
            };
            rows.push((rs, next));
            rs = next;
        }
    }
    rows
}

/// The row index holding `cursor`. At a soft-wrap boundary (a byte that both
/// ends one row and starts the next) `prefer_next` picks the later row.
fn row_of(rows: &[(usize, usize)], cursor: usize, prefer_next: bool) -> usize {
    let first = rows
        .iter()
        .position(|&(s, e)| cursor >= s && cursor <= e)
        .unwrap_or(0);
    if prefer_next && rows.get(first + 1).is_some_and(|&(s, _)| s == cursor) {
        first + 1
    } else {
        first
    }
}

/// A multi-line editable text area: soft-wraps to its width and scrolls
/// vertically, with a scrollbar once the text overflows.
#[must_use]
pub struct TextArea {
    id: WidgetId,
    edit: Edit,
    disabled: bool,
    dragging: bool,
    /// While a **double-click** drag is live: the word it started on — see
    /// [`TextInput::drag_word`].
    drag_word: Option<(usize, usize)>,
    vscroll: f32,
    line_h: f32,
    rect: Rect,
    /// Soft-wrapped visual rows (rebuilt at `arrange`).
    rows: Vec<LineCache>,
    /// Scrollbar column shown (the wrapped text overflows the height).
    has_bar: bool,
    bar_w: f32,
    /// Minimum thumb length (from the theme's metrics, refreshed at `arrange`).
    bar_min_thumb: f32,
    bar_dragging: bool,
    bar_grab: f32,
    /// Caret affinity at a soft-wrap boundary: the boundary byte ends one row
    /// and starts the next — `true` renders the caret at the next row's start
    /// (the typing flow), `false` at the previous row's end (the End key).
    caret_next_row: bool,
    /// The pending [`TextCommit`] — see [`TextArea::take_commit`].
    commit: Option<TextCommit>,
    /// In-progress IME composition (see [`TextInput`]'s twin fields).
    preedit: String,
    preedit_cursor: usize,
    preedit_w: f32,
    preedit_caret_w: f32,
}

impl TextArea {
    pub fn new() -> Self {
        Self::with_text(String::new())
    }

    pub fn with_text(text: impl Into<String>) -> Self {
        Self {
            id: next_id(),
            edit: Edit::new(text.into()),
            disabled: false,
            dragging: false,
            drag_word: None,
            vscroll: 0.0,
            line_h: 16.0,
            rect: Rect::ZERO,
            rows: Vec::new(),
            has_bar: false,
            bar_w: 8.0,
            bar_min_thumb: 24.0,
            bar_dragging: false,
            bar_grab: 0.0,
            caret_next_row: true,
            commit: None,
            preedit: String::new(),
            preedit_cursor: 0,
            preedit_w: 0.0,
            preedit_caret_w: 0.0,
        }
    }

    fn clear_preedit(&mut self) {
        self.preedit.clear();
        self.preedit_cursor = 0;
        self.preedit_w = 0.0;
        self.preedit_caret_w = 0.0;
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn id(&self) -> WidgetId {
        self.id
    }

    pub fn text(&self) -> &str {
        &self.edit.text
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.edit = Edit::new(text.into());
    }

    /// Takes the pending [`TextCommit`], reporting it once — always
    /// [`TextCommit::FocusOut`] for an area (see
    /// [`TextInput::take_commit`](TextInput::take_commit)).
    pub fn take_commit(&mut self) -> Option<TextCommit> {
        self.commit.take()
    }

    fn inner(&self) -> Rect {
        self.rect.inset(crate::geom::Insets::all(PAD_Y))
    }

    /// Width available to the wrapped text (the scrollbar column is reserved
    /// once the text overflows).
    fn text_w(&self) -> f32 {
        let inner = self.inner();
        if self.has_bar {
            (inner.w - self.bar_w - BAR_GAP).max(1.0)
        } else {
            inner.w
        }
    }

    /// Byte bounds of the visual rows, for cursor navigation. Falls back to
    /// the logical `\n` lines when the wrap cache is stale (the host dispatched
    /// an edit with no layout in between) — never navigate off a stale
    /// boundary.
    fn nav_rows(&self) -> Vec<(usize, usize)> {
        let t = &self.edit.text;
        let fresh = self.rows.first().is_some_and(|f| f.start == 0)
            && self.rows.last().is_some_and(|l| l.end == t.len())
            && self.rows.iter().all(|lc| {
                lc.start <= lc.end
                    && lc.end <= t.len()
                    && t.is_char_boundary(lc.start)
                    && t.is_char_boundary(lc.end)
            });
        if fresh {
            self.rows.iter().map(|lc| (lc.start, lc.end)).collect()
        } else {
            line_bounds(t)
        }
    }

    /// The caret's visual row in the cached wrap, affinity-aware (see
    /// `caret_next_row`). Callers guard against a stale cache.
    fn cursor_row(&self) -> usize {
        let c = self.edit.cursor;
        let first = self
            .rows
            .iter()
            .position(|lc| c >= lc.start && c <= lc.end)
            .unwrap_or(0);
        if self.caret_next_row && self.rows.get(first + 1).is_some_and(|lc| lc.start == c) {
            first + 1
        } else {
            first
        }
    }

    fn content_h(&self) -> f32 {
        self.rows.len() as f32 * self.line_h
    }

    fn max_vscroll(&self) -> f32 {
        (self.content_h() - self.inner().h).max(0.0)
    }

    /// Moves the cursor up/down `delta` visual rows, keeping the char column.
    fn move_line(&mut self, delta: isize, extend: bool) {
        let rows = self.nav_rows();
        let cur = row_of(&rows, self.edit.cursor, self.caret_next_row);
        let (cs, ce) = rows[cur];
        let col = char_count(&self.edit.text[cs..self.edit.cursor.clamp(cs, ce)]);
        let target = (cur as isize + delta).clamp(0, rows.len() as isize - 1) as usize;
        let (ts, te) = rows[target];
        let byte = ts + nth_char_byte(&self.edit.text[ts..te], col);
        self.edit.set_cursor(byte, extend);
        self.caret_next_row = byte == ts;
    }

    fn scroll_to_caret(&mut self) {
        let top = self.cursor_row() as f32 * self.line_h;
        if top - self.vscroll < 0.0 {
            self.vscroll = top;
        }
        if top + self.line_h - self.vscroll > self.inner().h {
            self.vscroll = top + self.line_h - self.inner().h;
        }
        self.vscroll = self.vscroll.clamp(0.0, self.max_vscroll());
    }

    /// Byte offset at screen position, and whether it sits at the hit row's
    /// start (the caret affinity when that byte is a soft-wrap boundary).
    fn byte_at(&self, pos: Vec2) -> (usize, bool) {
        if self.rows.is_empty() {
            return (self.edit.text.len(), false);
        }
        let inner = self.inner();
        let li = (((pos.y - inner.y + self.vscroll) / self.line_h).floor() as isize)
            .clamp(0, self.rows.len() as isize - 1) as usize;
        let lc = &self.rows[li];
        // Clamp onto a live char boundary — the cache may be a frame stale.
        let mut byte = lc.byte_at(pos.x - inner.x).min(self.edit.text.len());
        while byte > 0 && !self.edit.text.is_char_boundary(byte) {
            byte -= 1;
        }
        (byte, byte == lc.start)
    }

    // --- scrollbar geometry (see `ScrollArea` for the shared look) ----------

    fn bar_rect(&self) -> Rect {
        let inner = self.inner();
        Rect::new(inner.right() - self.bar_w, inner.y, self.bar_w, inner.h)
    }

    fn thumb_h(&self) -> f32 {
        let vh = self.inner().h;
        if self.content_h() <= vh {
            return vh;
        }
        (vh * vh / self.content_h()).clamp(self.bar_min_thumb.min(vh), vh)
    }

    fn thumb_rect(&self) -> Rect {
        let bar = self.bar_rect();
        let th = self.thumb_h();
        let max = self.max_vscroll();
        let ty = if max > 0.0 {
            bar.y + (self.vscroll / max) * (bar.h - th)
        } else {
            bar.y
        };
        Rect::new(bar.x, ty, bar.w, th)
    }

    /// Sets the scroll so the thumb's top sits at screen `y` minus the grab.
    fn bar_drag_to(&mut self, y: f32) {
        let bar = self.bar_rect();
        let travel = (bar.h - self.thumb_h()).max(1.0);
        let ty = (y - self.bar_grab - bar.y).clamp(0.0, travel);
        self.vscroll = (ty / travel) * self.max_vscroll();
    }
}

impl Default for TextArea {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for TextArea {
    fn cursor(&self, pos: Vec2) -> CursorIcon {
        if self.disabled || (self.has_bar && self.bar_rect().contains(pos)) {
            CursorIcon::Default
        } else {
            CursorIcon::Text
        }
    }

    fn measure(&mut self, _avail: Size, ctx: &mut LayoutCtx) -> Size {
        let px = ctx.theme.font_px(TextRole::Body);
        self.line_h = px * 1.3;
        let n = self.edit.text.matches('\n').count() + 1;
        Size::new(
            240.0,
            (n as f32 * self.line_h).max(3.0 * self.line_h) + 2.0 * PAD_Y,
        )
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        self.line_h = ctx.theme.font_px(TextRole::Body) * 1.3;
        self.bar_w = ctx.theme.metrics().scrollbar;
        self.bar_min_thumb = ctx.theme.metrics().scrollbar_min_thumb;
        let inner = self.inner();
        // Wrap at the full width; if that overflows the height, re-wrap with
        // the scrollbar column reserved (a narrower wrap never yields fewer
        // rows, so this can't oscillate).
        let mut bounds = wrap_bounds(&self.edit.text, inner.w, ctx.fonts, ctx.theme);
        self.has_bar = bounds.len() as f32 * self.line_h > inner.h + 0.5;
        if self.has_bar {
            bounds = wrap_bounds(&self.edit.text, self.text_w(), ctx.fonts, ctx.theme);
        }
        self.rows = bounds
            .into_iter()
            .map(|(s, e)| {
                LineCache::build(&self.edit.text, s, e, ctx.fonts, ctx.theme, TextRole::Body)
            })
            .collect();
        if self.preedit.is_empty() {
            self.preedit_w = 0.0;
            self.preedit_caret_w = 0.0;
        } else {
            let font = ctx.theme.font();
            let px = ctx.theme.font_px(TextRole::Body);
            self.preedit_w = ctx.fonts.measure(font, &self.preedit, px);
            self.preedit_caret_w =
                ctx.fonts
                    .measure(font, &self.preedit[..self.preedit_cursor], px);
        }
        self.vscroll = self.vscroll.clamp(0.0, self.max_vscroll());
        self.scroll_to_caret();
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
        dl.push_clip(Rect::new(inner.x, inner.y, self.text_w(), inner.h));
        let px = ctx.theme.font_px(TextRole::Body);
        let (sa, sb) = self.edit.selection();
        // `rows` was built at the last arrange — if the host dispatched an
        // edit without a layout in between, skip the caret/preedit this frame
        // rather than index past the stale cache.
        let caret_row = focused.then(|| self.cursor_row()).filter(|cl| {
            self.rows.get(*cl).is_some_and(|lc| {
                lc.end <= self.edit.text.len()
                    && self.edit.cursor >= lc.start
                    && self.edit.cursor <= lc.end
                    && self.edit.text.is_char_boundary(lc.start)
                    && self.edit.text.is_char_boundary(lc.end)
            })
        });
        let composing = !self.preedit.is_empty();

        for (i, lc) in self.rows.iter().enumerate() {
            let y = inner.y - self.vscroll + i as f32 * self.line_h;
            if y + self.line_h < inner.y || y > inner.bottom() {
                continue; // off-screen line
            }
            // Selection band on this line.
            if sb > sa && sb > lc.start && sa <= lc.end {
                let a = sa.max(lc.start);
                let b = sb.min(lc.end);
                let ax = lc.x_of(a);
                let bx = if sb > lc.end {
                    lc.width() + 4.0
                } else {
                    lc.x_of(b)
                };
                dl.fill_rect(
                    Rect::new(inner.x + ax, y, bx - ax, self.line_h),
                    selection_fill(ctx.theme),
                );
            }
            let text = &self.edit.text[lc.start..lc.end];
            if composing && caret_row == Some(i) {
                // Composing: this line splits at the caret and the preedit run
                // sits between, underlined (see `TextInput::draw`).
                let pre_x = inner.x + lc.x_of(self.edit.cursor);
                ctx.theme.text(
                    dl,
                    ctx.fonts,
                    Vec2::new(inner.x, y + px),
                    &self.edit.text[lc.start..self.edit.cursor],
                    TextRole::Body,
                );
                ctx.theme.text(
                    dl,
                    ctx.fonts,
                    Vec2::new(pre_x, y + px),
                    &self.preedit,
                    TextRole::Body,
                );
                ctx.theme.text(
                    dl,
                    ctx.fonts,
                    Vec2::new(pre_x + self.preedit_w, y + px),
                    &self.edit.text[self.edit.cursor..lc.end],
                    TextRole::Body,
                );
                dl.fill_rect(
                    Rect::new(pre_x, y + px + 2.0, self.preedit_w, 1.0),
                    ctx.theme.ink_dim(),
                );
            } else {
                ctx.theme.text(
                    dl,
                    ctx.fonts,
                    Vec2::new(inner.x, y + px),
                    text,
                    TextRole::Body,
                );
            }
        }

        if let Some(cl) = caret_row
            && let Some(lc) = self.rows.get(cl)
        {
            // The caret tracks the composition cursor while composing
            // (`preedit_caret_w` is 0 otherwise).
            let cx = inner.x + lc.x_of(self.edit.cursor) + self.preedit_caret_w;
            let cy = inner.y - self.vscroll + cl as f32 * self.line_h;
            dl.fill_rect(
                Rect::new(cx, cy + 2.0, CARET_W, self.line_h - 4.0),
                ctx.theme.accent(),
            );
        }
        dl.pop_clip();

        if self.has_bar {
            ctx.theme.scrollbar(
                dl,
                self.bar_rect(),
                self.thumb_rect(),
                WidgetState {
                    pressed: self.bar_dragging,
                    ..Default::default()
                },
            );
        }
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        if self.disabled {
            return false;
        }
        match ev {
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                mods,
                ..
            } if ctx.is_target(self.id) => {
                ctx.consume_pointer();
                ctx.request_focus(self.id);
                // Scrollbar press: drag the thumb or page on the track.
                if self.has_bar && self.bar_rect().contains(ctx.pointer) {
                    let thumb = self.thumb_rect();
                    if thumb.contains(ctx.pointer) {
                        self.bar_dragging = true;
                        self.bar_grab = ctx.pointer.y - thumb.y;
                        ctx.capture(self.id);
                    } else {
                        let page = self.inner().h * 0.9;
                        let dy = if ctx.pointer.y < thumb.y { -page } else { page };
                        self.vscroll = (self.vscroll + dy).clamp(0.0, self.max_vscroll());
                    }
                    return true;
                }
                let (byte, at_start) = self.byte_at(ctx.pointer);
                // A double click takes the word under it, a triple the row it is
                // on — the same row Home/End move within, so the two agree about
                // where a row starts and ends. Neither starts a drag (see
                // `TextInput`), and Shift stays an explicit extend-from-here.
                match ctx.clicks {
                    2 if !mods.shift => {
                        self.drag_word = self.edit.word_range_at(byte);
                        self.edit.select_word_at(byte);
                        self.dragging = true;
                        ctx.capture(self.id);
                    }
                    n if n >= 3 && !mods.shift => {
                        let rows = self.nav_rows();
                        let (s, e) = rows[row_of(&rows, byte, at_start)];
                        self.edit.select_range(s, e);
                    }
                    _ => {
                        self.edit.set_cursor(byte, mods.shift);
                        self.dragging = true;
                        ctx.capture(self.id);
                    }
                }
                self.caret_next_row = at_start;
                true
            }
            Event::PointerMoved { .. } if self.bar_dragging && ctx.is_target(self.id) => {
                self.bar_drag_to(ctx.pointer.y);
                ctx.consume_pointer();
                true
            }
            Event::PointerMoved { .. } if self.dragging && ctx.is_target(self.id) => {
                let (byte, at_start) = self.byte_at(ctx.pointer);
                match self.drag_word {
                    Some(word) => self.edit.extend_word(word, byte),
                    None => self.edit.set_cursor(byte, true),
                }
                self.caret_next_row = at_start;
                ctx.consume_pointer();
                true
            }
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: false,
                ..
            } if self.dragging || self.bar_dragging => {
                self.dragging = false;
                self.drag_word = None;
                self.bar_dragging = false;
                ctx.consume_pointer();
                true
            }
            Event::Scroll { delta, .. }
                if self.rect.contains(ctx.pointer) && self.max_vscroll() > 0.0 =>
            {
                let dy = match delta {
                    crate::event::ScrollDelta::Lines(v) => v.y * self.line_h * 3.0,
                    crate::event::ScrollDelta::Pixels(v) => v.y,
                };
                self.vscroll = (self.vscroll + dy).clamp(0.0, self.max_vscroll());
                ctx.consume_pointer();
                true
            }
            Event::Key {
                key,
                pressed: true,
                mods,
                ..
            } if ctx.is_target(self.id) => {
                let ext = mods.shift;
                match key {
                    Key::Left => {
                        self.edit.move_left(ext);
                        self.caret_next_row = true;
                    }
                    Key::Right => {
                        self.edit.move_right(ext);
                        self.caret_next_row = true;
                    }
                    Key::Up => self.move_line(-1, ext),
                    Key::Down => self.move_line(1, ext),
                    Key::Home => {
                        let rows = self.nav_rows();
                        let (s, _) = rows[row_of(&rows, self.edit.cursor, self.caret_next_row)];
                        self.edit.set_cursor(s, ext);
                        self.caret_next_row = true;
                    }
                    Key::End => {
                        let rows = self.nav_rows();
                        let (_, e) = rows[row_of(&rows, self.edit.cursor, self.caret_next_row)];
                        self.edit.set_cursor(e, ext);
                        self.caret_next_row = false;
                    }
                    Key::Enter => {
                        self.edit.insert("\n");
                        self.caret_next_row = true;
                    }
                    Key::Backspace => {
                        self.edit.backspace();
                        self.caret_next_row = true;
                    }
                    Key::Delete => {
                        self.edit.delete_forward();
                        self.caret_next_row = true;
                    }
                    Key::Character('a' | 'A') if mods.ctrl => self.edit.select_all(),
                    // Copy/cut hand the selection to the host's OS clipboard
                    // (`Ui::take_clipboard`); consumed even selectionless so
                    // the chord never leaks to a host binding while focused.
                    Key::Character('c' | 'C') if mods.ctrl => {
                        if self.edit.has_selection() {
                            ctx.set_clipboard(self.edit.selected_text().to_string());
                        }
                    }
                    Key::Character('x' | 'X') if mods.ctrl => {
                        if self.edit.has_selection() {
                            ctx.set_clipboard(self.edit.selected_text().to_string());
                            self.edit.delete_selection();
                        }
                    }
                    _ => return false,
                }
                ctx.consume_keyboard();
                true
            }
            Event::Text(s) if ctx.is_target(self.id) => {
                self.clear_preedit();
                let filtered: String = s.chars().filter(|c| !c.is_control()).collect();
                if !filtered.is_empty() {
                    self.edit.insert(&filtered);
                    self.caret_next_row = true;
                }
                ctx.consume_keyboard();
                true
            }
            // A paste keeps its newlines (multi-line field); other control
            // characters — including a CRLF's `\r` — are dropped.
            Event::Paste(s) if ctx.is_target(self.id) => {
                self.clear_preedit();
                let filtered: String = s
                    .chars()
                    .filter(|c| *c == '\n' || !c.is_control())
                    .collect();
                if !filtered.is_empty() {
                    self.edit.insert(&filtered);
                    self.caret_next_row = true;
                }
                ctx.consume_keyboard();
                true
            }
            Event::ImePreedit { text, cursor } if ctx.is_target(self.id) => {
                self.preedit = text.to_string();
                self.preedit_cursor = preedit_caret(text, *cursor);
                ctx.consume_keyboard();
                true
            }
            // Focus left — the [`TextInput`] contract, minus Enter (which types a
            // newline here, so an area only ever commits on focus-out).
            Event::Blur(cause) if ctx.is_target(self.id) => {
                self.clear_preedit();
                self.dragging = false;
                self.bar_dragging = false;
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

    fn accepts_text_input(&self) -> bool {
        !self.disabled
    }

    fn ime_rect(&self) -> Option<Rect> {
        if self.disabled {
            return None;
        }
        let inner = self.inner();
        let cl = self.cursor_row();
        let lc = self.rows.get(cl)?;
        let cx = inner.x + lc.x_of(self.edit.cursor) + self.preedit_caret_w;
        let cy = inner.y - self.vscroll + cl as f32 * self.line_h;
        Some(Rect::new(cx, cy + 2.0, CARET_W, self.line_h - 4.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Gunmetal;

    const DEJAVU: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");

    fn themed() -> (Fonts, Gunmetal) {
        let mut fonts = Fonts::new();
        let font = fonts.add(DEJAVU.to_vec()).unwrap();
        (fonts, Gunmetal::new(font))
    }

    #[test]
    fn charset_accepts_partial_entries() {
        // Digits.
        assert!(Charset::Digits.accepts("0123"));
        assert!(!Charset::Digits.accepts("12a"));
        // Identifier: the empty string is a valid partial entry.
        assert!(Charset::Identifier.accepts(""));
        // Signed: a lone '-' and empty are valid partials.
        assert!(Charset::SignedInt.accepts(""));
        assert!(Charset::SignedInt.accepts("-"));
        assert!(Charset::SignedInt.accepts("-42"));
        assert!(!Charset::SignedInt.accepts("4-2"));
        // Decimal: one point, optional sign; "1." and "-.5" are valid partials.
        assert!(Charset::Decimal.accepts("1."));
        assert!(Charset::Decimal.accepts("-.5"));
        assert!(!Charset::Decimal.accepts("1.2.3"));
        assert!(!Charset::Decimal.accepts("1e5"));
        // Identifier: no leading digit.
        assert!(Charset::Identifier.accepts("_x1"));
        assert!(Charset::Identifier.accepts("alpha2"));
        assert!(!Charset::Identifier.accepts("1x"));

        // Slug: an identifier that also takes `-` - an asset id from a file
        // name. The leading-digit rule still holds.
        assert!(Charset::Slug.accepts(""));
        assert!(Charset::Slug.accepts("oak-stand-3"));
        assert!(Charset::Slug.accepts("_x1"));
        assert!(!Charset::Slug.accepts("1x"));
        assert!(
            !Charset::Slug.accepts("-lead"),
            "and a leading dash is not an identifier either"
        );
        assert!(!Charset::Slug.accepts("oak stand"), "still no spaces");
        assert!(
            !Charset::Identifier.accepts("oak-stand"),
            "which Identifier does not take"
        );
    }

    #[test]
    fn insert_filtered_drops_rejected_chars() {
        let mut e = Edit::new(String::new());
        e.insert_filtered("12a3", |t| Charset::Digits.accepts(t), None);
        assert_eq!(e.text, "123", "non-digits skipped, digits kept in order");
    }

    #[test]
    fn insert_filtered_enforces_max_len() {
        let mut e = Edit::new(String::new());
        e.insert_filtered("123456", |_| true, Some(4));
        assert_eq!(e.text, "1234", "stops at the char cap");
        // Already-full field rejects further input.
        e.insert_filtered("9", |_| true, Some(4));
        assert_eq!(e.text, "1234");
    }

    #[test]
    fn insert_filtered_validates_against_full_text_midstring() {
        // A decimal field already holding "1.5": typing another '.' anywhere is
        // rejected because the would-be full text has two points.
        let mut e = Edit::new("1.5".to_string());
        e.set_cursor(0, false); // cursor at start
        e.insert_filtered(".", |t| Charset::Decimal.accepts(t), None);
        assert_eq!(e.text, "1.5", "second decimal point rejected");
        // But a digit at the start is fine.
        e.insert_filtered("2", |t| Charset::Decimal.accepts(t), None);
        assert_eq!(e.text, "21.5");
    }

    /// A focused single-line field fires a commit on Enter (pollable via
    /// `Ui::fired`) — the toolkit-level signal a host turns into submit / read.
    /// The chord is consumed so it never leaks to a host binding; other keys and
    /// an unfocused field do not fire.
    /// A double click's word is a run of *like* characters, so an identifier
    /// comes out whole and the punctuation beside it does not come with it.
    #[test]
    fn select_word_at_takes_the_run_of_like_characters() {
        let word_at = |text: &str, byte: usize| {
            let mut e = Edit::new(text.to_string());
            e.select_word_at(byte);
            let (a, b) = e.selection();
            (text[a..b].to_string(), e.cursor)
        };

        let text = "foo_bar1(baz  qux)";
        assert_eq!(
            word_at(text, 2).0,
            "foo_bar1",
            "digits and _ are word characters"
        );
        assert_eq!(word_at(text, 0).0, "foo_bar1", "from the run's first byte");
        assert_eq!(word_at(text, 8).0, "(", "punctuation is its own class");
        assert_eq!(
            word_at(text, 13).0,
            "  ",
            "and a run of spaces is one 'word'"
        );
        assert_eq!(word_at(text, 9).0, "baz");
        // The caret lands at the run's end, so a following Shift+click extends
        // from the far edge (what a drag would have left behind).
        assert_eq!(word_at(text, 9).1, 12, "the caret ends up after the word");
        // A click past the last character takes the run that ends there.
        assert_eq!(word_at(text, text.len()).0, ")");
        assert_eq!(
            word_at("", 0).0,
            "",
            "empty text just collapses the selection"
        );
        // UTF-8: multi-byte letters are word characters like any other.
        assert_eq!(word_at("héllo wörld", 1).0, "héllo");
    }

    /// A double click in a field selects the word under it, a triple the whole
    /// field, and a plain click still just moves the caret — the streak comes
    /// from the `Ui`, which is the only thing holding a clock.
    #[test]
    fn a_double_click_takes_the_word_and_a_triple_the_field() {
        use crate::{Event, Modifiers, PointerButton, Ui};
        let (fonts, theme) = themed();
        let field = TextInput::with_text("foo bar");
        let id = field.id();
        let mut ui = Ui::new(field);
        ui.layout_in(Rect::new(0.0, 0.0, 300.0, 24.0), &theme, &fonts);

        // Past the end of the text, so the hit byte is the text length whatever
        // the font measures - the streak is what this test is about.
        let press = |x: f32, shift: bool| Event::PointerButton {
            button: PointerButton::Primary,
            pressed: true,
            pos: Vec2::new(x, 12.0),
            mods: if shift {
                Modifiers {
                    shift: true,
                    ..Modifiers::NONE
                }
            } else {
                Modifiers::NONE
            },
        };
        let release = |x: f32| Event::PointerButton {
            button: PointerButton::Primary,
            pressed: false,
            pos: Vec2::new(x, 12.0),
            mods: Modifiers::NONE,
        };
        let selected = |ui: &Ui| {
            ui.get::<TextInput>(id)
                .unwrap()
                .edit
                .selected_text()
                .to_string()
        };

        ui.dispatch(&[press(280.0, false), release(280.0)]);
        assert_eq!(selected(&ui), "", "one click only moves the caret");
        ui.dispatch(&[press(280.0, false), release(280.0)]);
        assert_eq!(selected(&ui), "bar", "the second takes the word under it");
        ui.dispatch(&[press(280.0, false), release(280.0)]);
        assert_eq!(selected(&ui), "foo bar", "the third takes the whole field");
        ui.dispatch(&[press(280.0, false), release(280.0)]);
        assert_eq!(
            selected(&ui),
            "foo bar",
            "and a fourth keeps it - the streak only grows"
        );

        // A press far enough away starts a new streak, however fast it lands.
        ui.dispatch(&[press(10.0, false), release(10.0)]);
        assert_eq!(
            selected(&ui),
            "",
            "a click elsewhere is a first click again"
        );

        // Shift is an explicit "extend from the caret", so it stays a caret move
        // however fast it was clicked: two rapid Shift presses at the end keep
        // extending from the caret the click above left at byte 0, where a bare
        // double click there would have taken "bar".
        ui.dispatch(&[press(280.0, true), release(280.0)]);
        let extended = selected(&ui);
        assert!(
            extended.ends_with("oo bar"),
            "Shift extends from the caret, got {extended:?}"
        );
        ui.dispatch(&[press(280.0, true), release(280.0)]);
        assert_eq!(
            selected(&ui),
            extended,
            "and a fast repeat still extends, it does not take a word"
        );
    }

    /// A double click's drag extends by **whole words**: the pointer wobbling
    /// inside the word it started on re-selects that same word (a per-character
    /// extend would cut it back to wherever the pointer landed), and dragging
    /// into the word before it takes both.
    #[test]
    fn a_double_click_drag_extends_by_whole_words() {
        use crate::{Event, Modifiers, PointerButton, Ui};
        let (fonts, theme) = themed();
        let field = TextInput::with_text("foo bar");
        let id = field.id();
        let mut ui = Ui::new(field);
        ui.layout_in(Rect::new(0.0, 0.0, 300.0, 24.0), &theme, &fonts);
        let press = |pressed| Event::PointerButton {
            button: PointerButton::Primary,
            pressed,
            pos: Vec2::new(280.0, 12.0),
            mods: Modifiers::NONE,
        };
        let selected = |ui: &Ui| {
            ui.get::<TextInput>(id)
                .unwrap()
                .edit
                .selected_text()
                .to_string()
        };
        // The x of a byte, so the drag targets can be named in text rather than
        // in whatever the font happens to measure.
        let x_of = |ui: &Ui, byte: usize| {
            let f = ui.get::<TextInput>(id).unwrap();
            f.inner().x + f.align_pad() + f.cache.x_of(byte) - f.scroll
        };

        ui.dispatch(&[press(true), press(false), press(true)]);
        assert_eq!(selected(&ui), "bar", "the double click took the word");

        // A wobble that stays inside "bar" keeps the whole word.
        let inside = x_of(&ui, 5); // between 'b' and 'a'
        ui.dispatch(&[Event::PointerMoved {
            pos: Vec2::new(inside, 12.0),
        }]);
        assert_eq!(
            selected(&ui),
            "bar",
            "the word survives the pointer moving inside it"
        );

        // Dragging back into "foo" takes both words and the space between.
        let back = x_of(&ui, 1); // inside "foo"
        ui.dispatch(&[
            Event::PointerMoved {
                pos: Vec2::new(back, 12.0),
            },
            press(false),
        ]);
        assert_eq!(
            selected(&ui),
            "foo bar",
            "and the drag extends word by word"
        );
    }

    #[test]
    fn enter_fires_a_commit() {
        use crate::{Event, Key, Modifiers, Ui};
        let key = |k| Event::Key {
            key: k,
            pressed: true,
            repeat: false,
            mods: Modifiers::NONE,
        };

        let field = TextInput::with_text("42");
        let id = field.id();
        let mut ui = Ui::new(field);
        ui.focus_first();
        let resp = ui.dispatch(&[key(Key::Enter)]);
        assert!(ui.fired(id), "Enter fires the focused field as a commit");
        assert!(
            resp.keyboard,
            "the Enter chord is consumed (never leaks to a host binding)"
        );
        // A non-committing key doesn't fire.
        ui.dispatch(&[key(Key::Left)]);
        assert!(!ui.fired(id), "arrow keys don't fire");
    }

    /// `caret()` exposes the caret's byte offset so a host that draws its own
    /// overlay (the editor console) can position a caret: end after `set_text`,
    /// then Home/End jump it to the bounds — a byte offset over the UTF-8 text.
    #[test]
    fn caret_reports_the_byte_offset() {
        use crate::{Event, Key, Modifiers, Ui};
        let key = |k| Event::Key {
            key: k,
            pressed: true,
            repeat: false,
            mods: Modifiers::NONE,
        };

        let field = TextInput::with_text("abé"); // 'é' is 2 bytes → 4 bytes total
        let id = field.id();
        let mut ui = Ui::new(field);
        ui.focus_first();
        let caret = |ui: &mut Ui| ui.get_mut::<TextInput>(id).unwrap().caret();
        assert_eq!(
            caret(&mut ui),
            4,
            "a fresh field parks the caret at the end"
        );
        ui.dispatch(&[key(Key::Home)]);
        assert_eq!(caret(&mut ui), 0, "Home moves the caret to the start");
        ui.dispatch(&[key(Key::End)]);
        assert_eq!(caret(&mut ui), 4, "End moves it back to the byte length");
    }

    /// `LineCache` lookups are defensive against stale bytes: a byte inside a
    /// multi-byte char snaps back to the previous boundary, a byte past the
    /// end snaps to the line width, and an unbuilt cache maps any x to its
    /// start.
    #[test]
    fn line_cache_snaps_off_boundary_lookups() {
        let (fonts, theme) = themed();
        let lc = LineCache::build("éa", 0, 3, &fonts, &theme, TextRole::Body);
        assert_eq!(
            lc.x_of(1),
            0.0,
            "a mid-char byte snaps back to the previous boundary"
        );
        assert!(
            (lc.x_of(9) - lc.width()).abs() < f32::EPSILON,
            "a past-the-end byte snaps to the line width"
        );
        assert_eq!(
            LineCache::default().byte_at(12.0),
            0,
            "an unbuilt cache maps any x to its start"
        );
    }

    /// The preedit caret clamp: a host cursor pointing inside a multi-byte
    /// char backs up to the previous char boundary.
    #[test]
    fn preedit_caret_clamps_to_char_boundaries() {
        assert_eq!(preedit_caret("é", Some((0, 1))), 0, "mid-char clamps back");
        assert_eq!(
            preedit_caret("ab", Some((0, 1))),
            1,
            "a boundary passes through"
        );
        assert_eq!(preedit_caret("ab", None), 2, "no cursor → caret at the end");
    }

    /// A whitespace run at a wrap point rides at the end of the row (clipped
    /// invisibly) instead of leading the next row.
    #[test]
    fn wrap_bounds_lets_whitespace_overflow_the_row() {
        let (fonts, theme) = themed();
        let px = theme.font_px(TextRole::Body);
        let max_w = fonts.measure(theme.font(), "aa", px);
        let rows = wrap_bounds("aa  b", max_w, &fonts, &theme);
        assert_eq!(
            rows,
            vec![(0, 4), (4, 5)],
            "both spaces stay on the first row; the next row starts at the word"
        );
    }

    /// Pre-layout geometry is safe: with no wrap cache a hit maps to the text
    /// end, and without overflow the thumb fills the whole track.
    #[test]
    fn text_area_pre_layout_geometry_is_defensive() {
        let ta = TextArea::with_text("hello");
        assert_eq!(
            ta.byte_at(Vec2::new(40.0, 10.0)),
            (5, false),
            "no rows yet → the hit maps to the text end"
        );

        let mut ta = TextArea::with_text("hi");
        ta.rect = Rect::new(0.0, 0.0, 120.0, 60.0);
        assert_eq!(
            ta.thumb_h(),
            ta.inner().h,
            "no overflow → the thumb spans the whole viewport"
        );
        assert_eq!(
            ta.thumb_rect(),
            ta.bar_rect(),
            "…and it sits over the whole track"
        );
    }
}
