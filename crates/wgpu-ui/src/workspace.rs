//! A **dockable workspace**: four edge docks (left / right / top / bottom)
//! around a central content "hole", plus a floating layer of in-app windows.
//! This is a *different* docking paradigm from [`crate::dock`] (a VS-Code-style
//! split-tree of tabbed leaves): here panels dock to a screen edge, stack along
//! it with resizer splitters, or float freely; dragging a titlebar past a small
//! threshold undocks a panel into the floating layer, dragging near an edge
//! *peeks* that dock open as a drop target, and dropping inside re-docks at a
//! midpoint-based insert position. Empty docks auto-hide; a close glyph hides a
//! panel (re-show it with [`show`](Workspace::show)).
//!
//! **Model vs. presentation.** The docking *model* — geometry, hit-testing,
//! drag/resize/dock/float input, visibility, and a serializable
//! [`WorkspaceLayout`] — is pure, headless-testable logic and lives here. A
//! panel is **metadata only** (`id` / `title` / place / sizes); its *content* is
//! rendered by the host into [`body_of`](Workspace::body_of), addressed by the
//! [`Press::Body`] a press reports. This keeps a host free to interleave its own
//! native passes and per-panel widget trees — the reason panels are not
//! `Box<dyn Widget>` the way [`crate::dock`]'s are. A generic [`Widget`] impl
//! draws plain themed chrome for simple hosts; a host that owns a richer look
//! (a textured material, an anchored grain) draws its own chrome from
//! [`layout`](Workspace::layout) instead.

use crate::draw::DrawList;
use crate::event::{Event, PointerButton};
use crate::geom::{Rect, Size, Vec2};
use crate::interact::{CursorIcon, WidgetId, next_id};
use crate::theme::{Bevel, TextRole};
use crate::widget::{DrawCtx, EventCtx, LayoutCtx, Widget};

/// Dock side indices (into the per-side arrays on [`Layout`]).
pub const LEFT: usize = 0;
pub const RIGHT: usize = 1;
pub const TOP: usize = 2;
pub const BOTTOM: usize = 3;

const SPLIT: f32 = 6.0; // splitter / dock-edge resizer thickness
const PEEK_DIST: f32 = 32.0; // edge proximity that peeks a dock during a drag
const DRAG_START: f32 = 3.0; // titlebar movement before a drag undocks
const MIN_PANEL: f32 = 50.0; // min extent of a docked panel
const MIN_DOCK: f32 = 120.0;
const MAX_DOCK: f32 = 520.0;
const HANDLE: f32 = 14.0; // floating resize-handle square
const FRAME_FLOAT: f32 = 2.0; // floating-window border ring (also content margin)
const FRAME_DOCK: f32 = 2.0; // docked-panel border ring
const MIN_VISIBLE: f32 = 32.0; // a floating window must keep this much on-screen

/// Where a floating panel's top-left `(x, y)` may actually sit, given the panel
/// width `pw` and a `w`x`h` window with a `top`-px reserved strip.
///
/// Two rules, and they are not symmetric. Horizontally and downwards a panel may
/// hang off the window as long as [`MIN_VISIBLE`] px of it stays inside — that is
/// how you park one mostly off-screen. **Upwards it may not move at all**: the
/// reserved strip is the host's menu bar and tab strip, and a titlebar dragged
/// under them is both unreadable and ungrabbable, so the panel could never be
/// moved back. `top` is therefore a hard floor, not a "keep some of it visible"
/// allowance.
///
/// The one rule, applied in three places — the undock, every move of a floating
/// drag, and [`Workspace::clamp_floating`] on load / resize — so a drag can never
/// leave a panel somewhere the next load would refuse to restore it to.
fn float_clamp(top: f32, pw: f32, x: f32, y: f32, w: f32, h: f32) -> (f32, f32) {
    let x_lo = MIN_VISIBLE - pw;
    (
        x.clamp(x_lo, (w - MIN_VISIBLE).max(x_lo)),
        y.clamp(top, (h - MIN_VISIBLE).max(top)),
    )
}

/// Titlebar height (logical px) — the drag/close band at a panel's top. A
/// model constant on purpose: the workspace geometry is pure and theme-free
/// (headless-testable, host-owned chrome), so it does NOT read
/// `Metrics::titlebar` — a host drawing its own chrome must keep the two in
/// agreement (both default to 22).
pub const TITLEBAR_H: f32 = 22.0;
/// Left inset of the titlebar's title text (a touch wider than the border).
pub const TITLE_PAD: f32 = 12.0;

// ----- panel chrome geometry (a panel rect + its border-ring width) ----------

/// The content box inside a panel's `frame`-px border ring.
fn content_box(r: Rect, frame: f32) -> Rect {
    Rect::new(
        r.x + frame,
        r.y + frame,
        (r.w - 2.0 * frame).max(0.0),
        (r.h - 2.0 * frame).max(0.0),
    )
}

/// The full titlebar band (drag handle + close) inside the border ring.
fn titlebar_band(r: Rect, frame: f32) -> Rect {
    let c = content_box(r, frame);
    Rect::new(c.x, c.y, c.w, TITLEBAR_H)
}

/// A panel's content area: inside the border, below the titlebar.
fn body_rect(r: Rect, frame: f32) -> Rect {
    let c = content_box(r, frame);
    Rect::new(c.x, c.y + TITLEBAR_H, c.w, (c.h - TITLEBAR_H).max(0.0))
}

/// The titlebar close-button hit area — the right [`TITLEBAR_H`] square of the band.
fn close_rect(r: Rect, frame: f32) -> Rect {
    let bar = titlebar_band(r, frame);
    Rect::new(bar.x + bar.w - TITLEBAR_H, bar.y, TITLEBAR_H, TITLEBAR_H)
}

/// The titlebar drag handle (the band minus the close square).
fn titlebar_rect(r: Rect, frame: f32) -> Rect {
    let bar = titlebar_band(r, frame);
    Rect::new(bar.x, bar.y, (bar.w - TITLEBAR_H).max(0.0), bar.h)
}

/// A panel's min size along a dock's stacking axis (height for L/R, width T/B).
fn along_min(min: (f32, f32), side: usize) -> f32 {
    if side == LEFT || side == RIGHT {
        min.1
    } else {
        min.0
    }
}

/// A panel's min size across a dock's axis (width for L/R, height for T/B).
fn cross_min(min: (f32, f32), side: usize) -> f32 {
    if side == LEFT || side == RIGHT {
        min.0
    } else {
        min.1
    }
}

/// Where a panel lives. `Floating` holds its top-left; size is per-panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place {
    /// Docked to a side ([`LEFT`]/[`RIGHT`]/[`TOP`]/[`BOTTOM`]).
    Docked(usize),
    /// Floating at an absolute `(x, y)` top-left.
    Floating(f32, f32),
    /// Off-screen (the Windows-menu / `show` re-opens it to its `prev` place).
    Hidden,
}

/// Why a [`Workspace`] request failed. The host renders its own message (a
/// console line, a status-bar note) from the payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceError {
    /// No panel is registered under this id; `known` carries the registered
    /// ids for the host's message.
    UnknownPanel { id: String, known: Vec<String> },
}

/// A panel to add to a [`Workspace`] — metadata only (content is host-drawn).
/// Built fluently: `PanelSpec::new(id, title).place(..).size(..).extent(..).bounds(..)`.
#[derive(Clone, Debug)]
pub struct PanelSpec {
    id: String,
    title: String,
    hint: String,
    place: Place,
    prev: Place,
    w: f32,
    h: f32,
    extent: f32,
    min: (f32, f32),
    max: (f32, f32),
}

impl PanelSpec {
    /// A panel with `id` (routing + serialization key) and display `title`.
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            hint: String::new(),
            place: Place::Hidden,
            prev: Place::Hidden,
            w: 240.0,
            h: 200.0,
            extent: 200.0,
            min: (100.0, 80.0),
            max: (800.0, 800.0),
        }
    }

    /// A placeholder line drawn in the body until the host renders content.
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = hint.into();
        self
    }

    /// The initial place (also seeds `prev`, where `show` restores a hide to).
    pub fn place(mut self, place: Place) -> Self {
        self.place = place;
        self.prev = place;
        self
    }

    /// Overrides where a hidden panel restores to (defaults to [`place`](Self::place)).
    pub fn prev(mut self, prev: Place) -> Self {
        self.prev = prev;
        self
    }

    /// Floating size (logical px).
    pub fn size(mut self, w: f32, h: f32) -> Self {
        self.w = w;
        self.h = h;
        self
    }

    /// Docked extent along the dock's stacking axis (logical px).
    pub fn extent(mut self, extent: f32) -> Self {
        self.extent = extent;
        self
    }

    /// Sensible `(w, h)` size bounds so content can't overflow a too-small
    /// window nor a window grow absurdly large.
    pub fn bounds(mut self, min: (f32, f32), max: (f32, f32)) -> Self {
        self.min = min;
        self.max = max;
        self
    }
}

/// A live panel in a [`Workspace`]. Fields are public for a host to read while
/// rendering content; construct via [`PanelSpec`] + [`Workspace::panel`].
pub struct Panel {
    pub id: String,
    pub title: String,
    /// Body placeholder until the host draws real content.
    pub hint: String,
    pub place: Place,
    /// Where `show(id, Some(true))` restores a hidden panel to.
    prev: Place,
    /// Floating size.
    pub w: f32,
    pub h: f32,
    /// Docked extent along the dock's stacking axis.
    pub extent: f32,
    pub min: (f32, f32),
    pub max: (f32, f32),
}

#[derive(Clone, Copy)]
enum Drag {
    None,
    /// Titlebar drag: grab offset within the panel, undocked yet?
    Move {
        panel: usize,
        grab: (f32, f32),
        start: (f32, f32),
        moved: bool,
    },
    DockEdge {
        side: usize,
    },
    /// Resize the `nth` docked panel of `side` via the splitter below/right.
    Splitter {
        side: usize,
        nth: usize,
    },
    FloatResize {
        panel: usize,
    },
}

/// What a primary-button press hit.
#[derive(Debug, Clone, PartialEq)]
pub enum Press {
    None,
    /// Titlebar / close / splitter / resizer — handled internally by the model.
    Chrome,
    /// A panel body — the host routes content interaction into `body`.
    Body {
        id: String,
        body: Rect,
    },
}

/// One frame's computed geometry (also the hit-test source).
pub struct Layout {
    /// The central content "hole" a host renders into (a map, a canvas, …).
    pub center: Rect,
    /// Visible dock areas (including drag-peeked empty ones).
    pub docks: [Option<Rect>; 4],
    /// Docked panels first (vec order), floating after — also draw order.
    pub panels: Vec<(usize, Rect)>,
    /// `(side, nth docked panel it resizes, rect)`.
    pub splitters: Vec<(usize, usize, Rect)>,
    /// Dock-edge resizer strips.
    pub edges: [Option<Rect>; 4],
}

