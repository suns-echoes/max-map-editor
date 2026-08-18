//! A popup emoji picker over the [`crate::emoji`] catalog.
//!
//! [`EmojiPicker`] wraps content exactly like [`crate::ContextMenu`]
//! wraps it (same tier shape, same dismissal story): closed, it is a
//! transparent passthrough; open (host-driven [`EmojiPicker::open_at`]),
//! it draws a popup plate in the overlay pass — a hand-rolled search
//! line over a scrollable grid of emoji cells with group headers —
//! and owns the event stream until a pick or a dismissal.
//!
//! The search line is deliberately NOT a [`crate::TextInput`]: leaf
//! widgets draw in the base pass, a popup draws overlay, and UTS #51
//! names are plain ASCII — a byte-append query line covers the whole
//! catalog without focus or IME choreography. The host should still
//! `Ui::focus` the picker after opening so Escape/typing target it.
//!
//! Cells are data, not widgets ([`crate::MenuItem`]'s grounds): the
//! address surface for hosts and script drivers is
//! [`EmojiPicker::visible_names`] / [`EmojiPicker::cell_rect`], and a
//! pick lands both as `Ui::fired(picker_id)` (action = catalog index)
//! and [`EmojiPicker::take_picked`].
//!
//! Rendering emoji in COLOR needs the `cosmic` text backend; on the
//! hand-rolled backend feed [`EmojiPicker::set_entries`] from
//! [`crate::emoji::supported`] so only renderable cells show.

use crate::draw::DrawList;
use crate::emoji::{EMOJI, EmojiEntry, GROUPS};
use crate::event::{Event, Key, PointerButton, ScrollDelta};
use crate::geom::{Rect, Size, Vec2};
use crate::interact::{WidgetId, next_id};
use crate::scroll::{PageKeys, Scroller};
use crate::theme::TextRole;
use crate::widget::{DrawCtx, EventCtx, LayoutCtx, Semantics, Widget, kind_of};

/// Grid columns.
const COLS: usize = 9;
/// Cell edge (also the uniform line height of the grid).
const CELL: f32 = 30.0;
/// Panel padding.
const PAD: f32 = 8.0;
/// The search line's height.
const SEARCH_H: f32 = 26.0;
/// Reserved scrollbar column.
const BAR_W: f32 = 12.0;
/// The grid viewport's height.
const GRID_H: f32 = CELL * 8.0;

/// One laid-out grid line: a group header or up to [`COLS`] cells
/// (catalog indices into the picker's entry slice).
enum Line {
    Header(u8),
    Cells(Vec<usize>),
}

/// The popup emoji picker (see the module docs).
#[must_use]
pub struct EmojiPicker {
    id: WidgetId,
    content: Box<dyn Widget>,
    rect: Rect,
    entries: Vec<&'static EmojiEntry>,
    open: bool,
    anchor: Vec2,
    panel: Rect,
    query: String,
    lines: Vec<Line>,
    scroller: Scroller,
    hover: Option<usize>,
    picked: Option<&'static EmojiEntry>,
}

impl EmojiPicker {
    /// Wrap `content`; the picker spans whatever rect the host gives
    /// it (the popup clamps inside that rect).
    pub fn new(content: impl Widget + 'static) -> Self {
        EmojiPicker {
            id: next_id(),
            content: Box::new(content),
            rect: Rect::ZERO,
            entries: EMOJI.iter().collect(),
            open: false,
            anchor: Vec2::ZERO,
            panel: Rect::ZERO,
            query: String::new(),
            lines: Vec::new(),
            scroller: Scroller::new(),
            hover: None,
            picked: None,
        }
    }

    /// Replace the catalog slice the grid shows (e.g. the result of
    /// [`crate::emoji::supported`]). Keeps catalog order.
    pub fn set_entries(&mut self, entries: Vec<&'static EmojiEntry>) {
        self.entries = entries;
        self.refilter();
    }

    /// Open at `pos` (host-driven, the [`crate::ContextMenu::open_at`]
    /// caveat applies: no `Ui` popup registration — suits a picker
    /// hosted in its own tier). The query resets; `Ui::focus` the
    /// picker afterwards so keys reach it.
    pub fn open_at(&mut self, pos: Vec2) {
        self.open = true;
        self.anchor = pos;
        self.query.clear();
        self.hover = None;
        self.scroller.set_offset(0.0);
        self.refilter();
    }

    pub fn close(&mut self) {
        self.open = false;
        self.hover = None;
        self.lines.clear();
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn id(&self) -> WidgetId {
        self.id
    }

    /// The picked entry, once per pick (also fired as `Ui::fired`
    /// with the entry's index into the current entry slice).
    pub fn take_picked(&mut self) -> Option<&'static EmojiEntry> {
        self.picked.take()
    }

