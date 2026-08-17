//! Menus: a [`MenuBar`] (app menu) and a [`ContextMenu`], both rendering a
//! cascade of dropdown panels with submenus. The whole open cascade is owned by
//! one widget and drawn in the overlay pass, so it works with the single-popup
//! model: hover opens submenus, clicking a leaf fires its action, clicking
//! outside dismisses.

use crate::draw::DrawList;
use crate::event::{Event, Key, PointerButton};
use crate::geom::{Insets, Rect, Size, Vec2};
use crate::icon::Icon;
use crate::interact::{WidgetId, WidgetState, next_id};
use crate::text::Fonts;
use crate::theme::{Emboss, POPUP_FRAME, ROW_FLOOR_ACTIVE, ROW_FLOOR_HOVER, TextRole, Theme};
use crate::widget::{DrawCtx, EventCtx, LayoutCtx, Widget};

const ARROW_W: f32 = 16.0;
/// Left check/toggle column width (logical px), used when a panel has any
/// checkable items.
const CHECK_W: f32 = 18.0;
/// Icon column width (logical px), used when a panel has any items carrying a
/// stencil [`Icon`] — the 16px stamp centered with a little air.
const ICON_W: f32 = 22.0;
/// Gap between a label and its right-aligned shortcut hint.
const SHORTCUT_GAP: f32 = 24.0;
/// Vertical padding inside a cascade panel: above the first row and below the
/// last, inside the popup frame.
const PANEL_VPAD: f32 = 4.0;
/// A separator row's height — the theme's line centered with ~2px margins,
/// not a full item row.
const SEP_H: f32 = 6.0;

/// One entry in a menu: a command, a submenu, or a separator. Commands may carry
/// a [`shortcut`](MenuItem::shortcut) hint, a [`checked`](MenuItem::checked)
/// mark (toggle items), and an [`enabled`](MenuItem::enabled) flag (disabled
/// items draw dim and don't fire).
#[must_use]
pub struct MenuItem {
    label: String,
    action: Option<u64>,
    children: Vec<MenuItem>,
    separator: bool,
    shortcut: Option<String>,
    /// `None` = not a toggle; `Some(b)` = a toggle item showing `b`.
    checked: Option<bool>,
    enabled: bool,
    /// A stencil stamped in a left icon column (see [`MenuItem::icon`]).
    icon: Option<Icon>,
    /// Columns layout for the child panel (see [`MenuItem::columns`]):
    /// `children` holds the columns' items flattened column-major; these give
    /// each column's header title and item count. Empty = a plain list panel.
    col_titles: Vec<String>,
    col_lens: Vec<usize>,
}

impl MenuItem {
    /// A command item that emits `action` when chosen.
    pub fn item(label: impl Into<String>, action: u64) -> Self {
        Self {
            label: label.into(),
            action: Some(action),
            children: Vec::new(),
            separator: false,
            shortcut: None,
            checked: None,
            enabled: true,
            col_titles: Vec::new(),
            col_lens: Vec::new(),
            icon: None,
        }
    }

    /// A submenu.
    pub fn sub(label: impl Into<String>, children: Vec<MenuItem>) -> Self {
        Self {
            label: label.into(),
            action: None,
            children,
            separator: false,
            shortcut: None,
            checked: None,
            enabled: true,
            col_titles: Vec::new(),
            col_lens: Vec::new(),
            icon: None,
        }
    }

    /// A submenu whose panel lays out as labelled **columns** — each a header
    /// title over a stack of items (e.g. map sizes over the maps of that
    /// size). Behaves like [`sub`](MenuItem::sub) otherwise.
    pub fn columns(label: impl Into<String>, columns: Vec<(String, Vec<MenuItem>)>) -> Self {
        let mut col_titles = Vec::with_capacity(columns.len());
        let mut col_lens = Vec::with_capacity(columns.len());
        let mut children = Vec::new();
        for (title, items) in columns {
            col_titles.push(title);
            col_lens.push(items.len());
            children.extend(items);
        }
        Self {
            label: label.into(),
            action: None,
            children,
            separator: false,
            shortcut: None,
            checked: None,
            enabled: true,
            col_titles,
            col_lens,
            icon: None,
        }
    }

    /// A horizontal separator line.
    pub fn separator() -> Self {
        Self {
            label: String::new(),
            action: None,
            children: Vec::new(),
            separator: true,
            shortcut: None,
            checked: None,
            enabled: true,
            col_titles: Vec::new(),
            col_lens: Vec::new(),
            icon: None,
        }
    }

    /// Attaches a right-aligned shortcut hint (e.g. `Ctrl+S`), drawn dim.
    pub fn shortcut(mut self, text: impl Into<String>) -> Self {
        self.shortcut = Some(text.into());
        self
    }

    /// Makes this a toggle item showing a checkmark when `checked`.
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    /// Enables or disables the item; disabled items draw dim and never fire.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Attaches a stencil [`Icon`], stamped in a left icon column shared by
    /// the whole panel (any item carrying one reserves the column). Stamped
    /// through [`Theme::icon`] in the row's ink — dim when disabled — so a
    /// theme re-treats menu icons with every other icon in the app.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    fn has_children(&self) -> bool {
        !self.children.is_empty()
    }
}

/// A columns panel's internal geometry: per-column x offsets/widths within
/// the panel, and each column's item count (children are flattened
/// column-major, so cell ↔ flat-index maps through these).
struct ColsLayout {
    xs: Vec<f32>,
    ws: Vec<f32>,
    lens: Vec<usize>,
}

/// The cached geometry of one open cascade panel.
struct Panel {
    rect: Rect,
    /// Index path (into the root items) of this panel's parent submenu; the
    /// panel shows the items at that path.
    path: Vec<usize>,
    /// `Some` when the parent item is a [`MenuItem::columns`] submenu.
    cols: Option<ColsLayout>,
    /// Plain panels: per-item y offsets within the content area (see
    /// [`row_offsets`]; separator rows are slimmer than item rows). Columns
    /// panels keep the uniform `row_h` grid instead and leave this empty.
    ys: Vec<f32>,
}

impl Panel {
    /// The flat item index under `p`, or `None` when `p` is inside the panel
    /// but on no item (the top/bottom padding, a columns header, a short
    /// column's empty tail).
    fn index_at(&self, p: Vec2, row_h: f32) -> Option<usize> {
        let dy = p.y - self.rect.y - PANEL_VPAD;
        match &self.cols {
            None => self.ys.windows(2).position(|w| dy >= w[0] && dy < w[1]),
            Some(c) => {
                // A negative dy (the top padding) saturates to row 0 — the
                // header band — which maps to no item below anyway.
                let row = (dy / row_h) as usize;
                let col =
                    c.xs.iter()
                        .zip(&c.ws)
                        .position(|(&x, &w)| p.x >= self.rect.x + x && p.x < self.rect.x + x + w)?;
                // Row 0 is the header band.
                let r = row.checked_sub(1)?;
                (r < c.lens[col]).then(|| c.lens[..col].iter().sum::<usize>() + r)
            }
        }
    }

    /// Flat item `i`'s cell rect.
    fn cell_rect(&self, i: usize, row_h: f32) -> Rect {
        match &self.cols {
            None => Rect::new(
                self.rect.x,
                self.rect.y + PANEL_VPAD + self.ys[i],
                self.rect.w,
                self.ys[i + 1] - self.ys[i],
            ),
            Some(c) => {
                let (mut col, mut base) = (0usize, 0usize);
                while col + 1 < c.lens.len() && i >= base + c.lens[col] {
                    base += c.lens[col];
                    col += 1;
                }
                Rect::new(
                    self.rect.x + c.xs[col],
                    self.rect.y + PANEL_VPAD + (i - base + 1) as f32 * row_h,
                    c.ws[col],
                    row_h,
                )
            }
        }
    }
}

/// Per-item y offsets within a panel's content area (`n + 1` prefix sums, the
/// last being the content height): item rows are `row_h` tall, separator rows
/// [`SEP_H`].
fn row_offsets(items: &[MenuItem], row_h: f32) -> Vec<f32> {
    let mut ys = Vec::with_capacity(items.len() + 1);
    let mut y = 0.0;
    for it in items {
        ys.push(y);
        y += if it.separator { SEP_H } else { row_h };
    }
    ys.push(y);
    ys
}

fn items_at<'a>(root: &'a [MenuItem], path: &[usize]) -> &'a [MenuItem] {
    let mut items = root;
    for &i in path {
        items = &items[i].children;
    }
    items
}