/// One panel's persistable state (place + sizes), keyed by id.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelLayout {
    pub id: String,
    pub place: Place,
    pub w: f32,
    pub h: f32,
    pub extent: f32,
}

/// A serializable snapshot of the whole layout (the host maps this to/from its
/// own persistence format — the boundary [`crate::dock::DockLayout`] also keeps).
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceLayout {
    pub dock_size: [f32; 4],
    pub panels: Vec<PanelLayout>,
}

/// A dockable workspace. Add panels with [`panel`](Self::panel); drive input
/// with [`on_press`](Self::on_press)/[`on_move`](Self::on_move)/[`on_release`](Self::on_release)
/// and read [`layout`](Self::layout) for geometry, or host it as a [`Widget`].
pub struct Workspace {
    id: WidgetId,
    pub panels: Vec<Panel>,
    /// Reserved strip above the docks (e.g. a menu bar). `0` by default.
    pub top: f32,
    /// Reserved strip below the docks (e.g. a status bar). `0` by default.
    pub bottom: f32,
    dock_size: [f32; 4],
    drag: Drag,
    /// Where the pointer is, or `None` once it has left — the chrome hover a
    /// host painting the frame itself asks about, and the anchor a dock peek
    /// measures from. See [`close_hovered`](Self::close_hovered).
    cursor: Option<Vec2>,
    /// The arranged rect (for the [`Widget`] impl); the pure API takes `w`/`h`.
    rect: Rect,
    /// The last body press, for a [`Widget`] host to poll (`take_press`).
    press: Option<Press>,
}

impl Workspace {
    /// An empty workspace (add panels with [`panel`](Self::panel)).
    pub fn new() -> Self {
        Self {
            id: next_id(),
            panels: Vec::new(),
            top: 0.0,
            bottom: 0.0,
            dock_size: [240.0, 280.0, 130.0, 150.0],
            drag: Drag::None,
            cursor: None,
            rect: Rect::ZERO,
            press: None,
        }
    }

    /// Adds a panel (builder-style).
    pub fn panel(mut self, spec: PanelSpec) -> Self {
        self.panels.push(Panel {
            id: spec.id,
            title: spec.title,
            hint: spec.hint,
            place: spec.place,
            prev: spec.prev,
            w: spec.w,
            h: spec.h,
            extent: spec.extent,
            min: spec.min,
            max: spec.max,
        });
        self
    }

    /// The widget id (for [`Ui`](crate::ui::Ui) hosting / polling).
    pub fn id(&self) -> WidgetId {
        self.id
    }

    /// Sets the per-side dock cross-axis sizes `[left, right, top, bottom]`.
    pub fn set_dock_size(&mut self, dock_size: [f32; 4]) {
        self.dock_size = dock_size;
    }

    /// The per-side dock cross-axis sizes.
    pub fn dock_size(&self) -> [f32; 4] {
        self.dock_size
    }

    pub fn find(&self, id: &str) -> Option<usize> {
        self.panels.iter().position(|p| p.id == id)
    }

    /// Is panel `id` currently on screen (not hidden)?
    pub fn is_visible(&self, id: &str) -> bool {
        self.find(id)
            .is_some_and(|i| self.panels[i].place != Place::Hidden)
    }

    /// Show/hide a panel (`None` toggles). Returns whether the panel is now
    /// visible; the host formats its own console/UI feedback.
    pub fn show(&mut self, id: &str, on: Option<bool>) -> Result<bool, WorkspaceError> {
        let Some(i) = self.find(id) else {
            return Err(self.unknown(id));
        };
        let p = &mut self.panels[i];
        let visible = p.place != Place::Hidden;
        let want = on.unwrap_or(!visible);
        if want && !visible {
            p.place = p.prev;
        } else if !want && visible {
            p.prev = p.place;
            p.place = Place::Hidden;
        }
        Ok(want)
    }

    /// Dock a panel to a side, or float it. This also becomes the panel's
    /// restore place — express visibility via [`show`](Self::show) rather than
    /// `Place::Hidden` here (which would also clobber the restore place). The
    /// host parses its own console/config strings into a [`Place`].
    pub fn dock_to(&mut self, id: &str, place: Place) -> Result<(), WorkspaceError> {
        let Some(i) = self.find(id) else {
            return Err(self.unknown(id));
        };
        self.panels[i].place = place;
        self.panels[i].prev = place;
        Ok(())
    }

    fn unknown(&self, id: &str) -> WorkspaceError {
        WorkspaceError::UnknownPanel {
            id: id.to_string(),
            known: self.ids().into_iter().map(String::from).collect(),
        }
    }

    fn ids(&self) -> Vec<&str> {
        self.panels.iter().map(|p| p.id.as_str()).collect()
    }

    /// Panels docked to `side`, in stacking order.
    fn docked(&self, side: usize) -> Vec<usize> {
        self.panels
            .iter()
            .enumerate()
            .filter(|(_, p)| p.place == Place::Docked(side))
            .map(|(i, _)| i)
            .collect()
    }

    /// A dock is force-shown while a titlebar drag hovers near its edge.
    fn peek(&self, w: f32, h: f32) -> [bool; 4] {
        let Drag::Move { moved: true, .. } = self.drag else {
            return [false; 4];
        };
        // A drag holds the pointer, so `cursor` is always live here; a `None`
        // (the pointer left mid-drag) simply peeks nothing.
        let Some(Vec2 { x: cx, y: cy }) = self.cursor else {
            return [false; 4];
        };
        [
            cx <= PEEK_DIST,
            w - cx <= PEEK_DIST,
            cy <= self.top + PEEK_DIST,
            h - cy <= PEEK_DIST,
        ]
    }

    /// How one dock's stacked panels share `avail` (its extent along the
    /// stacking axis, splitters already deducted).
    ///
    /// Each panel asks for its saved `extent` and the last one asks for the
    /// remainder — but **the sum has to be `avail`**, whatever they ask for.
    /// Letting the asks stand is what used to put a third panel opened into a
    /// two-panel-full dock at `dock.y + 795`: still "visible" to the model,
    /// dispatched to, and entirely off the screen. So an over-subscribed dock
    /// shrinks: every panel scales toward the space there is, and one that hits
    /// [`MIN_PANEL`] pins there while the rest re-share what is left (a dock too
    /// small even for that splits evenly). An *under*-subscribed one is already
    /// exact — the last panel's remainder absorbs the slack.
    fn stack_extents(&self, ids: &[usize], avail: f32) -> Vec<f32> {
        let n = ids.len();
        let mut ext: Vec<f32> = ids
            .iter()
            .map(|&i| self.panels[i].extent.max(MIN_PANEL))
            .collect();
        // The last panel takes whatever the others leave (never below the floor).
        let fixed: f32 = ext[..n - 1].iter().sum();
        ext[n - 1] = (avail - fixed).max(MIN_PANEL);

        // Water-fill down to `avail`. `floor` is MIN_PANEL unless the dock is too
        // small to give everyone that much, in which case an even split is the
        // best answer available and the loop reaches it in one pass.
        let floor = MIN_PANEL.min(avail / n as f32).max(0.0);
        let mut pinned = vec![false; n];
        loop {
            let (mut held, mut flex) = (0.0, 0.0);
            for (e, &p) in ext.iter().zip(&pinned) {
                if p { held += e } else { flex += e }
            }
            // Fits (or nothing left to give): done.
            if flex <= 0.0 || held + flex <= avail {
                break;
            }
            let k = (avail - held).max(0.0) / flex;
            // Pin whatever this pass would push under the floor, then re-share
            // among the rest; when none would, apply the scale and stop.
            let sinking: Vec<usize> = (0..n)
                .filter(|&j| !pinned[j] && ext[j] * k < floor)
                .collect();
            if sinking.is_empty() {
                for (e, &p) in ext.iter_mut().zip(&pinned) {
                    if !p {
                        *e *= k;
                    }
                }
                break;
            }
            for j in sinking {
                ext[j] = floor;
                pinned[j] = true;
            }
        }
        ext
    }

    /// Compute the frame's geometry for a `w`×`h` screen.
    pub fn layout(&self, w: f32, h: f32) -> Layout {
        // Reserve the bottom strip: docks + the center area live above it.
        let h = (h - self.bottom).max(1.0);
        let peek = self.peek(w, h);
        let occupied: [Vec<usize>; 4] = [
            self.docked(0),
            self.docked(1),
            self.docked(2),
            self.docked(3),
        ];
        let visible = [
            !occupied[0].is_empty() || peek[0],
            !occupied[1].is_empty() || peek[1],
            !occupied[2].is_empty() || peek[2],
            !occupied[3].is_empty() || peek[3],
        ];
        let size = |side: usize| {
            if visible[side] {
                self.dock_size[side].min(match side {
                    TOP | BOTTOM => (h - MIN_DOCK).max(MIN_DOCK),
                    _ => (w - MIN_DOCK).max(MIN_DOCK),
                })
            } else {
                0.0
            }
        };
        let (lw, rw, th, bh) = (size(LEFT), size(RIGHT), size(TOP), size(BOTTOM));

        let mut docks = [None; 4];
        let mut edges = [None; 4];
        // Top/bottom span the full width; left/right fill the middle band. All
        // of it sits below the reserved `top` strip.
        if visible[TOP] {
            docks[TOP] = Some(Rect::new(0.0, self.top, w, th));
            edges[TOP] = Some(Rect::new(0.0, self.top + th, w, SPLIT));
        }
        if visible[BOTTOM] {
            docks[BOTTOM] = Some(Rect::new(0.0, h - bh, w, bh));
            edges[BOTTOM] = Some(Rect::new(0.0, h - bh - SPLIT, w, SPLIT));
        }
        let y0 = self.top + th + if visible[TOP] { SPLIT } else { 0.0 };
        let y1 = h - bh - if visible[BOTTOM] { SPLIT } else { 0.0 };
        if visible[LEFT] {
            docks[LEFT] = Some(Rect::new(0.0, y0, lw, y1 - y0));
            edges[LEFT] = Some(Rect::new(lw, y0, SPLIT, y1 - y0));
        }
        if visible[RIGHT] {
            docks[RIGHT] = Some(Rect::new(w - rw, y0, rw, y1 - y0));
            edges[RIGHT] = Some(Rect::new(w - rw - SPLIT, y0, SPLIT, y1 - y0));
        }
        let x0 = lw + if visible[LEFT] { SPLIT } else { 0.0 };
        let x1 = w - rw - if visible[RIGHT] { SPLIT } else { 0.0 };
        let center = Rect::new(x0, y0, (x1 - x0).max(1.0), (y1 - y0).max(1.0));

        // Stack each dock's panels: all but the last keep their extent, the last
        // takes the remainder; splitters between.
        let mut panels = Vec::new();
        let mut splitters = Vec::new();
        for side in 0..4 {
            let Some(dock) = docks[side] else { continue };
            let ids = &occupied[side];
            if ids.is_empty() {
                continue;
            }
            let vertical = side == LEFT || side == RIGHT;
            let total = if vertical { dock.h } else { dock.w };
            let gaps = SPLIT * (ids.len() - 1) as f32;
            let extents = self.stack_extents(ids, total - gaps);
            let mut used = 0.0;
            for (nth, (&i, &ext)) in ids.iter().zip(&extents).enumerate() {
                let last = nth == ids.len() - 1;
                let r = if vertical {
                    Rect::new(dock.x, dock.y + used + SPLIT * nth as f32, dock.w, ext)
                } else {
                    Rect::new(dock.x + used + SPLIT * nth as f32, dock.y, ext, dock.h)
                };
                panels.push((i, r));
                if !last {
                    let s = if vertical {
                        Rect::new(dock.x, r.y + r.h, dock.w, SPLIT)
                    } else {
                        Rect::new(r.x + r.w, dock.y, SPLIT, dock.h)
                    };
                    splitters.push((side, nth, s));
                }
                used += ext;
            }
        }
        // Floating panels draw after (= above) docked ones, in vec order.
        for (i, p) in self.panels.iter().enumerate() {
            if let Place::Floating(x, y) = p.place {
                panels.push((i, Rect::new(x, y, p.w, p.h)));
            }
        }

        Layout {
            center,
            docks,
            panels,
            splitters,
            edges,
        }
    }