    /// The live query line.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Set the query (script drivers type through this).
    pub fn set_query(&mut self, q: &str) {
        self.query = q.to_string();
        self.scroller.set_offset(0.0);
        self.refilter();
    }

    /// UTS #51 names of every emoji the grid currently lists, grid
    /// order — the "what could a user read" address surface. Empty
    /// while closed.
    #[must_use]
    pub fn visible_names(&self) -> Vec<&'static str> {
        self.lines
            .iter()
            .filter_map(|l| match l {
                Line::Cells(cells) => Some(cells),
                Line::Header(_) => None,
            })
            .flatten()
            .map(|&i| self.entries[i].name)
            .collect()
    }

    /// The on-screen rect of the cell whose entry has this UTS #51
    /// `name` (script clicks). `None` while closed, before layout, or
    /// when the cell is scrolled out of the viewport.
    #[must_use]
    pub fn cell_rect(&self, name: &str) -> Option<Rect> {
        let grid = self.grid_rect();
        let mut y = grid.y - self.scroller.offset();
        for line in &self.lines {
            if let Line::Cells(cells) = line {
                for (c, &i) in cells.iter().enumerate() {
                    if self.entries[i].name == name {
                        let rect = Rect::new(grid.x + c as f32 * CELL, y, CELL, CELL);
                        return (rect.y >= grid.y - 0.5 && rect.bottom() <= grid.bottom() + 0.5)
                            .then_some(rect);
                    }
                }
            }
            y += CELL;
        }
        None
    }

    fn refilter(&mut self) {
        self.lines.clear();
        if !self.open {
            return;
        }
        let q = self.query.to_ascii_lowercase();
        let mut group: Option<u8> = None;
        let mut row: Vec<usize> = Vec::new();
        for (i, e) in self.entries.iter().enumerate() {
            if !q.is_empty() && !e.name.contains(q.as_str()) {
                continue;
            }
            // Group headers only while browsing — a search result is
            // one flat run.
            if q.is_empty() && group != Some(e.group) {
                if !row.is_empty() {
                    self.lines.push(Line::Cells(std::mem::take(&mut row)));
                }
                group = Some(e.group);
                self.lines.push(Line::Header(e.group));
            }
            row.push(i);
            if row.len() == COLS {
                self.lines.push(Line::Cells(std::mem::take(&mut row)));
            }
        }
        if !row.is_empty() {
            self.lines.push(Line::Cells(row));
        }
    }

    fn panel_size(&self) -> Size {
        Size::new(
            PAD * 2.0 + COLS as f32 * CELL + BAR_W,
            PAD * 2.0 + SEARCH_H + PAD + GRID_H,
        )
    }

    fn search_rect(&self) -> Rect {
        Rect::new(
            self.panel.x + PAD,
            self.panel.y + PAD,
            self.panel.w - PAD * 2.0,
            SEARCH_H,
        )
    }

    fn grid_rect(&self) -> Rect {
        Rect::new(
            self.panel.x + PAD,
            self.panel.y + PAD + SEARCH_H + PAD,
            COLS as f32 * CELL,
            GRID_H,
        )
    }

    fn content_height(&self) -> f32 {
        self.lines.len() as f32 * CELL
    }

    /// The grid cell under `pos`, as an index into `entries`.
    fn cell_at(&self, pos: Vec2) -> Option<usize> {
        let grid = self.grid_rect();
        if !grid.contains(pos) {
            return None;
        }
        let line = ((pos.y - grid.y + self.scroller.offset()) / CELL).floor();
        let col = ((pos.x - grid.x) / CELL).floor();
        if line < 0.0 || col < 0.0 {
            return None;
        }
        match self.lines.get(line as usize)? {
            Line::Header(_) => None,
            Line::Cells(cells) => cells.get(col as usize).copied(),
        }
    }
}