/// The item a non-empty `path` names (the submenu whose children form the
/// panel at that path).
fn node_at<'a>(root: &'a [MenuItem], path: &[usize]) -> Option<&'a MenuItem> {
    let (&last, rest) = path.split_last()?;
    items_at(root, rest).get(last)
}

/// Whether any item in this panel carries a checkmark column.
fn has_checks(items: &[MenuItem]) -> bool {
    items.iter().any(|it| it.checked.is_some())
}

/// Whether any item in this panel carries a stencil icon column.
fn has_icons(items: &[MenuItem]) -> bool {
    items.iter().any(|it| it.icon.is_some())
}

fn panel_width(items: &[MenuItem], fonts: &Fonts, theme: &dyn Theme) -> f32 {
    let font = theme.font();
    let px = theme.font_px(TextRole::Body);
    let pad = theme.metrics().pad;
    let widest = items
        .iter()
        .map(|it| fonts.measure(font, &it.label, px))
        .fold(0.0_f32, f32::max);
    let widest_shortcut = items
        .iter()
        .filter_map(|it| it.shortcut.as_deref())
        .map(|s| fonts.measure(font, s, px))
        .fold(0.0_f32, f32::max);
    let check = if has_checks(items) { CHECK_W } else { 0.0 };
    let icon = if has_icons(items) { ICON_W } else { 0.0 };
    let shortcut = if widest_shortcut > 0.0 {
        SHORTCUT_GAP + widest_shortcut
    } else {
        0.0
    };
    // Shortcut hints and submenu arrows right-align to the same column
    // (inset by `pad`, mirroring the left edge), so the wider of the two
    // reserves the space — the arrow gutter only exists where a panel has
    // submenus at all.
    let arrow = if items.iter().any(MenuItem::has_children) {
        ARROW_W
    } else {
        0.0
    };
    pad + check + icon + widest + shortcut.max(arrow) + pad
}

/// Builds the open cascade panels given the top panel's anchor (its top-left)
/// and the open submenu path.
fn build_panels(
    root: &[MenuItem],
    top_left: Vec2,
    open_path: &[usize],
    row_h: f32,
    fonts: &Fonts,
    theme: &dyn Theme,
) -> Vec<Panel> {
    let mut panels = Vec::new();
    // Panel 0: the top dropdown.
    let w0 = panel_width(root, fonts, theme);
    let ys0 = row_offsets(root, row_h);
    let h0 = ys0.last().copied().unwrap_or(0.0) + 2.0 * PANEL_VPAD;
    panels.push(Panel {
        rect: Rect::new(top_left.x, top_left.y, w0, h0),
        path: Vec::new(),
        cols: None,
        ys: ys0,
    });
    // One panel per open submenu level.
    for k in 0..open_path.len() {
        let parent = &panels[k];
        let row = open_path[k];
        let path: Vec<usize> = open_path[..=k].to_vec();
        let items = items_at(root, &path);
        if items.is_empty() {
            break;
        }
        // Anchor beside the parent's row cell (columns cells included); the
        // vertical pad is subtracted so the child's first row lines up with
        // the parent row, not the pad above it.
        let anchor_y = parent.cell_rect(row, row_h).y - PANEL_VPAD;
        let node = node_at(root, &path);
        let (w, h, cols, ys) = match node.filter(|n| !n.col_lens.is_empty()) {
            Some(n) => {
                // A columns panel: per-column widths (items or header title,
                // whichever is wider), a header band, then the tallest column.
                let font = theme.font();
                let px = theme.font_px(TextRole::Body);
                let pad = theme.metrics().pad;
                let mut xs = Vec::with_capacity(n.col_lens.len());
                let mut ws = Vec::with_capacity(n.col_lens.len());
                let (mut x, mut base, mut tallest) = (0.0f32, 0usize, 0usize);
                for (c, &len) in n.col_lens.iter().enumerate() {
                    let iw = panel_width(&items[base..base + len], fonts, theme);
                    let tw = fonts.measure(font, &n.col_titles[c], px) + 2.0 * pad;
                    xs.push(x);
                    ws.push(iw.max(tw));
                    x += iw.max(tw);
                    base += len;
                    tallest = tallest.max(len);
                }
                let layout = ColsLayout {
                    xs,
                    ws,
                    lens: n.col_lens.clone(),
                };
                let h = row_h * (tallest + 1) as f32 + 2.0 * PANEL_VPAD;
                (x, h, Some(layout), Vec::new())
            }
            None => {
                let ys = row_offsets(items, row_h);
                let h = ys.last().copied().unwrap_or(0.0) + 2.0 * PANEL_VPAD;
                (panel_width(items, fonts, theme), h, None, ys)
            }
        };
        panels.push(Panel {
            // Overlap the parent by both popup frames, so the two panels read
            // as one joined surface instead of a doubled 2px seam.
            rect: Rect::new(parent.rect.right() - 2.0 * POPUP_FRAME, anchor_y, w, h),
            path,
            cols,
            ys,
        });
    }
    panels
}

/// What the pointer is over inside an open cascade.
enum Over {
    /// Inside panel `pi`; `Some(flat item index)` when on an item cell (a
    /// columns header band / short-column tail is inside but on no item).
    Panel(usize, Option<usize>),
    Outside,
}

fn over_at(panels: &[Panel], p: Vec2, row_h: f32) -> Over {
    // Deepest panel first (it draws on top).
    for (pi, panel) in panels.iter().enumerate().rev() {
        if panel.rect.contains(p) {
            return Over::Panel(pi, panel.index_at(p, row_h));
        }
    }
    Over::Outside
}

/// Draws one cascade of panels (shared by the bar and the context menu).
fn draw_cascade(
    dl: &mut DrawList,
    ctx: &DrawCtx,
    root: &[MenuItem],
    panels: &[Panel],
    hover: &[usize],
    row_h: f32,
) {
    let px = ctx.theme.font_px(TextRole::Body);
    let pad = ctx.theme.metrics().pad;
    for panel in panels {
        // The popup surface (1px outset frame) — the same surface a select's
        // option list floats on, so every dropdown in the app reads alike.
        ctx.theme.popup(dl, panel.rect);
        let inner = panel.rect.inset(Insets::all(POPUP_FRAME));
        let items = items_at(root, &panel.path);
        let checks = has_checks(items);
        let check_w = if checks { CHECK_W } else { 0.0 };
        let icon_w = if has_icons(items) { ICON_W } else { 0.0 };
        // A columns panel starts with its header band: dim titles per column.
        if let Some(c) = &panel.cols
            && let Some(node) = node_at(root, &panel.path)
        {
            for (ci, title) in node.col_titles.iter().enumerate() {
                let baseline = Vec2::new(
                    panel.rect.x + c.xs[ci] + pad,
                    panel.rect.y + PANEL_VPAD + row_h * 0.5 + px * 0.34,
                );
                ctx.theme
                    .text_muted(dl, ctx.fonts, baseline, title, TextRole::Body);
            }
        }
        for (i, it) in items.iter().enumerate() {
            let row = panel.cell_rect(i, row_h);
            if it.separator {
                ctx.theme
                    .separator(dl, Rect::new(row.x + pad, row.y, row.w - 2.0 * pad, row.h));
                continue;
            }
            // Highlight if this row is on the hovered path — a row tint kept
            // inside the popup's frame (only a button covers its bevel).
            let on_hover = hover.len() == panel.path.len() + 1
                && hover[..panel.path.len()] == panel.path[..]
                && hover.last() == Some(&i);
            if on_hover {
                ctx.theme
                    .accent_row(dl, row.intersect(&inner), ROW_FLOOR_HOVER);
            }
            // Left check column: every toggle item draws a checkbox well (the
            // stock Checkbox read, sized for a menu row) — an accent square in
            // it when checked, an empty well when not. Geometric, so it never
            // depends on the theme font's glyph set.
            if let Some(checked) = it.checked {
                let s = 12.0;
                let b = Rect::new(
                    row.x + pad + (CHECK_W - s) * 0.5,
                    row.center().y - s * 0.5,
                    s,
                    s,
                );
                ctx.theme.well(
                    dl,
                    b,
                    WidgetState {
                        selected: checked,
                        ..WidgetState::default()
                    },
                );
                if checked {
                    dl.fill_rect(b.inset(Insets::all(3.0)), ctx.theme.accent());
                }
            }
            // Icon column: the stencil stamped through the theme in the row's
            // ink, matching the label's emboss — dim when disabled, like it.
            // Sized by `icon::fit` off the column, like every other icon key,
            // so the art drawn for this UI scale lands one cell per physical
            // pixel instead of being squeezed into a logical 16.
            if let Some(icon) = it.icon {
                let column = Rect::new(
                    row.x + pad + check_w,
                    row.center().y - ICON_W * 0.5,
                    ICON_W,
                    ICON_W,
                );
                let (stencil, ir) = crate::icon::fit(icon, column, ctx.scale);
                let ink = if it.enabled {
                    ctx.theme.ink()
                } else {
                    ctx.theme.ink_dim()
                };
                ctx.theme.icon(dl, ir, stencil, Emboss::Engraved, ink);
            }
            // Label — dim when disabled.
            let baseline = Vec2::new(row.x + pad + check_w + icon_w, row.center().y + px * 0.34);
            if it.enabled {
                ctx.theme
                    .text(dl, ctx.fonts, baseline, &it.label, TextRole::Body);
            } else {
                ctx.theme
                    .text_muted(dl, ctx.fonts, baseline, &it.label, TextRole::Body);
            }
            // Right-aligned shortcut hint (dim), inset from the right edge by
            // the same pad the label keeps on the left.
            if let Some(sc) = &it.shortcut {
                let sw = ctx.fonts.measure(ctx.theme.font(), sc, px);
                let sx = row.right() - pad - sw;
                let sb = Vec2::new(sx, row.center().y + px * 0.34);
                ctx.theme.text_muted(dl, ctx.fonts, sb, sc, TextRole::Body);
            }
            if it.has_children() {
                // A small right-pointing arrow (4px glyph), right-aligned to
                // the shortcut column: same `pad` inset as the left edge.
                let cx = row.right() - pad - 4.0;
                let cy = row.center().y;
                for j in 0..4 {
                    let hh = 4.0 - j as f32;
                    dl.fill_rect(
                        Rect::new(cx + j as f32, cy - hh, 1.0, 2.0 * hh),
                        ctx.theme.ink(),
                    );
                }
            }
        }
        // Vertical rules between the columns, spanning the content area.
        // Drawn after the rows so a hovered cell's tint (which fills the cell
        // up to the column boundary) never washes over them.
        if let Some(c) = &panel.cols {
            for &cx in c.xs.iter().skip(1) {
                ctx.theme.vseparator(
                    dl,
                    Rect::new(
                        panel.rect.x + cx - 1.0,
                        panel.rect.y + PANEL_VPAD,
                        2.0,
                        panel.rect.h - 2.0 * PANEL_VPAD,
                    ),
                );
            }
        }
    }
}