    /// Is the cursor over any workspace chrome (panel, splitter, edge, or the
    /// reserved bottom strip)? A host suppresses its content input when true.
    pub fn over_ui(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        if self.bottom > 0.0 && y >= h - self.bottom {
            return true;
        }
        let l = self.layout(w, h);
        l.panels.iter().any(|(_, r)| r.contains(Vec2::new(x, y)))
            || l.splitters
                .iter()
                .any(|(_, _, r)| r.contains(Vec2::new(x, y)))
            || l.edges
                .iter()
                .flatten()
                .any(|r| r.contains(Vec2::new(x, y)))
    }

    /// The topmost panel under the cursor (id + body rect) — e.g. wheel routing.
    pub fn body_at(&self, x: f32, y: f32, w: f32, h: f32) -> Option<(&str, Rect)> {
        let l = self.layout(w, h);
        l.panels
            .iter()
            .rev()
            .find(|(_, r)| r.contains(Vec2::new(x, y)))
            .map(|&(i, r)| (self.panels[i].id.as_str(), self.body_of(i, r)))
    }

    /// The mouse cursor for the pointer at `(x, y)` in a `w`×`h` workspace: an
    /// active chrome drag pins its cursor (grabbing hand for a titlebar drag,
    /// resize arrows for splitter/edge/grip — the pointer may outrun the
    /// strip mid-drag); idle, the resize affordances answer by hit-test, with
    /// panels occluding the strips below them. Splitters run across a dock's
    /// stacking axis: left/right docks stack vertically, so their splitters
    /// resize vertically (and the dock *edges* horizontally) — and vice versa.
    pub fn cursor_at(&self, x: f32, y: f32, w: f32, h: f32) -> CursorIcon {
        let edge_cursor = |side: usize| {
            if side == LEFT || side == RIGHT {
                CursorIcon::ResizeEW
            } else {
                CursorIcon::ResizeNS
            }
        };
        let splitter_cursor = |side: usize| {
            if side == LEFT || side == RIGHT {
                CursorIcon::ResizeNS
            } else {
                CursorIcon::ResizeEW
            }
        };
        match self.drag {
            Drag::Move { moved: true, .. } => return CursorIcon::Grabbing,
            Drag::DockEdge { side } => return edge_cursor(side),
            Drag::Splitter { side, .. } => return splitter_cursor(side),
            Drag::FloatResize { .. } => return CursorIcon::ResizeNWSE,
            _ => {}
        }
        let p = Vec2::new(x, y);
        let layout = self.layout(w, h);
        for &(i, r) in layout.panels.iter().rev() {
            let floating = matches!(self.panels[i].place, Place::Floating(..));
            let handle = Rect::new(r.x + r.w - HANDLE, r.y + r.h - HANDLE, HANDLE, HANDLE);
            if floating && handle.contains(p) {
                return CursorIcon::ResizeNWSE;
            }
            if r.contains(p) {
                return CursorIcon::Default;
            }
        }
        for &(side, _, r) in &layout.splitters {
            if r.contains(p) {
                return splitter_cursor(side);
            }
        }
        for side in 0..4 {
            if layout.edges[side].is_some_and(|r| r.contains(p)) {
                return edge_cursor(side);
            }
        }
        CursorIcon::Default
    }

    /// Pointer press. Chrome (titlebar/close/splitter/edge/grip) is handled
    /// internally; a body press is reported for the host to route.
    pub fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
        self.cursor = Some(Vec2::new(x, y));
        let layout = self.layout(w, h);