impl Widget for EmojiPicker {
    fn measure(&mut self, avail: Size, ctx: &mut LayoutCtx) -> Size {
        self.content.measure(avail, ctx)
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        self.content.arrange(rect, ctx);
        if !self.open {
            return;
        }
        // Slide left of the right edge, flip above at the bottom —
        // the ContextMenu clamp against the widget's own rect.
        let size = self.panel_size();
        let mut x = self.anchor.x.min((rect.right() - size.w).max(rect.x));
        let mut y = self.anchor.y;
        if y + size.h > rect.bottom() {
            y = (y - size.h).max(rect.y);
        }
        x = x.max(rect.x);
        self.panel = Rect::new(x, y, size.w, size.h);
        // The scroller's viewport spans the grid plus its bar column
        // (wheel-over-viewport is the machine's own rule).
        let grid = self.grid_rect();
        let view = Rect::new(grid.x, grid.y, grid.w + BAR_W, grid.h);
        self.scroller.layout(ctx, view, self.content_height());
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        self.content.draw(dl, ctx);
        if !self.open || !ctx.is_overlay() {
            return;
        }
        ctx.theme.popup(dl, self.panel);

        // Search line: a well with the query (or a dim hint) and a
        // block caret.
        let search = self.search_rect();
        ctx.theme
            .well(dl, search, crate::interact::WidgetState::default());
        let px = ctx.theme.font_px(TextRole::Body);
        let baseline = Vec2::new(search.x + 6.0, search.center().y + px * 0.34);
        let (text, ink) = if self.query.is_empty() {
            ("search emoji", ctx.theme.ink_dim())
        } else {
            (self.query.as_str(), ctx.theme.ink())
        };
        let w = crate::text::draw_line(dl, ctx.fonts, ctx.theme.font(), text, baseline, px, ink);
        let caret_x = if self.query.is_empty() {
            search.x + 6.0
        } else {
            search.x + 6.0 + w + 1.0
        };
        dl.fill_rect(
            Rect::new(caret_x, search.y + 5.0, 1.0, search.h - 10.0),
            ctx.theme.accent(),
        );

        // The grid, clipped to its viewport.
        let grid = self.grid_rect();
        dl.push_clip(grid);
        let mut y = grid.y - self.scroller.offset();
        for line in &self.lines {
            if y + CELL >= grid.y && y <= grid.bottom() {
                match line {
                    Line::Header(g) => {
                        let baseline = Vec2::new(grid.x + 2.0, y + CELL * 0.5 + px * 0.34);
                        ctx.theme.text_colored(
                            dl,
                            ctx.fonts,
                            baseline,
                            GROUPS[*g as usize],
                            TextRole::Small,
                            crate::theme::Emboss::Flat,
                            ctx.theme.ink_dim(),
                        );
                    }
                    Line::Cells(cells) => {
                        for (c, &i) in cells.iter().enumerate() {
                            let cell = Rect::new(grid.x + c as f32 * CELL, y, CELL, CELL);
                            if self.hover == Some(i) {
                                ctx.theme
                                    .accent_row(dl, cell, crate::theme::ROW_FLOOR_HOVER);
                            }
                            let e = self.entries[i];
                            let ew = ctx.fonts.measure(ctx.theme.font(), e.emoji, px);
                            let baseline = Vec2::new(
                                cell.x + (cell.w - ew) * 0.5,
                                cell.y + cell.h * 0.5 + px * 0.34,
                            );
                            crate::text::draw_line(
                                dl,
                                ctx.fonts,
                                ctx.theme.font(),
                                e.emoji,
                                baseline,
                                px,
                                ctx.theme.ink(),
                            );
                        }
                    }
                }
            }
            y += CELL;
        }
        dl.pop_clip();
        self.scroller.draw_bar(dl, ctx.theme);
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        if !self.open {
            return self.content.event(ev, ctx);
        }
        // Open: the picker owns the stream (ContextMenu's arms).
        if self
            .scroller
            .event_with(ev, ctx, self.id, PageKeys::WhenFocused)
        {
            return true;
        }
        match ev {
            Event::PointerMoved { .. } => {
                self.hover = self.cell_at(ctx.pointer);
                ctx.consume_pointer();
                true
            }
            // Wheel over the panel but off the grid viewport (the
            // scroller already handled over-viewport wheel above).
            Event::Scroll { delta, .. } => {
                if self.panel.contains(ctx.pointer) {
                    let dy = match delta {
                        ScrollDelta::Lines(v) => v.y * CELL,
                        ScrollDelta::Pixels(v) => v.y,
                    };
                    self.scroller.scroll_by(dy);
                }
                ctx.consume_pointer();
                true
            }
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                ..
            } => {
                if let Some(i) = self.cell_at(ctx.pointer) {
                    self.picked = Some(self.entries[i]);
                    ctx.fire(self.id, Some(i as u64));
                    self.close();
                } else if !self.panel.contains(ctx.pointer) {
                    self.close();
                }
                ctx.consume_pointer();
                true
            }
            Event::PointerButton {
                button: PointerButton::Secondary,
                pressed: true,
                ..
            } => {
                self.close();
                ctx.consume_pointer();
                true
            }
            Event::Text(s) if ctx.is_target(self.id) => {
                for ch in s.chars().filter(|c| !c.is_control()) {
                    self.query.push(ch);
                }
                self.scroller.set_offset(0.0);
                self.refilter();
                true
            }
            Event::Key {
                key: Key::Backspace,
                pressed: true,
                ..
            } if ctx.is_target(self.id) => {
                self.query.pop();
                self.refilter();
                ctx.consume_keyboard();
                true
            }
            Event::Key {
                key: Key::Enter,
                pressed: true,
                ..
            } if ctx.is_target(self.id) => {
                // Enter picks the first visible cell (script + keyboard
                // convenience).
                let first = self.lines.iter().find_map(|l| match l {
                    Line::Cells(cells) => cells.first().copied(),
                    Line::Header(_) => None,
                });
                if let Some(i) = first {
                    self.picked = Some(self.entries[i]);
                    ctx.fire(self.id, Some(i as u64));
                    self.close();
                }
                ctx.consume_keyboard();
                true
            }
            Event::Key {
                key: Key::Escape,
                pressed: true,
                ..
            } if ctx.is_target(self.id) => {
                // A live query clears first; a second Escape closes.
                if self.query.is_empty() {
                    self.close();
                } else {
                    self.query.clear();
                    self.refilter();
                }
                ctx.consume_keyboard();
                true
            }
            Event::Focus(false) => {
                self.content.event(ev, ctx);
                self.close();
                false
            }
            Event::Blur(_) if ctx.is_target(self.id) => {
                self.close();
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
        // Only while open: a closed picker is a transparent
        // passthrough and must stay OUT of the host's Tab cycle and
        // `focus_first` walks (it wraps whole tiers of chrome).
        self.open
    }

    fn semantics(&self) -> Semantics<'_> {
        Semantics::labeled(kind_of::<Self>(), "emoji-picker")
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
        if self.open {
            return self.rect.contains(pos).then_some(self.id);
        }
        self.content.hit_test(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_picker() -> EmojiPicker {
        let mut p = EmojiPicker::new(crate::Spacer::new());
        p.open_at(Vec2::new(10.0, 10.0));
        p
    }

    #[test]
    fn filter_narrows_and_headers_only_while_browsing() {
        let mut p = open_picker();
        assert!(
            p.lines.iter().any(|l| matches!(l, Line::Header(_))),
            "browsing shows group headers"
        );
        let all = p.visible_names().len();
        assert!(all > 3000);

        p.set_query("cherries");
        let names = p.visible_names();
        assert!(names.contains(&"cherries"), "{names:?}");
        assert!(names.len() < 10);
        assert!(
            !p.lines.iter().any(|l| matches!(l, Line::Header(_))),
            "a search result is one flat run"
        );

        p.set_query("");
        assert_eq!(p.visible_names().len(), all);
    }

    #[test]
    fn closed_picker_exposes_nothing() {
        let mut p = open_picker();
        p.close();
        assert!(p.visible_names().is_empty());
        assert!(p.cell_rect("cherries").is_none());
        assert!(p.take_picked().is_none());
    }

    /// The full hosted flow through a real `Ui`: open, focus, type a
    /// query, Enter picks the first hit (fired + take_picked), the
    /// popup closes; reopen and dismiss with an outside click.
    #[test]
    fn typed_search_and_pick_through_dispatch() {
        use crate::Ui;
        use crate::event::Modifiers;
        use crate::text::Fonts;
        use crate::theme::Gunmetal;

        let mut fonts = Fonts::new();
        let font = fonts
            .add(include_bytes!("../assets/DejaVuSans.ttf").to_vec())
            .unwrap();
        let theme = Gunmetal::new(font);

        let picker = EmojiPicker::new(crate::Spacer::new());
        let id = picker.id();
        let mut ui = Ui::new(picker);
        ui.layout(Size::new(640.0, 480.0), &theme, &fonts);

        ui.get_mut::<EmojiPicker>(id)
            .unwrap()
            .open_at(Vec2::new(50.0, 40.0));
        ui.layout(Size::new(640.0, 480.0), &theme, &fonts);
        assert!(ui.focus(id), "the picker takes focus for its keys");

        ui.dispatch(&[
            Event::Text("cherries".into()),
            Event::Key {
                key: Key::Enter,
                pressed: true,
                repeat: false,
                mods: Modifiers::NONE,
            },
        ]);
        assert!(ui.fired(id), "a pick fires");
        let p = ui.get_mut::<EmojiPicker>(id).unwrap();
        let picked = p.take_picked().expect("picked");
        assert_eq!(picked.name, "cherries");
        assert!(!p.is_open(), "a pick closes the popup");

        // Reopen; an outside primary press dismisses without a pick.
        p.open_at(Vec2::new(50.0, 40.0));
        ui.layout(Size::new(640.0, 480.0), &theme, &fonts);
        ui.dispatch(&[
            Event::PointerMoved {
                pos: Vec2::new(620.0, 460.0),
            },
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                pos: Vec2::new(620.0, 460.0),
                mods: Modifiers::NONE,
            },
        ]);
        let p = ui.get_mut::<EmojiPicker>(id).unwrap();
        assert!(!p.is_open(), "outside click dismisses");
        assert!(p.take_picked().is_none());
    }
}