// --- MenuBar ----------------------------------------------------------------

/// A horizontal application menu bar. Each title opens a dropdown cascade.
#[must_use]
pub struct MenuBar {
    id: WidgetId,
    menus: Vec<(String, Vec<MenuItem>)>,
    open: Option<usize>,
    open_path: Vec<usize>,
    hover: Vec<usize>,
    /// Header under the pointer while the bar is closed (hover highlight).
    hover_header: Option<usize>,
    /// Overrides the theme's titlebar metric (see [`bar_height`](Self::bar_height)).
    fixed_bar_h: Option<f32>,
    bar_h: f32,
    row_h: f32,
    headers: Vec<Rect>,
    panels: Vec<Panel>,
    rect: Rect,
}

impl MenuBar {
    pub fn new() -> Self {
        Self {
            id: next_id(),
            menus: Vec::new(),
            open: None,
            open_path: Vec::new(),
            hover: Vec::new(),
            hover_header: None,
            fixed_bar_h: None,
            bar_h: 22.0,
            row_h: 22.0,
            headers: Vec::new(),
            panels: Vec::new(),
            rect: Rect::ZERO,
        }
    }

    pub fn menu(mut self, title: impl Into<String>, items: Vec<MenuItem>) -> Self {
        self.menus.push((title.into(), items));
        self
    }

    /// Overrides the bar's height (defaults to the theme's titlebar metric) —
    /// for hosts fitting the bar into an existing chrome strip.
    pub fn bar_height(mut self, h: f32) -> Self {
        self.fixed_bar_h = Some(h);
        self
    }

    pub fn id(&self) -> WidgetId {
        self.id
    }

    /// Closes any open dropdown — for hosts that dismiss the menu after
    /// running a command or on a mode change.
    pub fn close(&mut self) {
        self.open = None;
        self.open_path.clear();
        self.hover.clear();
        self.panels.clear();
    }

    /// Whether a dropdown cascade is open.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Opens the dropdown titled `title` (as a click on its header would);
    /// `false` if no menu has that title.
    pub fn open_by_title(&mut self, title: &str) -> bool {
        match self.menus.iter().position(|(t, _)| t == title) {
            Some(i) => {
                self.open = Some(i);
                self.open_path.clear();
                self.hover.clear();
                true
            }
            None => false,
        }
    }

    /// Sets the checkmark of every toggle item firing `action` (a deep walk) —
    /// hosts with live toggle state re-sync marks each frame before drawing.
    pub fn set_checked(&mut self, action: u64, checked: bool) {
        fn walk(items: &mut [MenuItem], action: u64, checked: bool) {
            for it in items {
                if it.action == Some(action) && it.checked.is_some() {
                    it.checked = Some(checked);
                }
                walk(&mut it.children, action, checked);
            }
        }
        for (_, items) in &mut self.menus {
            walk(items, action, checked);
        }
    }

    /// Replaces the whole menu set (closing any open cascade) — for hosts
    /// whose menu structure depends on app state (mode switches). For live
    /// enabled/checked flags on a stable structure, prefer
    /// [`set_item_enabled`](Self::set_item_enabled) /
    /// [`set_checked`](Self::set_checked), which keep an open cascade alive.
    pub fn set_menus(&mut self, menus: Vec<(String, Vec<MenuItem>)>) {
        self.close();
        self.hover_header = None;
        self.menus = menus;
        self.headers.clear();
    }

    /// Enables or disables every item firing `action` (a deep walk, the
    /// [`set_checked`](Self::set_checked) shape) — hosts with live gating
    /// re-sync flags as state changes; an open cascade stays open and the
    /// row dims in place.
    pub fn set_item_enabled(&mut self, action: u64, enabled: bool) {
        fn walk(items: &mut [MenuItem], action: u64, enabled: bool) {
            for it in items {
                if it.action == Some(action) {
                    it.enabled = enabled;
                }
                walk(&mut it.children, action, enabled);
            }
        }
        for (_, items) in &mut self.menus {
            walk(items, action, enabled);
        }
    }

    /// The bar's header titles, in order.
    #[must_use]
    pub fn titles(&self) -> Vec<&str> {
        self.menus.iter().map(|(t, _)| t.as_str()).collect()
    }

    /// The on-screen rect of the header titled `title` — a script driver
    /// presses its center to open the menu through the real event path.
    /// Valid after layout; `None` for an unknown title.
    #[must_use]
    pub fn header_rect(&self, title: &str) -> Option<Rect> {
        let i = self.menus.iter().position(|(t, _)| t == title)?;
        self.headers.get(i).copied()
    }

    /// The labels of every item in the open cascade, panel order (separators
    /// skipped) — the script/test story's "what could a user read right now"
    /// ([`ContextMenu::open_labels`]'s twin: menu items are data, not
    /// widgets, so they carry no semantics node). Empty while closed or
    /// before layout has built the panels.
    #[must_use]
    pub fn open_labels(&self) -> Vec<&str> {
        let mut out = Vec::new();
        for panel in &self.panels {
            for it in items_at(self.root(), &panel.path) {
                if !it.separator {
                    out.push(it.label.as_str());
                }
            }
        }
        out
    }

    /// The on-screen cell rect of the open-cascade item labeled `label`
    /// ([`ContextMenu::item_rect`]'s twin). `None` while closed, for a label
    /// not in an open panel, or before layout has built the panels.
    #[must_use]
    pub fn item_rect(&self, label: &str) -> Option<Rect> {
        for panel in &self.panels {
            let items = items_at(self.root(), &panel.path);
            for (i, it) in items.iter().enumerate() {
                if !it.separator && it.label == label {
                    return Some(panel.cell_rect(i, self.row_h));
                }
            }
        }
        None
    }