        // Topmost first: floating panels are at the tail of `layout.panels`.
        for &(i, r) in layout.panels.iter().rev() {
            let frame = self.frame_of(i);
            if close_rect(r, frame).contains(Vec2::new(x, y)) {
                self.panels[i].prev = self.panels[i].place;
                self.panels[i].place = Place::Hidden;
                return Press::Chrome;
            }
            if titlebar_rect(r, frame).contains(Vec2::new(x, y)) {
                // `raise` reorders the vec — re-resolve the index by id.
                let id = self.panels[i].id.clone();
                self.raise(i);
                self.drag = Drag::Move {
                    panel: self.find(&id).unwrap_or(i),
                    grab: (x - r.x, y - r.y),
                    start: (x, y),
                    moved: false,
                };
                return Press::Chrome;
            }
            let floating = matches!(self.panels[i].place, Place::Floating(..));
            let handle = Rect::new(r.x + r.w - HANDLE, r.y + r.h - HANDLE, HANDLE, HANDLE);
            if floating && handle.contains(Vec2::new(x, y)) {
                let id = self.panels[i].id.clone();
                self.raise(i);
                self.drag = Drag::FloatResize {
                    panel: self.find(&id).unwrap_or(i),
                };
                return Press::Chrome;
            }
            if r.contains(Vec2::new(x, y)) {
                let id = self.panels[i].id.clone();
                if floating {
                    self.raise(i);
                }
                return Press::Body {
                    id,
                    body: body_rect(r, frame),
                };
            }
        }
        for &(side, nth, r) in &layout.splitters {
            if r.contains(Vec2::new(x, y)) {
                self.drag = Drag::Splitter { side, nth };
                return Press::Chrome;
            }
        }
        for side in 0..4 {
            if layout.edges[side].is_some_and(|r| r.contains(Vec2::new(x, y))) {
                self.drag = Drag::DockEdge { side };
                return Press::Chrome;
            }
        }
        Press::None
    }

    /// `raise` moves a floating panel to the end of the vec (topmost) — indices
    /// in an active drag are resolved by id afterwards.
    fn raise(&mut self, i: usize) {
        if matches!(self.panels[i].place, Place::Floating(..)) && i + 1 != self.panels.len() {
            let p = self.panels.remove(i);
            self.panels.push(p);
        }
    }

    /// Pointer move. Returns true when the workspace wants a redraw.
    pub fn on_move(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        self.cursor = Some(Vec2::new(x, y));
        match self.drag {
            Drag::None => false,
            Drag::Move {
                panel,
                mut grab,
                start,
                moved,
            } => {
                let mut i = panel;
                if !moved {
                    if (x - start.0).abs() < DRAG_START && (y - start.1).abs() < DRAG_START {
                        return false;
                    }
                    // Undock: become floating at the cursor, keeping the grab
                    // point inside the (possibly narrower) floating titlebar —
                    // and take the top z-index immediately (`raise` reorders the
                    // vec, so re-resolve the dragged index by id).
                    grab.0 = grab.0.min(self.panels[i].w - TITLEBAR_H);
                    let at = float_clamp(self.top, self.panels[i].w, x - grab.0, y - grab.1, w, h);
                    self.panels[i].place = Place::Floating(at.0, at.1);
                    let id = self.panels[i].id.clone();
                    self.raise(i);
                    i = self.find(&id).unwrap_or(i);
                    self.drag = Drag::Move {
                        panel: i,
                        grab,
                        start,
                        moved: true,
                    };
                }
                let at = float_clamp(self.top, self.panels[i].w, x - grab.0, y - grab.1, w, h);
                self.panels[i].place = Place::Floating(at.0, at.1);
                true
            }
            Drag::DockEdge { side } => {
                let v = match side {
                    LEFT => x,
                    RIGHT => w - x,
                    TOP => y - self.top,
                    _ => h - y,
                };
                let lo = self.dock_cross_min(side);
                self.dock_size[side] = v.clamp(lo, MAX_DOCK.max(lo)).round();
                true
            }
            Drag::Splitter { side, nth } => {
                let ids = self.docked(side);
                let layout = self.layout(w, h);
                if let Some(&i) = ids.get(nth) {
                    let along = if side == LEFT || side == RIGHT { y } else { x };
                    let origin = layout
                        .panels
                        .iter()
                        .find(|(p, _)| *p == i)
                        .map(|(_, r)| {
                            if side == LEFT || side == RIGHT {
                                r.y
                            } else {
                                r.x
                            }
                        })
                        .unwrap_or(0.0);
                    self.panels[i].extent = (along - origin)
                        .max(along_min(self.panels[i].min, side))
                        .round();
                }
                true
            }
            Drag::FloatResize { panel } => {
                let i = panel;
                if let Place::Floating(px, py) = self.panels[i].place {
                    let (min, max) = (self.panels[i].min, self.panels[i].max);
                    self.panels[i].w = (x - px).clamp(min.0, max.0).round();
                    self.panels[i].h = (y - py).clamp(min.1, max.1).round();
                }
                true
            }
        }
    }

    /// Pointer release. Returns true when a drag was finished.
    pub fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        self.cursor = Some(Vec2::new(x, y));
        // Compute the layout while the drag is still live: a peeked-empty dock's
        // drop rect only exists during the drag, and clearing `self.drag` first
        // would make the drop miss it.
        let layout = self.layout(w, h);
        match std::mem::replace(&mut self.drag, Drag::None) {
            Drag::None => false,
            Drag::Move { panel, moved, .. } => {
                if moved {
                    self.drop_at(panel, x, y, &layout);
                }
                true
            }
            _ => true,
        }
    }

    /// Drop a dragged panel: into the dock under the cursor (insert position by
    /// midpoint along the dock axis), or stay floating. `layout` must be
    /// computed while the drag is live (peeks included).
    fn drop_at(&mut self, i: usize, x: f32, y: f32, layout: &Layout) {
        let Some(side) =
            (0..4).find(|&s| layout.docks[s].is_some_and(|r| r.contains(Vec2::new(x, y))))
        else {
            let p = &mut self.panels[i];
            p.prev = p.place;
            return;
        };
        self.panels[i].place = Place::Docked(side);
        self.panels[i].prev = Place::Docked(side);
        // Insert before the first dock-mate whose midpoint is past the cursor.
        let vertical = side == LEFT || side == RIGHT;
        let target = layout
            .panels
            .iter()
            .filter(|(p, _)| *p != i && self.panels[*p].place == Place::Docked(side))
            .find(|(_, r)| {
                if vertical {
                    y < r.y + r.h / 2.0
                } else {
                    x < r.x + r.w / 2.0
                }
            })
            .map(|(p, _)| *p);
        let moved = self.panels.remove(i);
        match target {
            Some(t) => {
                let t = if t > i { t - 1 } else { t };
                self.panels.insert(t, moved);
            }
            None => self.panels.push(moved),
        }
    }

    /// Is panel `i` floating (a host uses this to anchor per-panel effects, e.g.
    /// a material grain, that differ for floating vs docked)?
    pub fn is_floating(&self, i: usize) -> bool {
        matches!(self.panels[i].place, Place::Floating(..))
    }

    /// Whether a panel is being dragged (its titlebar may render "active").
    pub fn is_dragging(&self, i: usize) -> bool {
        matches!(self.drag, Drag::Move { panel, moved: true, .. } if panel == i)
    }

    /// Whether *any* workspace gesture holds the pointer — a titlebar move, a
    /// splitter, a dock edge or a float's resize grip.
    ///
    /// A host asks this before letting its own z-order override
    /// [`cursor_at`](Self::cursor_at): mid-gesture the pointer belongs to the
    /// workspace wherever it has wandered to, so the grab / resize cursor must
    /// survive the pointer leaving the chrome it started on.
    pub fn dragging(&self) -> bool {
        !matches!(self.drag, Drag::None)
    }

    /// Panel `i`'s border-ring width (also the content margin, so it drives both
    /// the chrome draw and the body/titlebar/close hit geometry).
    pub fn frame_of(&self, i: usize) -> f32 {
        if self.is_floating(i) {
            FRAME_FLOAT
        } else {
            FRAME_DOCK
        }
    }

    /// Panel `i`'s content area for rect `r` (inside its border-as-margin).
    pub fn body_of(&self, i: usize, r: Rect) -> Rect {
        body_rect(r, self.frame_of(i))
    }

    /// Panel `i`'s titlebar drag-handle rect for its arranged rect `r`.
    pub fn titlebar_of(&self, i: usize, r: Rect) -> Rect {
        titlebar_rect(r, self.frame_of(i))
    }

    /// Panel `i`'s close-glyph rect for its arranged rect `r`.
    pub fn close_of(&self, i: usize, r: Rect) -> Rect {
        close_rect(r, self.frame_of(i))
    }

    /// Whether the pointer is over panel `i`'s close `x`, given the rect it was
    /// drawn into — for a host that paints the panel frame itself and needs the
    /// affordance to light.
    ///
    /// The workspace answers because it already owns both halves: the same
    /// [`close_rect`] this asks about is what [`on_press`](Self::on_press) hides
    /// the panel from, so the hover and the click can never disagree. A host
    /// re-testing its own copy of the rect against its own copy of the pointer
    /// is the drift this exists to prevent.
    ///
    /// There is no matching *pressed* state on purpose: a close acts on the
    /// press, so the panel is gone before a held-down face could be drawn.
    pub fn close_hovered(&self, i: usize, r: Rect) -> bool {
        self.cursor
            .is_some_and(|p| close_rect(r, self.frame_of(i)).contains(p))
    }

    /// The pointer left — it went out of the window, or something press-modal
    /// (a menu cascade) covered the frame. Clears the chrome hover, exactly as
    /// [`Event::PointerLeft`] does for a hosted [`Ui`](crate::Ui): a host whose
    /// z-order changed *without* a pointer event has no other way to say so, and
    /// without it the affordance under the cursor stays lit behind the cascade.
    pub fn on_pointer_left(&mut self) {
        self.cursor = None;
    }

    /// Peeked-empty docks (drag drop-target highlights), for a host to paint.
    pub fn peeked_docks(&self, w: f32, h: f32) -> [Option<Rect>; 4] {
        let layout = self.layout(w, h);
        let peek = self.peek(w, h);
        let mut out = [None; 4];
        for side in 0..4 {
            if peek[side] && self.docked(side).is_empty() {
                out[side] = layout.docks[side];
            }
        }
        out
    }

    /// Clamp floating panels so at least [`MIN_VISIBLE`] px stays on-screen and
    /// the titlebar never hides above the top strip. Run on load + on resize;
    /// the move drag applies the same rule per pointer move, so a panel cannot
    /// be *dragged* anywhere a reload would refuse to put it.
    pub fn clamp_floating(&mut self, w: f32, h: f32) {
        let top = self.top;
        for p in &mut self.panels {
            if let Place::Floating(fx, fy) = &mut p.place {
                let (cx, cy) = float_clamp(top, p.w, *fx, *fy, w, h);
                (*fx, *fy) = (cx, cy);
            }
        }
    }

    /// The smallest a dock's cross-axis may be to fit its widest panel's content
    /// (never below [`MIN_DOCK`]).
    fn dock_cross_min(&self, side: usize) -> f32 {
        self.docked(side)
            .iter()
            .fold(MIN_DOCK, |m, &i| m.max(cross_min(self.panels[i].min, side)))
    }

    /// Clamp every panel into its size range and every dock into its cross-axis
    /// bounds. The companion to [`clamp_floating`](Self::clamp_floating).
    pub fn clamp_sizes(&mut self, w: f32, h: f32) {
        let avail_h = (h - self.top).max(MIN_VISIBLE);
        for p in &mut self.panels {
            match p.place {
                Place::Floating(..) => {
                    p.w = p.w.clamp(p.min.0, p.max.0.min(w).max(p.min.0));
                    p.h = p.h.clamp(p.min.1, p.max.1.min(avail_h).max(p.min.1));
                }
                Place::Docked(side) => p.extent = p.extent.max(along_min(p.min, side)),
                Place::Hidden => {}
            }
        }
        for side in 0..4 {
            let lo = self.dock_cross_min(side);
            self.dock_size[side] = self.dock_size[side].clamp(lo, MAX_DOCK.max(lo));
        }
    }

    /// A serializable snapshot of the current layout (place + sizes per panel,
    /// plus the dock sizes) for the host to persist.
    pub fn save_layout(&self) -> WorkspaceLayout {
        WorkspaceLayout {
            dock_size: self.dock_size,
            panels: self
                .panels
                .iter()
                .map(|p| PanelLayout {
                    id: p.id.clone(),
                    place: p.place,
                    w: p.w,
                    h: p.h,
                    extent: p.extent,
                })
                .collect(),
        }
    }

    /// Apply a saved [`WorkspaceLayout`]: set dock sizes + each *known* panel's
    /// place/size (unknown ids are skipped, so it is forward-compatible), then
    /// clamp into the `w`×`h` screen. Panels are updated in place — stacking
    /// order (vec order) is not reordered, matching a keyed persistence format.
    pub fn load_layout(&mut self, layout: &WorkspaceLayout, w: f32, h: f32) {
        self.dock_size = layout.dock_size;
        for pl in &layout.panels {
            let Some(idx) = self.find(&pl.id) else {
                continue;
            };
            let p = &mut self.panels[idx];
            p.w = pl.w;
            p.h = pl.h;
            p.extent = pl.extent;
            p.place = pl.place;
            // A hidden panel keeps its default `prev` so `show` restores it to a
            // sensible dock; otherwise track the loaded place.
            if !matches!(pl.place, Place::Hidden) {
                p.prev = pl.place;
            }
        }
        // Loaded sizes are untrusted — clamp size then position.
        self.clamp_sizes(w, h);
        self.clamp_floating(w, h);
    }

    /// Takes the last body press a [`Widget`]-hosted workspace recorded, so the
    /// host can route content interaction (the pure API returns it from
    /// [`on_press`](Self::on_press) directly).
    pub fn take_press(&mut self) -> Option<Press> {
        self.press.take()
    }
}

impl Default for Workspace {
    /// A generic demo layout: one panel left, two stacked right (with a
    /// splitter), one bottom, and a hidden extra — enough to exercise every
    /// docking behavior. Hosts usually build their own via [`panel`](Self::panel).
    fn default() -> Self {
        Self::new()
            .panel(
                PanelSpec::new("left", "Left")
                    .place(Place::Docked(LEFT))
                    .size(260.0, 220.0)
                    .extent(220.0)
                    .bounds((150.0, 150.0), (480.0, 480.0)),
            )
            .panel(
                PanelSpec::new("right-a", "Right A")
                    .place(Place::Docked(RIGHT))
                    .size(300.0, 320.0)
                    .extent(320.0)
                    .bounds((170.0, 140.0), (560.0, 900.0)),
            )
            .panel(
                PanelSpec::new("right-b", "Right B")
                    .place(Place::Docked(RIGHT))
                    .size(300.0, 220.0)
                    .extent(220.0)
                    .bounds((180.0, 170.0), (251.0, 640.0)),
            )
            .panel(
                PanelSpec::new("bottom", "Bottom")
                    .place(Place::Docked(BOTTOM))
                    .size(360.0, 160.0)
                    .extent(360.0)
                    .bounds((300.0, 140.0), (1200.0, 4096.0)),
            )
            .panel(
                PanelSpec::new("extra", "Extra")
                    .place(Place::Hidden)
                    .prev(Place::Docked(RIGHT))
                    .size(300.0, 320.0)
                    .extent(320.0)
                    .bounds((170.0, 140.0), (560.0, 900.0)),
            )
    }
}

impl Widget for Workspace {
    fn measure(&mut self, avail: Size, _ctx: &mut LayoutCtx) -> Size {
        avail
    }

    fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    /// [`cursor_at`](Self::cursor_at) — same origin-arranged assumption as
    /// [`draw`](Widget::draw).
    fn cursor(&self, pos: Vec2) -> CursorIcon {
        self.cursor_at(pos.x, pos.y, self.rect.w, self.rect.h)
    }

    /// Generic themed chrome (base pass): panel frames + titlebars + titles +
    /// close glyphs, splitter/edge strips, peek highlights, and a floating-panel
    /// resize grip. A host wanting a richer look draws its own from
    /// [`layout`](Self::layout). **Assumes the workspace is arranged from the
    /// surface origin** (the common `Ui` root case); the pure geometry API has
    /// no such restriction.
    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if !ctx.is_base() {
            return;
        }
        let (w, h) = (self.rect.w, self.rect.h);
        let layout = self.layout(w, h);
        // Peek drop-targets under everything.
        for r in self.peeked_docks(w, h).into_iter().flatten() {
            dl.fill_rect(r, ctx.theme.accent().with_alpha(60));
        }
        // Splitter + edge strips.
        for r in layout.edges.iter().flatten() {
            ctx.theme.frame(dl, *r, ctx.theme.ink_dim(), Bevel::Raised);
        }
        for (_, _, r) in &layout.splitters {
            ctx.theme.frame(dl, *r, ctx.theme.ink_dim(), Bevel::Raised);
        }
        // Panels in draw order (docked then floating).
        let px = ctx.theme.font_px(TextRole::Title);
        for &(i, r) in &layout.panels {
            let frame = self.frame_of(i);
            ctx.theme.panel(dl, r);
            let bar = titlebar_band(r, frame);
            ctx.theme.titlebar(dl, bar);
            let title = &self.panels[i].title;
            let base = Vec2::new(bar.x + TITLE_PAD, bar.center().y + px * 0.34);
            ctx.theme.text(dl, ctx.fonts, base, title, TextRole::Title);
            // Close glyph — ASCII `x`, so it renders in a font that carries
            // only the ASCII range (the `Theme::ellipsized` rule).
            let close = close_rect(r, frame);
            let cbase = Vec2::new(close.x + 6.0, close.center().y + px * 0.34);
            ctx.theme.text(dl, ctx.fonts, cbase, "x", TextRole::Title);
            if self.is_floating(i) {
                // A stepped resize grip in the bottom-right corner.
                let (x1, y1) = (r.x + r.w, r.y + r.h);
                for k in 0..HANDLE as i32 {
                    let row_w = HANDLE - k as f32;
                    dl.fill_rect(
                        Rect::new(x1 - row_w, y1 - 1.0 - k as f32, row_w, 1.0),
                        ctx.theme.ink_dim(),
                    );
                }
            }
        }
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        let (w, h) = (self.rect.w, self.rect.h);
        match ev {
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                ..
            } if ctx.is_target(self.id) => {
                let p = ctx.pointer;
                match self.on_press(p.x, p.y, w, h) {
                    Press::None => false,
                    Press::Chrome => {
                        ctx.capture(self.id);
                        ctx.consume_pointer();
                        true
                    }
                    body @ Press::Body { .. } => {
                        // Record for the host to route into panel content.
                        self.press = Some(body);
                        ctx.consume_pointer();
                        true
                    }
                }
            }
            Event::PointerMoved { .. } if ctx.is_target(self.id) => {
                if self.on_move(ctx.pointer.x, ctx.pointer.y, w, h) {
                    ctx.consume_pointer();
                    true
                } else {
                    false
                }
            }
            // Unconsumed: the leave is everyone's, and it only clears the chrome
            // hover.
            Event::PointerLeft => {
                self.on_pointer_left();
                false
            }
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: false,
                ..
            } => {
                let finished = self.on_release(ctx.pointer.x, ctx.pointer.y, w, h);
                if finished {
                    ctx.consume_pointer();
                }
                finished
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

    fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
        if self.over_ui(pos.x, pos.y, self.rect.w, self.rect.h) {
            Some(self.id)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f32 = 1280.0;
    const H: f32 = 800.0;

    fn ws() -> Workspace {
        Workspace::default()
    }

    /// The arranged rect of panel `id` at the test window size.
    fn panel_rect(w: &Workspace, id: &str) -> Rect {
        let i = w.find(id).expect("a known panel");
        w.layout(W, H)
            .panels
            .iter()
            .find(|(p, _)| *p == i)
            .map(|(_, r)| *r)
            .expect("a shown panel is in the layout")
    }

    /// **Every docked panel lands inside its dock.** Showing a third panel into
    /// a right dock two panels already fill used to place it at the end of their
    /// extents — past the dock, off the screen — while the model happily called
    /// it visible, dispatched pointer events to it and persisted its place. The
    /// stack now shrinks to fit: each panel keeps at least `MIN_PANEL`, the sum
    /// is the dock, and nothing is left outside it.
    #[test]
    fn a_dock_shrinks_its_stack_instead_of_pushing_a_panel_off_the_end() {
        let mut w = ws();
        let dock = |w: &Workspace| w.layout(W, H).docks[RIGHT].expect("the right dock");
        let rects = |w: &Workspace| {
            let l = w.layout(W, H);
            let side: Vec<Rect> = l
                .panels
                .iter()
                .filter(|(i, _)| w.panels[*i].place == Place::Docked(RIGHT))
                .map(|&(_, r)| r)
                .collect();
            side
        };

        // Two panels: they already share the dock exactly (the last takes the
        // remainder), so nothing shrinks.
        let d = dock(&w);
        let two = rects(&w);
        assert_eq!(two.len(), 2);
        assert!(
            (two[1].bottom() - d.bottom()).abs() < 0.5,
            "the last panel ends at the dock: {two:?}"
        );

        // A third, whose extent the dock cannot possibly honour.
        assert!(w.show("extra", Some(true)).expect("extra is a known panel"));
        let three = rects(&w);
        assert_eq!(three.len(), 3, "all three are laid out");
        for (n, r) in three.iter().enumerate() {
            assert!(r.y >= d.y - 0.5, "panel {n} starts inside the dock: {r:?}");
            assert!(
                r.bottom() <= d.bottom() + 0.5,
                "panel {n} ends inside the dock: {r:?}"
            );
            assert!(
                r.h >= MIN_PANEL - 0.5,
                "panel {n} keeps a usable height: {r:?}"
            );
        }
        assert!(
            (three[2].bottom() - d.bottom()).abs() < 0.5,
            "and the stack still fills it"
        );
        // The point of the whole thing: the panel just opened can be pointed at.
        let mid = three[2].center();
        assert_eq!(
            w.body_at(mid.x, mid.y, W, H).map(|(id, _)| id),
            Some("extra")
        );

        // A dock too small even for the floor splits evenly rather than
        // stacking three panels down the screen.
        let tiny = w.layout(W, 3.0 * MIN_PANEL);
        let cramped: Vec<Rect> = tiny
            .panels
            .iter()
            .filter(|(i, _)| w.panels[*i].place == Place::Docked(RIGHT))
            .map(|&(_, r)| r)
            .collect();
        let d = tiny.docks[RIGHT].expect("the right dock");
        for r in &cramped {
            assert!(
                r.bottom() <= d.bottom() + 0.5,
                "still inside a cramped dock: {r:?}"
            );
        }
    }

    /// `cursor_at`: dock edges resize across their axis, splitters along the
    /// dock's stacking axis, a floating grip diagonally; panel bodies keep the
    /// arrow; a titlebar drag pins the grabbing hand once it passes the drag
    /// threshold.
    #[test]
    fn cursor_at_reports_resize_and_grab_affordances() {
        let mut w = ws();
        let layout = w.layout(W, H);

        let e = layout.edges[LEFT].expect("left dock has an edge resizer");
        let c = e.center();
        assert_eq!(w.cursor_at(c.x, c.y, W, H), CursorIcon::ResizeEW, "edge");

        let (_, _, sr) = layout
            .splitters
            .iter()
            .copied()
            .find(|&(side, ..)| side == RIGHT)
            .expect("right dock stacks two panels");
        let c = sr.center();
        assert_eq!(
            w.cursor_at(c.x, c.y, W, H),
            CursorIcon::ResizeNS,
            "vertical-stack splitter"
        );

        let (_, pr) = layout.panels[0];
        let c = pr.center();
        assert_eq!(w.cursor_at(c.x, c.y, W, H), CursorIcon::Default, "body");

        // Float a panel: its bottom-right grip shows the diagonal arrows.
        w.dock_to("left", Place::Floating(300.0, 200.0)).unwrap();
        let l2 = w.layout(W, H);
        let &(_, r) = l2.panels.last().expect("floating panel draws last");
        assert_eq!(
            w.cursor_at(r.right() - 2.0, r.bottom() - 2.0, W, H),
            CursorIcon::ResizeNWSE,
            "floating grip"
        );

        // Titlebar drag: plain arrow until the drag threshold, then grabbing.
        let (tx, ty) = (r.x + 40.0, r.y + 5.0);
        assert_eq!(w.on_press(tx, ty, W, H), Press::Chrome);
        assert_eq!(
            w.cursor_at(tx, ty, W, H),
            CursorIcon::Default,
            "pressed, not yet dragging"
        );
        w.on_move(tx + 60.0, ty + 40.0, W, H);
        assert_eq!(
            w.cursor_at(tx + 60.0, ty + 40.0, W, H),
            CursorIcon::Grabbing,
            "live titlebar drag"
        );
        w.on_release(tx + 60.0, ty + 40.0, W, H);
        assert_eq!(w.cursor_at(0.0, 0.0, W, H), CursorIcon::Default);
    }

    #[test]
    fn save_load_round_trips_places_and_dock_sizes() {
        let mut a = ws();
        a.dock_to("right-b", Place::Docked(LEFT)).unwrap();
        a.dock_to("left", Place::Floating(120.0, 90.0)).unwrap();
        a.show("bottom", Some(false)).unwrap();
        a.set_dock_size([333.0, 280.0, 130.0, 150.0]);
        let saved = a.save_layout();
        let mut b = ws();
        b.load_layout(&saved, W, H);
        for id in ["right-b", "left", "bottom", "right-a"] {
            let (pa, pb) = (a.find(id).unwrap(), b.find(id).unwrap());
            assert_eq!(
                a.panels[pa].place, b.panels[pb].place,
                "{id} place round-trips"
            );
        }
        assert_eq!(a.dock_size(), b.dock_size(), "dock sizes round-trip");
    }

    #[test]
    fn clamp_keeps_floats_on_screen() {
        let mut w = ws();
        w.top = 24.0;
        w.dock_to("left", Place::Floating(5000.0, -500.0)).unwrap();
        w.clamp_floating(W, H);
        let p = &w.panels[w.find("left").unwrap()];
        let Place::Floating(x, y) = p.place else {
            panic!("still floating")
        };
        assert!(
            x <= W - MIN_VISIBLE && x + p.w >= MIN_VISIBLE,
            "≥32px visible horizontally"
        );
        assert!(
            y >= w.top && y <= H - MIN_VISIBLE,
            "titlebar below the top strip"
        );
    }

    /// **A titlebar drag obeys the same bounds as a load.** Dragging up past the
    /// reserved `top` strip used to slide the titlebar under the host's menu bar
    /// and tab strip, where it is neither readable nor grabbable — the panel
    /// could not be dragged back out, and only a resize (which re-runs
    /// `clamp_floating`) recovered it.
    ///
    /// The floor is hard on the top edge only: sideways and downwards a panel
    /// may still be parked mostly off-screen, which is deliberate.
    #[test]
    fn a_move_drag_cannot_push_a_panel_above_the_top_strip() {
        let mut w = ws();
        w.top = 24.0;
        w.dock_to("left", Place::Floating(300.0, 200.0)).unwrap();
        let r = panel_rect(&w, "left");
        let (tx, ty) = (r.x + 40.0, r.y + 5.0);
        assert_eq!(w.on_press(tx, ty, W, H), Press::Chrome);

        let float_y = |w: &Workspace| match w.panels[w.find("left").unwrap()].place {
            Place::Floating(_, y) => y,
            other => panic!("still floating, got {other:?}"),
        };
        let float_x = |w: &Workspace| match w.panels[w.find("left").unwrap()].place {
            Place::Floating(x, _) => x,
            other => panic!("still floating, got {other:?}"),
        };

        // Drag far above the window: the top strip is the floor, mid-drag.
        w.on_move(tx, ty - 4000.0, W, H);
        assert_eq!(
            float_y(&w),
            w.top,
            "clamped to the reserved strip while dragging"
        );
        // And it stays clamped at the drop, not just while moving.
        w.on_release(tx, ty - 4000.0, W, H);
        assert!(float_y(&w) >= w.top, "and the release does not undo it");

        // Sideways and downwards the panel may still hang off the window - only
        // MIN_VISIBLE has to stay inside, which is what parks one out of the way.
        assert_eq!(w.on_press(tx, float_y(&w) + 5.0, W, H), Press::Chrome);
        w.on_move(tx + 5000.0, float_y(&w) + 5.0, W, H);
        assert!(float_x(&w) <= W - MIN_VISIBLE, "kept partly on-screen");
        assert!(
            float_x(&w) > W / 2.0,
            "but genuinely dragged off to the right"
        );
    }

    /// `dragging` is the "the pointer is mine" flag a host needs to let a
    /// workspace gesture outrank its own z-order — see `cursor_at`, which must
    /// keep reporting the grabbing hand after the drag leaves the chrome.
    #[test]
    fn dragging_reports_any_live_gesture() {
        let mut w = ws();
        assert!(!w.dragging(), "idle");
        let r = panel_rect(&w, "left");
        w.on_press(r.x + 40.0, r.y + 5.0, W, H);
        assert!(w.dragging(), "a titlebar press already holds the pointer");
        w.on_release(r.x + 40.0, r.y + 5.0, W, H);
        assert!(!w.dragging(), "released");

        let e = w.layout(W, H).edges[LEFT].expect("left dock has an edge resizer");
        w.on_press(e.center().x, e.center().y, W, H);
        assert!(w.dragging(), "a dock-edge drag counts too");
        w.on_release(e.center().x, e.center().y, W, H);
        assert!(!w.dragging());
    }

    #[test]
    fn clamp_sizes_bounds_floating_and_docks() {
        let mut w = ws();
        let i = w.find("left").unwrap();
        let (min, max) = (w.panels[i].min, w.panels[i].max);
        w.panels[i].place = Place::Floating(10.0, 50.0);
        w.panels[i].w = 5.0;
        w.panels[i].h = 5.0;
        w.clamp_sizes(W, H);
        assert_eq!((w.panels[i].w, w.panels[i].h), min, "min enforced");
        w.panels[i].w = 5000.0;
        w.panels[i].h = 5000.0;
        w.clamp_sizes(W, H);
        assert_eq!((w.panels[i].w, w.panels[i].h), max, "max enforced");
        w.panels[i].max = (9999.0, 9999.0);
        w.panels[i].w = 9999.0;
        w.clamp_sizes(W, H);
        assert_eq!(w.panels[i].w, W, "floating width capped at the viewport");
        // A dock too thin for its widest panel's content is widened.
        let mut d = ws();
        d.set_dock_size([10.0, 280.0, 130.0, 150.0]);
        d.clamp_sizes(W, H);
        let mm = d.panels[d.find("left").unwrap()].min;
        assert!(
            d.dock_size()[LEFT] >= cross_min(mm, LEFT),
            "dock fits content min"
        );
    }

    #[test]
    fn default_layout_partitions_the_screen() {
        let l = ws().layout(W, H);
        // Four visible panels (the fifth is hidden); center inset on 3 sides
        // (no top dock by default → center touches the screen top).
        assert_eq!(l.panels.len(), 4);
        assert!(l.center.x > 0.0);
        assert!(l.center.x + l.center.w < W);
        assert!(l.center.y + l.center.h < H);
        assert!(l.docks[TOP].is_none());
        assert_eq!(l.center.y, 0.0);
        assert_eq!(l.center.y, l.docks[LEFT].unwrap().y);
        // Right dock stacks two panels with one splitter between them.
        assert_eq!(l.splitters.len(), 1);
        assert_eq!(l.splitters[0].0, RIGHT);
    }

    #[test]
    fn hiding_a_dock_panel_grows_the_center() {
        let mut w = ws();
        let before = w.layout(W, H).center;
        w.show("left", Some(false)).unwrap();
        let after = w.layout(W, H).center;
        assert!(after.x < before.x, "left dock auto-hides when emptied");
        assert!(after.w > before.w);
        w.show("left", Some(true)).unwrap();
        assert_eq!(w.panels[w.find("left").unwrap()].place, Place::Docked(LEFT));
    }

    #[test]
    fn window_toggle_round_trips() {
        let mut w = ws();
        w.show("bottom", None).unwrap();
        assert_eq!(w.panels[w.find("bottom").unwrap()].place, Place::Hidden);
        w.show("bottom", None).unwrap();
        assert_eq!(
            w.panels[w.find("bottom").unwrap()].place,
            Place::Docked(BOTTOM)
        );
        assert!(w.show("nonsense", None).is_err());
    }

    #[test]
    fn dock_command_moves_between_sides_and_float() {
        let mut w = ws();
        w.dock_to("right-b", Place::Docked(LEFT)).unwrap();
        assert_eq!(w.docked(LEFT).len(), 2);
        assert_eq!(w.docked(RIGHT).len(), 1);
        w.dock_to("right-b", Place::Floating(50.0, 60.0)).unwrap();
        let i = w.find("right-b").unwrap();
        assert_eq!(w.panels[i].place, Place::Floating(50.0, 60.0));
        assert!(w.dock_to("no-such-panel", Place::Docked(LEFT)).is_err());
    }

    #[test]
    fn titlebar_drag_undocks_then_drops_into_a_dock() {
        let mut w = ws();
        let l = w.layout(W, H);
        let mm = w.find("left").unwrap();
        let r = l.panels.iter().find(|(i, _)| *i == mm).unwrap().1;
        let (px, py) = (r.x + 40.0, r.y + 8.0);
        assert_eq!(Press::Chrome, w.on_press(px, py, W, H));
        // A 2-px wiggle does not undock.
        w.on_move(px + 2.0, py, W, H);
        assert_eq!(w.panels[w.find("left").unwrap()].place, Place::Docked(LEFT));
        // Crossing the threshold floats it at the cursor.
        w.on_move(640.0, 400.0, W, H);
        assert!(matches!(
            w.panels[w.find("left").unwrap()].place,
            Place::Floating(..)
        ));
        // Releasing over the right dock docks it there.
        let right = w.layout(W, H).docks[RIGHT].unwrap();
        w.on_move(right.x + right.w / 2.0, right.y + right.h - 10.0, W, H);
        assert!(w.on_release(right.x + right.w / 2.0, right.y + right.h - 10.0, W, H));
        assert_eq!(
            w.panels[w.find("left").unwrap()].place,
            Place::Docked(RIGHT)
        );
        assert_eq!(w.docked(RIGHT).len(), 3);
        assert_eq!(*w.docked(RIGHT).last().unwrap(), w.find("left").unwrap());
        assert!(w.layout(W, H).docks[LEFT].is_none());
    }

    #[test]
    fn undocking_takes_top_z_immediately() {
        let mut w = ws();
        w.dock_to("right-a", Place::Floating(400.0, 200.0)).unwrap();
        let l = w.layout(W, H);
        let pi = w.find("right-b").unwrap();
        let r = l.panels.iter().find(|(i, _)| *i == pi).unwrap().1;
        assert_eq!(Press::Chrome, w.on_press(r.x + 30.0, r.y + 8.0, W, H));
        w.on_move(640.0, 300.0, W, H);
        assert_eq!(
            w.panels.last().unwrap().id,
            "right-b",
            "undocked panel is topmost"
        );
        w.on_move(700.0, 350.0, W, H);
        let p = w.panels.last().unwrap();
        assert_eq!(p.id, "right-b");
        assert!(matches!(p.place, Place::Floating(px, _) if px > 600.0));
    }

    #[test]
    fn re_docks_into_an_emptied_peeked_dock() {
        let mut w = ws();
        let l = w.layout(W, H);
        let mm = w.find("left").unwrap();
        let r = l.panels.iter().find(|(i, _)| *i == mm).unwrap().1;
        assert_eq!(Press::Chrome, w.on_press(r.x + 40.0, r.y + 8.0, W, H));
        w.on_move(600.0, 400.0, W, H);
        assert!(
            w.layout(W, H).docks[LEFT].is_none(),
            "emptied dock auto-hides"
        );
        w.on_move(30.0, 400.0, W, H);
        assert!(
            w.layout(W, H).docks[LEFT].is_some(),
            "peeked open during the drag"
        );
        assert!(w.on_release(30.0, 400.0, W, H));
        assert_eq!(w.panels[w.find("left").unwrap()].place, Place::Docked(LEFT));
    }

    #[test]
    fn drop_outside_any_dock_stays_floating() {
        let mut w = ws();
        let l = w.layout(W, H);
        let mm = w.find("left").unwrap();
        let r = l.panels.iter().find(|(i, _)| *i == mm).unwrap().1;
        assert_eq!(Press::Chrome, w.on_press(r.x + 40.0, r.y + 8.0, W, H));
        w.on_move(600.0, 300.0, W, H);
        w.on_release(600.0, 300.0, W, H);
        let p = &w.panels[w.find("left").unwrap()];
        assert!(matches!(p.place, Place::Floating(..)));
        assert_eq!(p.prev, p.place, "floating place survives a later hide/show");
    }

    #[test]
    fn close_glyph_hides_and_show_restores() {
        let mut w = ws();
        let l = w.layout(W, H);
        let tb = w.find("bottom").unwrap();
        let r = l.panels.iter().find(|(i, _)| *i == tb).unwrap().1;
        let close = close_rect(r, FRAME_DOCK);
        assert_eq!(
            Press::Chrome,
            w.on_press(close.x + 4.0, close.y + 4.0, W, H)
        );
        assert_eq!(w.panels[w.find("bottom").unwrap()].place, Place::Hidden);
        assert!(w.layout(W, H).docks[BOTTOM].is_none());
        w.show("bottom", Some(true)).unwrap();
        assert_eq!(
            w.panels[w.find("bottom").unwrap()].place,
            Place::Docked(BOTTOM)
        );
    }

    /// The close `x` lights from the pointer the workspace already tracks — the
    /// same rect the press hides the panel from, so hover and click cannot
    /// disagree — and goes dark when the pointer leaves, whether it left the
    /// window or a cascade covered the frame. A host painting the frame itself
    /// has no other way to know either.
    #[test]
    fn close_hover_follows_the_pointer_and_the_leave() {
        let mut w = ws();
        let l = w.layout(W, H);
        let tb = w.find("bottom").unwrap();
        let r = l.panels.iter().find(|(i, _)| *i == tb).unwrap().1;
        let close = w.close_of(tb, r);

        assert!(!w.close_hovered(tb, r), "no pointer yet, no hover");
        w.on_move(close.x + 4.0, close.y + 4.0, W, H);
        assert!(w.close_hovered(tb, r), "the pointer is on the x");
        let other = l.panels.iter().find(|(i, _)| *i != tb).unwrap().1;
        assert!(
            !w.close_hovered(w.find("left").unwrap(), other),
            "and only the panel whose own rect it is"
        );
        w.on_move(r.center().x, r.center().y, W, H);
        assert!(!w.close_hovered(tb, r), "the body is not the x");

        w.on_move(close.x + 4.0, close.y + 4.0, W, H);
        w.on_pointer_left();
        assert!(
            !w.close_hovered(tb, r),
            "the leave clears it where no move would"
        );
    }

    #[test]
    fn dock_edge_drag_resizes_and_clamps() {
        let mut w = ws();
        let l = w.layout(W, H);
        let edge = l.edges[LEFT].unwrap();
        assert_eq!(Press::Chrome, w.on_press(edge.x + 2.0, edge.y + 50.0, W, H));
        w.on_move(400.0, edge.y + 50.0, W, H);
        w.on_release(400.0, edge.y + 50.0, W, H);
        assert_eq!(w.layout(W, H).docks[LEFT].unwrap().w, 400.0);
        let edge = w.layout(W, H).edges[LEFT].unwrap();
        assert_eq!(Press::Chrome, w.on_press(edge.x + 2.0, edge.y + 50.0, W, H));
        w.on_move(5000.0, edge.y + 50.0, W, H);
        w.on_release(5000.0, edge.y + 50.0, W, H);
        assert_eq!(w.layout(W, H).docks[LEFT].unwrap().w, MAX_DOCK);
    }

    #[test]
    fn splitter_drag_resizes_the_dock_mate_above() {
        let mut w = ws();
        let l = w.layout(W, H);
        let (side, nth, s) = l.splitters[0];
        assert_eq!((side, nth), (RIGHT, 0));
        assert_eq!(Press::Chrome, w.on_press(s.x + 2.0, s.y + 2.0, W, H));
        let dock_top = l.docks[RIGHT].unwrap().y;
        w.on_move(s.x + 2.0, dock_top + 150.0, W, H);
        w.on_release(s.x + 2.0, dock_top + 150.0, W, H);
        let i = w.docked(RIGHT)[0];
        assert_eq!(w.panels[i].extent, 150.0);
    }

    #[test]
    fn body_press_reports_panel_and_rect() {
        let mut w = ws();
        let l = w.layout(W, H);
        let ti = w.find("right-a").unwrap();
        let r = l.panels.iter().find(|(i, _)| *i == ti).unwrap().1;
        match w.on_press(r.x + 50.0, r.y + 100.0, W, H) {
            Press::Body { id, body } => {
                assert_eq!(id, "right-a");
                assert_eq!(body, body_rect(r, FRAME_DOCK));
            }
            other => panic!("expected Body, got {other:?}"),
        }
        assert_eq!(
            w.body_at(r.x + 50.0, r.y + 100.0, W, H).unwrap().0,
            "right-a"
        );
        let c = w.layout(W, H).center;
        assert!(w.body_at(c.x + c.w / 2.0, c.y + c.h / 2.0, W, H).is_none());
    }

    #[test]
    fn over_ui_separates_chrome_from_center() {
        let w = ws();
        let l = w.layout(W, H);
        let c = l.center;
        assert!(!w.over_ui(c.x + c.w / 2.0, c.y + c.h / 2.0, W, H));
        let r = l.panels[0].1;
        assert!(w.over_ui(r.x + 5.0, r.y + 5.0, W, H));
    }

    #[test]
    fn floating_resize_respects_minimums() {
        let mut w = ws();
        w.dock_to("left", Place::Floating(100.0, 100.0)).unwrap();
        let i = w.find("left").unwrap();
        let (pw, ph) = (w.panels[i].w, w.panels[i].h);
        let handle = (100.0 + pw - 4.0, 100.0 + ph - 4.0);
        assert_eq!(Press::Chrome, w.on_press(handle.0, handle.1, W, H));
        w.on_move(100.0 + 10.0, 100.0 + 10.0, W, H);
        w.on_release(100.0 + 10.0, 100.0 + 10.0, W, H);
        let i = w.find("left").unwrap();
        assert_eq!((w.panels[i].w, w.panels[i].h), w.panels[i].min);
    }

    #[test]
    fn bottom_strip_reserved_and_center_shrinks() {
        let mut w = ws();
        let full = w.layout(W, H).center;
        w.bottom = 22.0;
        let reserved = w.layout(W, H).center;
        assert!(
            reserved.y + reserved.h <= H - w.bottom + 0.001,
            "center clears the bottom strip"
        );
        assert!(reserved.h < full.h);
        // A click in the reserved strip counts as chrome (never reaches center).
        assert!(w.over_ui(W * 0.5, H - 5.0, W, H));
    }

    /// `PanelSpec::hint` seeds the body placeholder; `is_visible` reports
    /// non-hidden panels and is false for hidden or unknown ids.
    #[test]
    fn hint_and_visibility_queries() {
        let mut w = Workspace::new().panel(
            PanelSpec::new("a", "A")
                .hint("coming soon")
                .place(Place::Docked(LEFT)),
        );
        assert_eq!(w.panels[0].hint, "coming soon", "hint carries over");
        assert!(w.is_visible("a"));
        w.show("a", Some(false)).unwrap();
        assert!(!w.is_visible("a"), "hidden panels are not visible");
        assert!(!w.is_visible("ghost"), "unknown ids are never visible");
    }

    /// A top-docked panel opens the top dock below the reserved `top` strip,
    /// full-width, with its edge resizer hanging underneath.
    #[test]
    fn top_dock_sits_below_the_reserved_strip() {
        let mut w = ws();
        w.top = 20.0;
        w.dock_to("bottom", Place::Docked(TOP)).unwrap();
        let l = w.layout(W, H);
        let dock = l.docks[TOP].expect("top dock is visible");
        assert_eq!(dock.y, 20.0, "dock starts below the reserved strip");
        assert_eq!(dock.w, W, "top dock spans the full width");
        let edge = l.edges[TOP].expect("top dock has an edge resizer");
        assert_eq!(edge.y, dock.y + dock.h, "resizer hangs below the dock");
        assert_eq!(l.center.y, edge.y + edge.h, "center starts under it");
    }

    /// Panels stacked on a horizontal dock (bottom) split along x: the
    /// splitter strip is vertical, resizes EW, and the dock edge resizes NS.
    #[test]
    fn bottom_dock_stacks_horizontally_with_vertical_splitters() {
        let mut w = ws();
        w.dock_to("left", Place::Docked(BOTTOM)).unwrap();
        let l = w.layout(W, H);
        let (_, nth, s) = l
            .splitters
            .iter()
            .copied()
            .find(|&(side, ..)| side == BOTTOM)
            .expect("bottom dock stacks two panels");
        assert_eq!(nth, 0, "the splitter resizes the first bottom panel");
        let first = l
            .panels
            .iter()
            .find(|(i, _)| w.panels[*i].place == Place::Docked(BOTTOM))
            .expect("a bottom-docked panel")
            .1;
        assert_eq!(s.x, first.x + first.w, "splitter abuts the first panel");
        assert_eq!(
            s.h,
            l.docks[BOTTOM].unwrap().h,
            "splitter spans the dock height"
        );
        let c = s.center();
        assert_eq!(
            w.cursor_at(c.x, c.y, W, H),
            CursorIcon::ResizeEW,
            "horizontal-stack splitter"
        );
        let e = l.edges[BOTTOM].unwrap().center();
        assert_eq!(
            w.cursor_at(e.x, e.y, W, H),
            CursorIcon::ResizeNS,
            "bottom edge resizer"
        );
    }

    /// An active chrome drag pins its cursor even when the pointer outruns
    /// the strip: dock edge → edge arrows, splitter → stack arrows, floating
    /// grip → the diagonal.
    #[test]
    fn active_drags_pin_their_resize_cursors() {
        let mut w = ws();
        let e = w.layout(W, H).edges[LEFT].unwrap().center();
        assert_eq!(w.on_press(e.x, e.y, W, H), Press::Chrome);
        assert_eq!(
            w.cursor_at(700.0, 400.0, W, H),
            CursorIcon::ResizeEW,
            "edge drag keeps its arrows away from the strip"
        );
        w.on_release(700.0, 400.0, W, H);

        let (_, _, s) = w.layout(W, H).splitters[0]; // right dock stacks two
        let c = s.center();
        assert_eq!(w.on_press(c.x, c.y, W, H), Press::Chrome);
        assert_eq!(
            w.cursor_at(50.0, 50.0, W, H),
            CursorIcon::ResizeNS,
            "splitter drag pins the stacking-axis arrows"
        );
        w.on_release(50.0, 50.0, W, H);

        w.dock_to("left", Place::Floating(300.0, 200.0)).unwrap();
        let i = w.find("left").unwrap();
        let grip = (300.0 + w.panels[i].w - 3.0, 200.0 + w.panels[i].h - 3.0);
        assert_eq!(w.on_press(grip.0, grip.1, W, H), Press::Chrome);
        assert_eq!(
            w.cursor_at(0.0, 0.0, W, H),
            CursorIcon::ResizeNWSE,
            "grip drag pins the diagonal"
        );
        w.on_release(0.0, 0.0, W, H);
    }

    /// A body press on a floating panel raises it to the top of the float
    /// stack; presses over the bare center report `Press::None`; and with no
    /// drag in progress, moves and releases claim nothing.
    #[test]
    fn body_press_raises_floats_and_empty_space_is_none() {
        let mut w = ws();
        w.dock_to("left", Place::Floating(400.0, 300.0)).unwrap();
        w.dock_to("bottom", Place::Floating(700.0, 300.0)).unwrap();
        assert_ne!(
            w.panels.last().unwrap().id,
            "left",
            "precondition: left floats below bottom"
        );
        assert!(matches!(
            w.on_press(450.0, 400.0, W, H),
            Press::Body { ref id, .. } if id == "left"
        ));
        assert_eq!(
            w.panels.last().unwrap().id,
            "left",
            "a body press raises the floating panel"
        );
        assert_eq!(
            w.on_press(200.0, 100.0, W, H),
            Press::None,
            "the bare center hits nothing"
        );
        assert!(
            !w.on_move(210.0, 110.0, W, H),
            "no drag: move claims nothing"
        );
        assert!(
            !w.on_release(210.0, 110.0, W, H),
            "no drag: release claims nothing"
        );
    }

    /// Dock-edge drags size each side from its own screen edge: right from
    /// the right border, top below the reserved strip, bottom from the screen
    /// bottom — clamped up to the dock's content minimum.
    #[test]
    fn edge_drags_measure_from_their_screen_edge() {
        let mut w = ws();
        let e = w.layout(W, H).edges[RIGHT].unwrap().center();
        assert_eq!(w.on_press(e.x, e.y, W, H), Press::Chrome);
        w.on_move(W - 350.0, e.y, W, H);
        w.on_release(W - 350.0, e.y, W, H);
        assert_eq!(w.dock_size()[RIGHT], 350.0, "right dock: w - x");

        let mut w = ws();
        w.top = 20.0;
        w.dock_to("bottom", Place::Docked(TOP)).unwrap();
        let e = w.layout(W, H).edges[TOP].unwrap().center();
        assert_eq!(w.on_press(e.x, e.y, W, H), Press::Chrome);
        w.on_move(e.x, 220.0, W, H);
        w.on_release(e.x, 220.0, W, H);
        assert_eq!(w.dock_size()[TOP], 200.0, "top dock: y - top strip");

        let mut w = ws();
        let e = w.layout(W, H).edges[BOTTOM].unwrap().center();
        assert_eq!(w.on_press(e.x, e.y, W, H), Press::Chrome);
        w.on_move(e.x, H - 10.0, W, H);
        w.on_release(e.x, H - 10.0, W, H);
        assert_eq!(
            w.dock_size()[BOTTOM],
            w.dock_cross_min(BOTTOM),
            "bottom dock: h - y, clamped to the widest panel's cross min"
        );
    }

    /// A splitter on a horizontal (bottom) dock resizes its panel along x —
    /// the extent is the pointer's distance from the panel's left origin.
    #[test]
    fn horizontal_splitter_drag_sets_extent_from_x() {
        let mut w = ws();
        w.dock_to("left", Place::Docked(BOTTOM)).unwrap();
        let (_, nth, s) = w
            .layout(W, H)
            .splitters
            .iter()
            .copied()
            .find(|&(side, ..)| side == BOTTOM)
            .expect("bottom dock stacks two panels");
        assert_eq!(nth, 0, "the splitter resizes the first bottom panel");
        let c = s.center();
        assert_eq!(w.on_press(c.x, c.y, W, H), Press::Chrome);
        w.on_move(500.0, c.y, W, H);
        w.on_release(500.0, c.y, W, H);
        let i = w.docked(BOTTOM)[0];
        assert_eq!(
            w.panels[i].extent, 500.0,
            "extent = pointer x - panel origin (0)"
        );
    }

    /// A splitter drag whose dock loses its panels mid-drag (e.g. a host
    /// console command re-docking them) is a safe no-op.
    #[test]
    fn splitter_drag_survives_dock_emptied_mid_drag() {
        let mut w = ws();
        let (side, nth, s) = w.layout(W, H).splitters[0];
        assert_eq!((side, nth), (RIGHT, 0));
        let c = s.center();
        assert_eq!(w.on_press(c.x, c.y, W, H), Press::Chrome);
        let before: Vec<f32> = w.panels.iter().map(|p| p.extent).collect();
        w.dock_to("right-a", Place::Floating(100.0, 100.0)).unwrap();
        w.dock_to("right-b", Place::Floating(140.0, 140.0)).unwrap();
        assert!(
            w.on_move(600.0, 500.0, W, H),
            "the drag still owns the move"
        );
        let after: Vec<f32> = w.panels.iter().map(|p| p.extent).collect();
        assert_eq!(before, after, "no extent changes without a dock mate");
        w.on_release(600.0, 500.0, W, H);
    }

    /// Dropping into an occupied dock inserts at the midpoint-based position:
    /// above a vertical dock-mate's midpoint lands before it, and left of a
    /// horizontal mate's midpoint likewise.
    #[test]
    fn drop_inserts_before_the_midpoint_dock_mate() {
        // Vertical: drop "left" near the top of the right dock → stacked first.
        let mut w = ws();
        let i = w.find("left").unwrap();
        let r = w
            .layout(W, H)
            .panels
            .iter()
            .find(|(p, _)| *p == i)
            .unwrap()
            .1;
        assert_eq!(w.on_press(r.x + 40.0, r.y + 8.0, W, H), Press::Chrome);
        // y=100 is above right-a's midpoint but clear of the top-edge peek.
        w.on_move(W - 100.0, 100.0, W, H);
        w.on_release(W - 100.0, 100.0, W, H);
        let ids: Vec<&str> = w
            .docked(RIGHT)
            .iter()
            .map(|&i| w.panels[i].id.as_str())
            .collect();
        assert_eq!(
            ids,
            ["left", "right-a", "right-b"],
            "dropped above right-a's midpoint → stacks first"
        );

        // Horizontal: drop "right-a" near the bottom dock's left edge.
        let i = w.find("right-a").unwrap();
        let r = w
            .layout(W, H)
            .panels
            .iter()
            .find(|(p, _)| *p == i)
            .unwrap()
            .1;
        assert_eq!(w.on_press(r.x + 40.0, r.y + 8.0, W, H), Press::Chrome);
        w.on_move(100.0, H - 50.0, W, H);
        w.on_release(100.0, H - 50.0, W, H);
        let ids: Vec<&str> = w
            .docked(BOTTOM)
            .iter()
            .map(|&i| w.panels[i].id.as_str())
            .collect();
        assert_eq!(
            ids,
            ["right-a", "bottom"],
            "dropped left of the mate's midpoint → stacks first"
        );
    }

    /// `is_dragging` reports only the panel of a live (threshold-crossed)
    /// titlebar drag.
    #[test]
    fn is_dragging_tracks_the_live_titlebar_drag() {
        let mut w = ws();
        let i = w.find("left").unwrap();
        assert!(!w.is_dragging(i), "idle: nothing drags");
        let r = w
            .layout(W, H)
            .panels
            .iter()
            .find(|(p, _)| *p == i)
            .unwrap()
            .1;
        assert_eq!(w.on_press(r.x + 40.0, r.y + 8.0, W, H), Press::Chrome);
        assert!(!w.is_dragging(i), "pressed but not yet past the threshold");
        w.on_move(400.0, 300.0, W, H);
        let i = w.find("left").unwrap(); // raise reordered the vec
        assert!(w.is_dragging(i), "past the threshold the drag is live");
        assert!(
            !w.is_dragging((i + 1) % w.panels.len()),
            "other panels are not dragging"
        );
        w.on_release(400.0, 300.0, W, H);
        assert!(
            !w.is_dragging(w.find("left").unwrap()),
            "the drag ends on release"
        );
    }

    /// `titlebar_of`/`close_of` partition the titlebar band, and both are live
    /// hit areas: the handle starts a chrome drag, the close square hides.
    #[test]
    fn titlebar_and_close_rects_are_live_hit_areas() {
        let mut w = ws();
        let i = w.find("left").unwrap();
        let r = w
            .layout(W, H)
            .panels
            .iter()
            .find(|(p, _)| *p == i)
            .unwrap()
            .1;
        let tb = w.titlebar_of(i, r);
        let cl = w.close_of(i, r);
        assert_eq!(tb.h, TITLEBAR_H, "handle is titlebar-high");
        assert_eq!(
            (cl.w, cl.h),
            (TITLEBAR_H, TITLEBAR_H),
            "close is a titlebar square"
        );
        assert_eq!(cl.x, tb.x + tb.w, "close sits at the handle's right edge");
        let c = tb.center();
        assert_eq!(
            w.on_press(c.x, c.y, W, H),
            Press::Chrome,
            "the handle is draggable chrome"
        );
        w.on_release(c.x, c.y, W, H);
        let c = cl.center();
        assert_eq!(w.on_press(c.x, c.y, W, H), Press::Chrome);
        assert_eq!(
            w.panels[w.find("left").unwrap()].place,
            Place::Hidden,
            "the close square hides the panel"
        );
    }

    /// `peeked_docks` exposes only drag-peeked *empty* docks: nothing when
    /// idle, the emptied side's drop rect during a near-edge drag, and never
    /// an occupied side.
    #[test]
    fn peeked_docks_reports_only_empty_peeked_sides() {
        let mut w = ws();
        assert_eq!(w.peeked_docks(W, H), [None; 4], "idle: no peeks");
        let i = w.find("left").unwrap();
        let r = w
            .layout(W, H)
            .panels
            .iter()
            .find(|(p, _)| *p == i)
            .unwrap()
            .1;
        assert_eq!(w.on_press(r.x + 40.0, r.y + 8.0, W, H), Press::Chrome);
        w.on_move(10.0, 400.0, W, H); // near the (now emptied) left edge
        let peeks = w.peeked_docks(W, H);
        assert!(peeks[LEFT].is_some(), "the emptied left dock peeks open");
        assert_eq!(
            peeks[LEFT],
            w.layout(W, H).docks[LEFT],
            "the peek rect is the dock's drop rect"
        );
        assert_eq!(peeks[RIGHT], None, "occupied docks never peek");
        assert_eq!(peeks[TOP], None);
        assert_eq!(peeks[BOTTOM], None);
        w.on_release(600.0, 400.0, W, H);
    }

    /// `load_layout` skips unknown panel ids (forward compatibility) while
    /// still applying the known ones.
    #[test]
    fn load_layout_skips_unknown_ids() {
        let mut w = ws();
        let mut saved = w.save_layout();
        saved.panels.push(PanelLayout {
            id: "from-the-future".into(),
            place: Place::Docked(TOP),
            w: 100.0,
            h: 100.0,
            extent: 100.0,
        });
        let i = saved.panels.iter().position(|p| p.id == "left").unwrap();
        saved.panels[i].place = Place::Docked(RIGHT);
        w.load_layout(&saved, W, H);
        assert_eq!(
            w.panels[w.find("left").unwrap()].place,
            Place::Docked(RIGHT),
            "known ids are applied"
        );
        assert!(
            w.layout(W, H).docks[TOP].is_none(),
            "the unknown id opened no dock"
        );
    }
}