    fn root(&self) -> &[MenuItem] {
        self.open.map_or(&[], |m| self.menus[m].1.as_slice())
    }
}

impl Default for MenuBar {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for MenuBar {
    fn measure(&mut self, avail: Size, ctx: &mut LayoutCtx) -> Size {
        self.bar_h = self.fixed_bar_h.unwrap_or(ctx.theme.metrics().titlebar);
        self.row_h = ctx.theme.metrics().control_height;
        Size::new(avail.w, self.bar_h)
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = Rect::new(rect.x, rect.y, rect.w, self.bar_h);
        let font = ctx.theme.font();
        let px = ctx.theme.font_px(TextRole::Body);
        let pad = ctx.theme.metrics().pad;
        self.headers.clear();
        let mut x = rect.x;
        for (title, _) in &self.menus {
            let w = ctx.fonts.measure(font, title, px) + 2.0 * pad;
            self.headers.push(Rect::new(x, rect.y, w, self.bar_h));
            x += w;
        }
        self.panels = if let Some(m) = self.open {
            let anchor = Vec2::new(self.headers[m].x, self.rect.bottom());
            build_panels(
                &self.menus[m].1,
                anchor,
                &self.open_path,
                self.row_h,
                ctx.fonts,
                ctx.theme,
            )
        } else {
            Vec::new()
        };
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if ctx.is_base() {
            ctx.theme.panel(dl, self.rect);
            // Header highlights are flat row tints kept inside the bar's frame
            // ring — menu titles are not buttons, so they never grow a bevel
            // or cover the bar's own (only a real button covers its bevel).
            let inner = self
                .rect
                .inset(Insets::all(ctx.theme.metrics().modal_frame));
            for (i, (title, _)) in self.menus.iter().enumerate() {
                let hr = self.headers[i];
                if self.open == Some(i) {
                    ctx.theme
                        .accent_row(dl, hr.intersect(&inner), ROW_FLOOR_ACTIVE);
                } else if self.open.is_none() && self.hover_header == Some(i) {
                    ctx.theme
                        .accent_row(dl, hr.intersect(&inner), ROW_FLOOR_HOVER);
                }
                let px = ctx.theme.font_px(TextRole::Body);
                let pad = ctx.theme.metrics().pad;
                let baseline = Vec2::new(hr.x + pad, hr.center().y + px * 0.34);
                // Menubar titles are raised faces: full emboss.
                ctx.theme.text_em(
                    dl,
                    ctx.fonts,
                    baseline,
                    title,
                    TextRole::Body,
                    Emboss::Raised,
                );
            }
        } else if self.open.is_some() {
            draw_cascade(dl, ctx, self.root(), &self.panels, &self.hover, self.row_h);
        }
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        match ev {
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                ..
            } if self.open.is_none() && ctx.is_target(self.id) => {
                // Open the clicked title's menu.
                if let Some(i) = self.headers.iter().position(|h| h.contains(ctx.pointer)) {
                    self.open = Some(i);
                    self.open_path.clear();
                    self.hover.clear();
                    ctx.consume_pointer();
                    ctx.request_focus(self.id);
                    ctx.open_popup(self.id);
                    return true;
                }
                false
            }
            Event::PointerMoved { .. } if self.open.is_none() => {
                // Closed: track the hovered header for its highlight.
                let over = self
                    .headers
                    .iter()
                    .position(|h| h.contains(ctx.pointer))
                    .filter(|_| ctx.is_target(self.id));
                let changed = over != self.hover_header;
                self.hover_header = over;
                changed
            }
            _ if self.open.is_some() => match ev {
                Event::PointerMoved { .. } => {
                    // Switch menus by hovering the bar.
                    if let Some(i) = self.headers.iter().position(|h| h.contains(ctx.pointer)) {
                        if self.open != Some(i) {
                            self.open = Some(i);
                            self.open_path.clear();
                        }
                        self.hover.clear();
                    } else if let Over::Panel(pi, Some(row)) =
                        over_at(&self.panels, ctx.pointer, self.row_h)
                    {
                        let path = self.panels[pi].path.clone();
                        let info = items_at(self.root(), &path)
                            .get(row)
                            .map(|it| (it.separator, it.has_children(), it.enabled));
                        if let Some((sep, has_kids, enabled)) = info
                            && !sep
                            && enabled
                        {
                            let mut hp = path.clone();
                            hp.push(row);
                            self.hover = hp.clone();
                            // Open this item's submenu (or collapse deeper ones).
                            self.open_path = if has_kids { hp } else { path };
                        }
                    }
                    ctx.consume_pointer();
                    true
                }
                Event::PointerButton {
                    button: PointerButton::Primary,
                    pressed: true,
                    ..
                } => {
                    match over_at(&self.panels, ctx.pointer, self.row_h) {
                        Over::Panel(pi, Some(row)) => {
                            let path = self.panels[pi].path.clone();
                            let info = items_at(self.root(), &path)
                                .get(row)
                                .map(|it| (it.action, it.has_children(), it.enabled));
                            if let Some((action, has_kids, enabled)) = info
                                && enabled
                            {
                                if let Some(a) = action {
                                    ctx.fire(self.id, Some(a));
                                    self.close();
                                    ctx.close_popup();
                                } else if has_kids {
                                    let mut hp = path;
                                    hp.push(row);
                                    self.open_path = hp;
                                }
                            }
                        }
                        // Inside a panel but on no item (a columns header /
                        // empty tail): swallow the click, keep the menu open.
                        Over::Panel(_, None) => {}
                        Over::Outside => {
                            match self.headers.iter().position(|h| h.contains(ctx.pointer)) {
                                // Clicking the open title again closes its
                                // dropdown (the header is a toggle). The
                                // pointer is on this header, so refresh the
                                // closed-state hover highlight (it was last
                                // tracked before the menu opened).
                                Some(i) if self.open == Some(i) => {
                                    self.close();
                                    ctx.close_popup();
                                    self.hover_header = Some(i);
                                }
                                // A sibling title switches to its menu (hover
                                // usually got there first; presses without a
                                // preceding move land here).
                                Some(i) => {
                                    self.open = Some(i);
                                    self.open_path.clear();
                                    self.hover.clear();
                                }
                                None => {
                                    self.close();
                                    ctx.close_popup();
                                }
                            }
                        }
                    }
                    ctx.consume_pointer();
                    true
                }
                // Any other button dismisses the cascade wherever it lands —
                // a right-click never picks an item, and swallowing it keeps
                // it from acting on whatever lies underneath.
                Event::PointerButton { pressed: true, .. } => {
                    self.close();
                    ctx.close_popup();
                    ctx.consume_pointer();
                    true
                }
                Event::Key {
                    key: Key::Escape,
                    pressed: true,
                    ..
                } if ctx.is_target(self.id) => {
                    self.close();
                    ctx.close_popup();
                    ctx.consume_keyboard();
                    true
                }
                // Window focus lost: the dismissing click may never arrive —
                // drop the cascade now (the popup-owner contract on
                // [`EventCtx::open_popup`]).
                Event::Focus(false) => {
                    self.close();
                    ctx.close_popup();
                    false
                }
                // Losing the keyboard closes the cascade: open-but-unfocused
                // would eat every pointer event while Escape goes elsewhere.
                // On the blur path the request is ignored — the `Ui` drops the
                // routing state itself; this arm syncs the widget's own state.
                Event::Blur(_) if ctx.is_target(self.id) => {
                    self.close();
                    ctx.close_popup();
                    false
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn id(&self) -> WidgetId {
        self.id
    }

    fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
        self.rect.contains(pos).then_some(self.id)
    }
}

// --- ContextMenu ------------------------------------------------------------

/// Wraps content and pops a menu cascade at the pointer on a right-click.
#[must_use]
pub struct ContextMenu {
    id: WidgetId,
    content: Box<dyn Widget>,
    items: Vec<MenuItem>,
    open: bool,
    anchor: Vec2,
    open_path: Vec<usize>,
    hover: Vec<usize>,
    row_h: f32,
    panels: Vec<Panel>,
    rect: Rect,
}

impl ContextMenu {
    /// True while the menu is popped open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn new(items: Vec<MenuItem>, content: impl Widget + 'static) -> Self {
        Self {
            id: next_id(),
            content: Box::new(content),
            items,
            open: false,
            anchor: Vec2::ZERO,
            open_path: Vec::new(),
            hover: Vec::new(),
            row_h: 24.0,
            panels: Vec::new(),
            rect: Rect::ZERO,
        }
    }

    pub fn id(&self) -> WidgetId {
        self.id
    }

    /// Replaces the item tree (closing any open cascade) — for hosts whose
    /// menu contents depend on state snapshotted at open time.
    pub fn set_items(&mut self, items: Vec<MenuItem>) {
        self.close();
        self.items = items;
    }

    /// Opens the cascade at `pos` programmatically (a command-driven open, as
    /// a right-click inside the content would). Like
    /// [`MenuBar::open_by_title`], this skips the `Ui` popup registration, so
    /// it suits a menu hosted alone in its own `Ui` (the host routes events
    /// to it while open); inside a shared tree, prefer the built-in
    /// right-click open.
    pub fn open_at(&mut self, pos: Vec2) {
        self.open = true;
        self.anchor = pos;
        self.open_path.clear();
        self.hover.clear();
    }

    /// Closes the cascade (host-driven: Escape at the shell, a wheel scroll
    /// under a position-baked menu, an `… off` command).
    pub fn close(&mut self) {
        self.open = false;
        self.open_path.clear();
        self.hover.clear();
        self.panels.clear();
    }

    /// The labels of every item in the open cascade, panel order (separators
    /// skipped) — the script/test story's "what could a user read right now".
    /// Menu items are data, not widgets, so they carry no semantics node; this
    /// and [`item_rect`](Self::item_rect) are how a host or script driver
    /// addresses rows by the label a user reads. Empty while closed or before
    /// layout has built the panels.
    #[must_use]
    pub fn open_labels(&self) -> Vec<&str> {
        let mut out = Vec::new();
        for panel in &self.panels {
            for it in items_at(&self.items, &panel.path) {
                if !it.separator {
                    out.push(it.label.as_str());
                }
            }
        }
        out
    }

    /// The on-screen cell rect of the open-cascade item labeled `label` —
    /// a script driver presses its center to pick the row through the real
    /// event path. `None` while closed, for a label not in an open panel, or
    /// before layout has built the panels.
    #[must_use]
    pub fn item_rect(&self, label: &str) -> Option<Rect> {
        for panel in &self.panels {
            let items = items_at(&self.items, &panel.path);
            for (i, it) in items.iter().enumerate() {
                if !it.separator && it.label == label {
                    return Some(panel.cell_rect(i, self.row_h));
                }
            }
        }
        None
    }
}

impl Widget for ContextMenu {
    fn measure(&mut self, avail: Size, ctx: &mut LayoutCtx) -> Size {
        self.row_h = ctx.theme.metrics().control_height;
        self.content.measure(avail, ctx)
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        self.content.arrange(rect, ctx);
        self.panels = if self.open {
            // Keep the root panel on-screen (within this widget's rect): slide
            // left at the right edge, flip above the anchor at the bottom edge
            // (but never off the top).
            let w0 = panel_width(&self.items, ctx.fonts, ctx.theme);
            let h0 = row_offsets(&self.items, self.row_h)
                .last()
                .copied()
                .unwrap_or(0.0)
                + 2.0 * PANEL_VPAD;
            let mut anchor = self.anchor;
            anchor.x = anchor.x.min((rect.right() - w0).max(rect.x));
            if anchor.y + h0 > rect.bottom() {
                anchor.y = (anchor.y - h0).max(rect.y);
            }
            build_panels(
                &self.items,
                anchor,
                &self.open_path,
                self.row_h,
                ctx.fonts,
                ctx.theme,
            )
        } else {
            Vec::new()
        };
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if ctx.is_base() {
            self.content.draw(dl, ctx);
        } else {
            self.content.draw(dl, ctx); // forward overlay pass (nested popups)
            if self.open {
                draw_cascade(dl, ctx, &self.items, &self.panels, &self.hover, self.row_h);
            }
        }
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        if !self.open && self.content.event(ev, ctx) {
            return true;
        }
        if !self.open {
            // Open on a secondary press inside — but never while another popup
            // is live (its owner handles this press as an outside-click dismiss;
            // opening here would strand that owner's open flag).
            if let Event::PointerButton {
                button: PointerButton::Secondary,
                pressed: true,
                ..
            } = ev
                && self.rect.contains(ctx.pointer)
                && !ctx.any_popup_open()
            {
                self.open = true;
                self.anchor = ctx.pointer;
                self.open_path.clear();
                self.hover.clear();
                ctx.consume_pointer();
                ctx.request_focus(self.id);
                ctx.open_popup(self.id);
                return true;
            }
            return false;
        }
        // Open: the Ui routes pointer events here.
        match ev {
            Event::PointerMoved { .. } => {
                if let Over::Panel(pi, Some(row)) = over_at(&self.panels, ctx.pointer, self.row_h) {
                    let path = self.panels[pi].path.clone();
                    let info = items_at(&self.items, &path)
                        .get(row)
                        .map(|it| (it.separator, it.has_children(), it.enabled));
                    if let Some((sep, kids, enabled)) = info
                        && !sep
                        && enabled
                    {
                        let mut hp = path.clone();
                        hp.push(row);
                        self.hover = hp.clone();
                        self.open_path = if kids { hp } else { path };
                    }
                }
                ctx.consume_pointer();
                true
            }
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                ..
            } => {
                match over_at(&self.panels, ctx.pointer, self.row_h) {
                    Over::Panel(pi, Some(row)) => {
                        let path = self.panels[pi].path.clone();
                        let info = items_at(&self.items, &path)
                            .get(row)
                            .map(|it| (it.action, it.has_children(), it.enabled));
                        if let Some((action, kids, enabled)) = info
                            && enabled
                        {
                            if let Some(a) = action {
                                ctx.fire(self.id, Some(a));
                                self.close();
                                ctx.close_popup();
                            } else if kids {
                                let mut hp = path;
                                hp.push(row);
                                self.open_path = hp;
                            }
                        }
                    }
                    // On a panel but on no item: swallow, keep the menu open.
                    Over::Panel(_, None) => {}
                    Over::Outside => {
                        self.close();
                        ctx.close_popup();
                    }
                }
                ctx.consume_pointer();
                true
            }
            // A secondary press while open dismisses too (it can't mean a new
            // open — the cascade owns the pointer until it closes).
            Event::PointerButton {
                button: PointerButton::Secondary,
                pressed: true,
                ..
            } => {
                self.close();
                ctx.close_popup();
                ctx.consume_pointer();
                true
            }
            Event::Key {
                key: Key::Escape,
                pressed: true,
                ..
            } if ctx.is_target(self.id) => {
                self.close();
                ctx.close_popup();
                ctx.consume_keyboard();
                true
            }
            // Window focus lost: drop the cascade (the popup-owner contract on
            // [`EventCtx::open_popup`]) — and forward the event to the wrapped
            // content first, which is otherwise cut off while the cascade is
            // open and has its own disarming to do on this event.
            Event::Focus(false) => {
                self.content.event(ev, ctx);
                self.close();
                ctx.close_popup();
                false
            }
            // Losing the keyboard closes the cascade (see [`MenuBar`]'s arm —
            // same rationale, and the `Ui` drops the routing state itself on
            // the blur path).
            Event::Blur(_) if ctx.is_target(self.id) => {
                self.close();
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
        if !self.rect.contains(pos) {
            return None;
        }
        self.content.hit_test(pos).or(Some(self.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Ui;
    use crate::event::Modifiers;
    use crate::text::Fonts;
    use crate::theme::Gunmetal;

    const DEJAVU: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");

    fn themed() -> (Fonts, Gunmetal) {
        let mut fonts = Fonts::new();
        let font = fonts.add(DEJAVU.to_vec()).unwrap();
        (fonts, Gunmetal::new(font))
    }

    fn press(x: f32, y: f32) -> Event {
        Event::PointerButton {
            button: PointerButton::Primary,
            pressed: true,
            pos: Vec2::new(x, y),
            mods: Modifiers::NONE,
        }
    }

    fn moved(x: f32, y: f32) -> Event {
        Event::PointerMoved {
            pos: Vec2::new(x, y),
        }
    }

    #[test]
    fn panel_width_grows_with_shortcut_and_check() {
        let (fonts, theme) = themed();
        let plain = vec![MenuItem::item("Save", 1)];
        let with_sc = vec![MenuItem::item("Save", 1).shortcut("Ctrl+S")];
        let with_chk = vec![MenuItem::item("Grid", 1).checked(true)];
        let base = panel_width(&plain, &fonts, &theme);
        assert!(
            panel_width(&with_sc, &fonts, &theme) > base,
            "a shortcut hint widens the panel"
        );
        assert!(
            panel_width(&with_chk, &fonts, &theme) > base,
            "a check column widens the panel"
        );
    }

    /// One item carrying a stencil reserves the icon column for the whole
    /// panel, and drawing an open cascade stamps the stencil's quads.
    #[test]
    fn an_icon_column_widens_the_panel_and_stamps() {
        use crate::icon;

        let (fonts, theme) = themed();
        let plain = vec![MenuItem::item("Delete", 1), MenuItem::item("Rename", 2)];
        let iconed = vec![
            MenuItem::item("Delete", 1).icon(icon::TRASH),
            MenuItem::item("Rename", 2),
        ];
        assert_eq!(
            panel_width(&iconed, &fonts, &theme),
            panel_width(&plain, &fonts, &theme) + ICON_W,
            "an icon column widens the panel by exactly its width"
        );

        let mut cm = ContextMenu::new(iconed, crate::layout::Spacer::new());
        cm.open_at(Vec2::new(10.0, 10.0));
        let mut ui = Ui::new(cm);
        ui.layout(Size::new(300.0, 300.0), &theme, &fonts);
        let mut dl = DrawList::new();
        let plain_quads = {
            let mut cm = ContextMenu::new(
                vec![MenuItem::item("Delete", 1), MenuItem::item("Rename", 2)],
                crate::layout::Spacer::new(),
            );
            cm.open_at(Vec2::new(10.0, 10.0));
            let mut ui = Ui::new(cm);
            ui.layout(Size::new(300.0, 300.0), &theme, &fonts);
            let mut dl = DrawList::new();
            ui.draw(&mut dl, &theme, &fonts);
            dl.cmds.len()
        };
        ui.draw(&mut dl, &theme, &fonts);
        assert!(
            dl.cmds.len() > plain_quads,
            "the stamped stencil adds draw commands: {} vs {plain_quads}",
            dl.cmds.len()
        );
    }

    /// `open_labels`/`item_rect` are the host/script address surface: menu
    /// items are data (no semantics node), so a driver reads the open
    /// cascade's labels and presses a row's rect through the real path.
    #[test]
    fn open_labels_and_item_rect_address_the_open_cascade() {
        let (fonts, theme) = themed();
        let mut cm = ContextMenu::new(Vec::new(), crate::layout::Spacer::new());
        let id = cm.id();
        cm.set_items(vec![
            MenuItem::item("Rename", 1),
            MenuItem::separator(),
            MenuItem::item("Delete", 2),
        ]);
        assert!(cm.open_labels().is_empty(), "closed: nothing to read");
        assert!(cm.item_rect("Delete").is_none(), "closed: no rects");

        cm.open_at(Vec2::new(10.0, 10.0));
        let mut ui = Ui::new(cm);
        ui.layout(Size::new(300.0, 300.0), &theme, &fonts);
        let cm = ui.get::<ContextMenu>(id).unwrap();
        assert_eq!(
            cm.open_labels(),
            vec!["Rename", "Delete"],
            "panel order, separators skipped"
        );
        assert!(cm.item_rect("Bogus").is_none(), "unknown label refused");
        let r = cm.item_rect("Delete").unwrap();
        ui.dispatch(&[press(r.center().x, r.center().y)]);
        assert_eq!(ui.actions(), [2], "pressing the rect's center fires it");
    }

    #[test]
    fn disabled_item_does_not_fire_and_keeps_menu_open() {
        let (fonts, theme) = themed();
        let bar = MenuBar::new().menu(
            "File",
            vec![
                MenuItem::item("Save", 1).shortcut("Ctrl+S"),
                MenuItem::item("Paste", 2).enabled(false),
                MenuItem::item("Grid", 3).checked(true),
            ],
        );
        let id = bar.id();
        let mut ui = Ui::new(bar);

        let lay = |ui: &mut Ui| ui.layout(Size::new(300.0, 300.0), &theme, &fonts);
        lay(&mut ui);
        // Open the File menu (header at the top-left of the 22px bar).
        ui.dispatch(&[press(10.0, 10.0)]);
        lay(&mut ui); // build the cascade panels for the open menu

        // Rows are 24px tall below the bar (22px) and the panel pad (4px):
        // Save 26..50, Paste 50..74. Click the disabled Paste → nothing
        // fires, menu stays open.
        ui.dispatch(&[press(12.0, 58.0)]);
        assert!(
            ui.actions().is_empty(),
            "disabled item fires no action: {:?}",
            ui.actions()
        );

        // The menu is still open, so clicking enabled Save now fires action 1
        // (had the disabled click closed it, this row would be inert).
        ui.dispatch(&[press(12.0, 34.0)]);
        assert_eq!(ui.actions(), [1], "enabled Save fires");
        let _ = id;
    }

    #[test]
    fn separator_rows_are_slim_and_the_padding_is_inert() {
        let (fonts, theme) = themed();
        let mut cm = ContextMenu::new(Vec::new(), crate::layout::Spacer::new());
        let id = cm.id();
        cm.set_items(vec![
            MenuItem::item("Use", 1),
            MenuItem::separator(),
            MenuItem::item("Delete", 2),
        ]);
        cm.open_at(Vec2::new(10.0, 10.0));
        let mut ui = Ui::new(cm);
        ui.layout(Size::new(300.0, 300.0), &theme, &fonts);
        let p = ui.get::<ContextMenu>(id).unwrap().panels[0].rect;
        let row_h = theme.metrics().control_height;
        assert_eq!(
            p.h,
            2.0 * row_h + SEP_H + 2.0 * PANEL_VPAD,
            "two item rows + a slim separator + the panel pads"
        );

        // A click in the top padding is inside the panel but on no item: it
        // fires nothing and the cascade stays open.
        ui.dispatch(&[press(p.x + 4.0, p.y + PANEL_VPAD * 0.5)]);
        assert!(ui.actions().is_empty(), "the padding fires nothing");
        assert!(ui.get::<ContextMenu>(id).unwrap().is_open());

        // The row after the separator starts SEP_H (not a full row) below the
        // first row's end.
        ui.dispatch(&[press(p.x + 4.0, p.y + PANEL_VPAD + row_h + SEP_H + 4.0)]);
        assert_eq!(ui.actions(), [2], "Delete sits right after the separator");
    }

    #[test]
    fn host_apis_open_close_and_sync_checkmarks() {
        let bar = MenuBar::new()
            .menu("File", vec![MenuItem::item("Open", 1)])
            .menu(
                "View",
                vec![
                    MenuItem::item("Grid", 2).checked(false),
                    MenuItem::sub("More", vec![MenuItem::item("Pass", 3).checked(false)]),
                ],
            );
        let mut bar = bar;
        assert!(!bar.is_open());
        assert!(bar.open_by_title("View"), "known title opens");
        assert!(bar.is_open());
        assert!(!bar.open_by_title("Bogus"), "unknown title refused");
        bar.close();
        assert!(!bar.is_open());
        // set_checked walks nested items by action id.
        bar.set_checked(3, true);
        assert_eq!(bar.menus[1].1[1].children[0].checked, Some(true));
        bar.set_checked(2, true);
        assert_eq!(bar.menus[1].1[0].checked, Some(true));
        // A non-toggle item never grows a checkmark.
        bar.set_checked(1, true);
        assert_eq!(bar.menus[0].1[0].checked, None);
    }

    /// The bar's script/test address surface (the `ContextMenu` twins):
    /// `header_rect` presses open a menu through the real event path,
    /// `open_labels`/`item_rect` address the open cascade's rows, and the
    /// live-gating setters flip flags in place.
    #[test]
    fn bar_address_surface_and_live_gating() {
        let (fonts, theme) = themed();
        let bar = MenuBar::new()
            .menu(
                "File",
                vec![
                    MenuItem::item("Open", 1),
                    MenuItem::separator(),
                    MenuItem::item("Quit", 2),
                ],
            )
            .menu("Edit", vec![MenuItem::item("Undo", 3)]);
        let id = bar.id();
        let mut ui = Ui::new(bar);
        let lay = |ui: &mut Ui| ui.layout(Size::new(400.0, 300.0), &theme, &fonts);
        lay(&mut ui);

        {
            let bar = ui.get::<MenuBar>(id).unwrap();
            assert_eq!(bar.titles(), vec!["File", "Edit"]);
            assert!(bar.open_labels().is_empty(), "closed: nothing to read");
            assert!(bar.item_rect("Open").is_none());
            assert!(bar.header_rect("Bogus").is_none(), "unknown title refused");
        }

        // Press the Edit header's center → its menu opens for real.
        let hr = ui.get::<MenuBar>(id).unwrap().header_rect("Edit").unwrap();
        ui.dispatch(&[press(hr.center().x, hr.center().y)]);
        lay(&mut ui);
        assert_eq!(
            ui.get::<MenuBar>(id).unwrap().open_labels(),
            vec!["Undo"],
            "the open cascade's rows, separators skipped"
        );

        // Live gating: disable Undo in place — the cascade stays open and
        // the row no longer fires; re-enabled it fires again.
        ui.get_mut::<MenuBar>(id)
            .unwrap()
            .set_item_enabled(3, false);
        let r = ui.get::<MenuBar>(id).unwrap().item_rect("Undo").unwrap();
        ui.dispatch(&[press(r.center().x, r.center().y)]);
        assert!(ui.actions().is_empty(), "disabled row fires nothing");
        assert!(ui.get::<MenuBar>(id).unwrap().is_open());
        ui.get_mut::<MenuBar>(id).unwrap().set_item_enabled(3, true);
        ui.dispatch(&[press(r.center().x, r.center().y)]);
        assert_eq!(ui.actions(), [3], "re-enabled row fires");

        // set_menus replaces the structure and closes anything open.
        let bar = ui.get_mut::<MenuBar>(id).unwrap();
        bar.open_by_title("File");
        bar.set_menus(vec![("Help".to_string(), vec![MenuItem::item("About", 9)])]);
        assert!(!bar.is_open(), "set_menus closes the cascade");
        assert_eq!(bar.titles(), vec!["Help"]);
    }

    #[test]
    fn columns_submenu_lays_out_and_fires_per_column() {
        let (fonts, theme) = themed();
        let cols = vec![
            (
                "Small".to_string(),
                vec![MenuItem::item("A", 10), MenuItem::item("B", 11)],
            ),
            ("Large".to_string(), vec![MenuItem::item("C", 20)]),
        ];
        let bar = MenuBar::new().menu(
            "File",
            vec![MenuItem::item("New", 1), MenuItem::columns("Maps", cols)],
        );
        let mut ui = Ui::new(bar);
        let lay = |ui: &mut Ui| ui.layout(Size::new(500.0, 400.0), &theme, &fonts);
        lay(&mut ui);
        // Open File, then click the "Maps" columns item (row 1: y 50..74,
        // below the bar and the panel pad) to open its columns panel.
        ui.dispatch(&[press(10.0, 10.0)]);
        lay(&mut ui);
        ui.dispatch(&[press(12.0, 58.0)]);
        lay(&mut ui);
        assert!(
            ui.actions().is_empty(),
            "opening a columns submenu fires nothing"
        );

        // The columns panel anchors at the parent's right edge, top-padded so
        // its header band row (50..74) lines up with the Maps row. Clicking
        // the band is inert and keeps the cascade open. (The width fixture
        // needs a child so it reserves the arrow gutter like the real item.)
        let file_w = panel_width(
            &[
                MenuItem::item("New", 1),
                MenuItem::sub("Maps", vec![MenuItem::item("x", 99)]),
            ],
            &fonts,
            &theme,
        );
        let px = file_w + 8.0; // just inside the first column
        ui.dispatch(&[press(px, 58.0)]);
        assert!(ui.actions().is_empty(), "the header band is inert");

        // First column, first item row (74..98) → fires A (10).
        ui.dispatch(&[press(px, 82.0)]);
        assert_eq!(ui.actions(), [10], "column 1 row 1 fires its item");
    }

    #[test]
    fn arrow_gutter_is_reserved_only_where_submenus_exist() {
        let (fonts, theme) = themed();
        let plain = vec![MenuItem::item("Edit", 1)];
        let with_sub = vec![MenuItem::sub("Edit", vec![MenuItem::item("A", 2)])];
        assert!(
            panel_width(&with_sub, &fonts, &theme) > panel_width(&plain, &fonts, &theme),
            "a submenu reserves its arrow gutter; a plain panel does not"
        );
    }

    #[test]
    fn context_menu_opens_programmatically_clamps_and_fires() {
        let (fonts, theme) = themed();
        let mut cm = ContextMenu::new(Vec::new(), crate::layout::Spacer::new());
        let id = cm.id();
        // Items snapshot the host's state at open time.
        cm.set_items(vec![
            MenuItem::item("Use", 1),
            MenuItem::separator(),
            MenuItem::item("Delete", 2),
        ]);
        // A command-driven open near the bottom-right corner: the root panel
        // slides left and flips above the anchor to stay on-screen.
        cm.open_at(Vec2::new(295.0, 295.0));
        assert!(cm.is_open());
        let mut ui = Ui::new(cm);
        ui.layout(Size::new(300.0, 300.0), &theme, &fonts);
        let p = ui.get::<ContextMenu>(id).unwrap().panels[0].rect;
        assert!(
            p.right() <= 300.0 && p.bottom() <= 300.0,
            "root panel stays on-screen: {p:?}"
        );
        assert!(p.y < 295.0, "flipped above the anchor: {p:?}");

        // Clicking the first row fires its action and closes the cascade.
        ui.dispatch(&[press(p.x + 4.0, p.y + 4.0)]);
        assert_eq!(ui.actions(), [1], "the clicked item fires");
        assert!(!ui.get::<ContextMenu>(id).unwrap().is_open());

        // Reopen; an outside primary press dismisses without firing.
        ui.get_mut::<ContextMenu>(id)
            .unwrap()
            .open_at(Vec2::new(10.0, 10.0));
        ui.layout(Size::new(300.0, 300.0), &theme, &fonts);
        ui.dispatch(&[press(290.0, 290.0)]);
        assert!(ui.actions().is_empty(), "outside click fires nothing");
        assert!(!ui.get::<ContextMenu>(id).unwrap().is_open());
    }

    /// A stale open path naming a childless submenu stops the cascade there
    /// instead of building an empty panel.
    #[test]
    fn build_panels_stops_at_an_empty_submenu() {
        let (fonts, theme) = themed();
        let root = vec![MenuItem::sub("Empty", Vec::new())];
        let panels = build_panels(&root, Vec2::ZERO, &[0], 24.0, &fonts, &theme);
        assert_eq!(
            panels.len(),
            1,
            "no child panel for an item without children"
        );
    }

    /// Hovering a separator or a disabled row leaves the previous hover in
    /// place — they are not hover targets.
    #[test]
    fn separators_and_disabled_rows_are_not_hover_targets() {
        let (fonts, theme) = themed();
        let bar = MenuBar::new().menu(
            "File",
            vec![
                MenuItem::item("Save", 1),
                MenuItem::separator(),
                MenuItem::item("Paste", 2).enabled(false),
            ],
        );
        let id = bar.id();
        let mut ui = Ui::new(bar);
        let lay = |ui: &mut Ui| ui.layout(Size::new(300.0, 300.0), &theme, &fonts);
        lay(&mut ui);
        ui.dispatch(&[press(10.0, 10.0)]); // open "File"
        lay(&mut ui);

        // Rows below the 22px bar + 4px panel pad: Save 26..50, the slim
        // separator 50..56, Paste 56..80.
        ui.dispatch(&[moved(12.0, 38.0)]); // Save
        assert_eq!(ui.get::<MenuBar>(id).unwrap().hover, vec![0]);
        ui.dispatch(&[moved(12.0, 53.0)]); // the separator
        assert_eq!(
            ui.get::<MenuBar>(id).unwrap().hover,
            vec![0],
            "a separator does not steal the hover"
        );
        ui.dispatch(&[moved(12.0, 68.0)]); // the disabled Paste
        assert_eq!(
            ui.get::<MenuBar>(id).unwrap().hover,
            vec![0],
            "a disabled row does not steal the hover"
        );
        // Moving into empty space (no header, no panel) changes nothing: the
        // cascade stays open on the same hover until a click dismisses it.
        ui.dispatch(&[moved(250.0, 250.0)]);
        let bar = ui.get::<MenuBar>(id).unwrap();
        assert_eq!(bar.hover, vec![0], "empty space keeps the hover");
        assert!(bar.is_open(), "…and the cascade open");
    }

    /// Hover walks a context cascade: a submenu row opens its child panel, a
    /// leaf row collapses deeper panels, separators/disabled rows and empty
    /// space are ignored; a press on the submenu row opens it too, a press on
    /// a disabled row is inert, and a press on the child leaf fires.
    #[test]
    fn context_menu_hover_and_press_navigate_the_cascade() {
        let (fonts, theme) = themed();
        let mut cm = ContextMenu::new(Vec::new(), crate::layout::Spacer::new());
        let id = cm.id();
        cm.set_items(vec![
            MenuItem::item("Cut", 1),
            MenuItem::sub("More", vec![MenuItem::item("A", 2)]),
            MenuItem::separator(),
            MenuItem::item("Off", 3).enabled(false),
        ]);
        cm.open_at(Vec2::new(10.0, 10.0));
        let mut ui = Ui::new(cm);
        let lay = |ui: &mut Ui| ui.layout(Size::new(300.0, 300.0), &theme, &fonts);
        lay(&mut ui);

        let row_h = theme.metrics().control_height;
        let p0 = ui.get::<ContextMenu>(id).unwrap().panels[0].rect;
        let more = Vec2::new(p0.x + 8.0, p0.y + PANEL_VPAD + 1.5 * row_h);
        ui.dispatch(&[moved(more.x, more.y)]);
        lay(&mut ui);
        {
            let cm = ui.get::<ContextMenu>(id).unwrap();
            assert_eq!(
                cm.hover,
                vec![1],
                "the hovered submenu row is the hover path"
            );
            assert_eq!(cm.panels.len(), 2, "hovering a submenu opens its panel");
        }

        // Rows below "More": the slim separator, then the disabled "Off" —
        // neither steals the hover; nor does empty space beside the panels.
        let sep_y = p0.y + PANEL_VPAD + 2.0 * row_h + SEP_H * 0.5;
        ui.dispatch(&[moved(p0.x + 8.0, sep_y)]);
        assert_eq!(
            ui.get::<ContextMenu>(id).unwrap().hover,
            vec![1],
            "a separator does not steal the hover"
        );
        let off = Vec2::new(p0.x + 8.0, sep_y + SEP_H * 0.5 + 0.5 * row_h);
        ui.dispatch(&[moved(off.x, off.y)]);
        assert_eq!(
            ui.get::<ContextMenu>(id).unwrap().hover,
            vec![1],
            "a disabled row does not steal the hover"
        );
        ui.dispatch(&[moved(250.0, 250.0)]);
        assert_eq!(
            ui.get::<ContextMenu>(id).unwrap().hover,
            vec![1],
            "empty space keeps the hover"
        );

        // A press on the disabled row is inert: nothing fires, still open.
        ui.dispatch(&[press(off.x, off.y)]);
        assert!(ui.actions().is_empty(), "a disabled row never fires");
        assert!(ui.get::<ContextMenu>(id).unwrap().is_open());
        lay(&mut ui);

        ui.dispatch(&[moved(p0.x + 8.0, p0.y + PANEL_VPAD + 0.5 * row_h)]); // leaf "Cut"
        lay(&mut ui);
        {
            let cm = ui.get::<ContextMenu>(id).unwrap();
            assert_eq!(cm.hover, vec![0], "hover moves to the leaf");
            assert_eq!(
                cm.panels.len(),
                1,
                "leaving the submenu row collapses its panel"
            );
        }

        ui.dispatch(&[press(more.x, more.y)]); // press the submenu row itself
        lay(&mut ui);
        assert!(
            ui.actions().is_empty(),
            "opening a submenu by press fires nothing"
        );
        let p1 = ui.get::<ContextMenu>(id).unwrap().panels[1].rect;
        ui.dispatch(&[press(p1.x + 8.0, p1.y + PANEL_VPAD + 0.5 * row_h)]);
        assert_eq!(ui.actions(), [2], "the child leaf fires");
        assert!(
            !ui.get::<ContextMenu>(id).unwrap().is_open(),
            "firing closes the cascade"
        );
    }

    /// An open cascade is a pointer grab (`Response::capturing`), and it does
    /// not survive the window's focus: the dismissing click may land in
    /// another window and never arrive, so `Focus(false)` drops the cascade on
    /// both sides — the widget's flag and the `Ui`'s routing state.
    #[test]
    fn menubar_cascade_reports_the_grab_and_drops_on_focus_loss() {
        let (fonts, theme) = themed();
        let bar = MenuBar::new().menu("File", vec![MenuItem::item("Open", 1)]);
        let id = bar.id();
        let mut ui = Ui::new(bar);
        ui.layout(Size::new(300.0, 300.0), &theme, &fonts);

        let r = ui.dispatch(&[press(10.0, 10.0)]); // click-open registers the popup
        assert!(ui.get::<MenuBar>(id).unwrap().is_open());
        assert!(r.capturing, "an open cascade is a pointer grab");

        let r = ui.dispatch(&[Event::Focus(false)]);
        assert!(
            !ui.get::<MenuBar>(id).unwrap().is_open(),
            "focus loss closes the cascade"
        );
        assert!(!ui.popup_open(), "…and the Ui's popup with it");
        assert!(!r.capturing, "…releasing the grab");
    }

    /// Losing the keyboard closes the cascade: open-but-unfocused would eat
    /// every pointer event while Escape goes to whatever got the keyboard.
    #[test]
    fn menubar_cascade_closes_when_its_owner_is_blurred() {
        use crate::event::BlurCause;

        let (fonts, theme) = themed();
        let bar = MenuBar::new().menu("File", vec![MenuItem::item("Open", 1)]);
        let id = bar.id();
        let mut ui = Ui::new(bar);
        ui.layout(Size::new(300.0, 300.0), &theme, &fonts);

        ui.dispatch(&[press(10.0, 10.0)]);
        assert!(ui.popup_open());
        ui.blur(BlurCause::Moved);
        assert!(!ui.popup_open(), "the Ui dropped the popup with the focus");
        assert!(
            !ui.get::<MenuBar>(id).unwrap().is_open(),
            "the owner's flag followed"
        );
    }

    /// A non-primary press dismisses the cascade wherever it lands — a
    /// right-click on an item row never fires it, and it must not act on
    /// whatever lies underneath either.
    #[test]
    fn a_secondary_press_dismisses_an_open_cascade() {
        let (fonts, theme) = themed();
        let bar = MenuBar::new().menu("File", vec![MenuItem::item("Open", 1)]);
        let id = bar.id();
        let mut ui = Ui::new(bar);
        ui.layout(Size::new(300.0, 300.0), &theme, &fonts);

        ui.dispatch(&[press(10.0, 10.0)]);
        assert!(ui.get::<MenuBar>(id).unwrap().is_open());
        let right = Event::PointerButton {
            button: PointerButton::Secondary,
            pressed: true,
            pos: Vec2::new(12.0, 34.0), // on the "Open" row
            mods: Modifiers::NONE,
        };
        let r = ui.dispatch(&[right]);
        assert!(
            !ui.get::<MenuBar>(id).unwrap().is_open(),
            "a secondary press dismisses"
        );
        assert!(ui.actions().is_empty(), "nothing fired");
        assert!(r.pointer, "…and it is swallowed");
    }

    /// The same contract for a right-click-opened `ContextMenu` — and the
    /// wrapped content still hears `Focus(false)` (it has its own disarming to
    /// do), which the open cascade otherwise cuts off.
    #[test]
    fn context_menu_drops_on_focus_loss() {
        let (fonts, theme) = themed();
        let cm = ContextMenu::new(vec![MenuItem::item("Use", 1)], crate::layout::Spacer::new());
        let id = cm.id();
        let mut ui = Ui::new(cm);
        ui.layout(Size::new(300.0, 300.0), &theme, &fonts);

        let right = Event::PointerButton {
            button: PointerButton::Secondary,
            pressed: true,
            pos: Vec2::new(150.0, 150.0),
            mods: Modifiers::NONE,
        };
        let r = ui.dispatch(&[right]);
        assert!(ui.get::<ContextMenu>(id).unwrap().is_open());
        assert!(r.capturing, "an open context cascade is a pointer grab");

        let r = ui.dispatch(&[Event::Focus(false)]);
        assert!(
            !ui.get::<ContextMenu>(id).unwrap().is_open(),
            "focus loss closes the cascade"
        );
        assert!(!ui.popup_open() && !r.capturing, "the grab is released");
    }
}
