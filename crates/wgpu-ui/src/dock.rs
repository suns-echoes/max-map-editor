//! A docking area: a binary split tree whose leaves are tabbed groups of
//! panels. Dividers resize by dragging; tabs switch the visible panel and can be
//! dragged onto another leaf's drop zones to re-dock (center = merge into its
//! tabs, an edge = split that leaf). The structure ([`DockLayout`]) is
//! serializable for persistence.
//!
//! **Maturity:** experimental — exercised only by its own tests so far. It is a
//! *different paradigm* from `workspace` (its own feature) (VS-Code-style
//! split tree vs edge-dock + floating windows + center hole); `Workspace` is
//! the battle-tested model with a production consumer. Prefer `Workspace`
//! unless you specifically want split-tree/tabbed-leaf docking, and expect this
//! module's API to move when it gains a real consumer.
//!
//! The tree structure carries no geometry; rectangles are recomputed each
//! `arrange` into a flat cache keyed by node *path* (a sequence of 0/1 turns),
//! which the event handler uses.

use crate::draw::DrawList;
use crate::event::{Event, PointerButton};
use crate::geom::{Rect, Size, Vec2};
use crate::interact::{WidgetId, WidgetState, next_id};
use crate::theme::{Role, TextRole};
use crate::widget::{DrawCtx, EventCtx, LayoutCtx, Widget};

const DIVIDER: f32 = 5.0;
const TAB_H: f32 = 24.0;
const DRAG_THRESHOLD: f32 = 6.0;

/// A drop zone within a leaf region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Center,
    Left,
    Right,
    Top,
    Bottom,
}

/// A persistable description of the dock tree (panels referenced by id).
#[derive(Clone, Debug, PartialEq)]
pub enum DockLayout {
    Tabs {
        panels: Vec<u64>,
        active: usize,
    },
    Split {
        vertical: bool,
        ratio: f32,
        a: Box<DockLayout>,
        b: Box<DockLayout>,
    },
}

struct DockPanel {
    id: u64,
    title: String,
    content: Box<dyn Widget>,
}

enum Node {
    Leaf {
        tabs: Vec<usize>,
        active: usize,
    },
    Split {
        vertical: bool,
        ratio: f32,
        a: Box<Node>,
        b: Box<Node>,
    },
}

impl Node {
    fn leaf(tabs: Vec<usize>) -> Node {
        Node::Leaf { tabs, active: 0 }
    }

    /// The path (0=a, 1=b turns) to the leaf containing panel index `p`.
    fn find(&self, p: usize, path: &mut Vec<u8>) -> bool {
        match self {
            Node::Leaf { tabs, .. } => tabs.contains(&p),
            Node::Split { a, b, .. } => {
                path.push(0);
                if a.find(p, path) {
                    return true;
                }
                path.pop();
                path.push(1);
                if b.find(p, path) {
                    return true;
                }
                path.pop();
                false
            }
        }
    }

    fn at_mut(&mut self, path: &[u8]) -> &mut Node {
        match path.split_first() {
            None => self,
            Some((&0, rest)) => match self {
                Node::Split { a, .. } => a.at_mut(rest),
                _ => self,
            },
            Some((_, rest)) => match self {
                Node::Split { b, .. } => b.at_mut(rest),
                _ => self,
            },
        }
    }

    /// Removes panel `p`; returns true if this node became empty (its parent
    /// should collapse it).
    fn remove(&mut self, p: usize) -> bool {
        match self {
            Node::Leaf { tabs, active } => {
                if let Some(i) = tabs.iter().position(|&t| t == p) {
                    tabs.remove(i);
                    *active = (*active).min(tabs.len().saturating_sub(1));
                }
                tabs.is_empty()
            }
            Node::Split { a, b, .. } => {
                if a.remove(p) {
                    // Collapse: replace self with b.
                    let b = std::mem::replace(b.as_mut(), Node::leaf(Vec::new()));
                    *self = b;
                } else if b.remove(p) {
                    let a = std::mem::replace(a.as_mut(), Node::leaf(Vec::new()));
                    *self = a;
                }
                false
            }
        }
    }

    fn to_layout(&self, panels: &[DockPanel]) -> DockLayout {
        match self {
            Node::Leaf { tabs, active } => DockLayout::Tabs {
                panels: tabs.iter().map(|&i| panels[i].id).collect(),
                active: *active,
            },
            Node::Split {
                vertical,
                ratio,
                a,
                b,
            } => DockLayout::Split {
                vertical: *vertical,
                ratio: *ratio,
                a: Box::new(a.to_layout(panels)),
                b: Box::new(b.to_layout(panels)),
            },
        }
    }

    fn from_layout(l: &DockLayout, id_to_idx: &impl Fn(u64) -> Option<usize>) -> Node {
        match l {
            DockLayout::Tabs { panels, active } => {
                let tabs: Vec<usize> = panels.iter().filter_map(|&id| id_to_idx(id)).collect();
                let active = (*active).min(tabs.len().saturating_sub(1));
                Node::Leaf { tabs, active }
            }
            DockLayout::Split {
                vertical,
                ratio,
                a,
                b,
            } => Node::Split {
                vertical: *vertical,
                ratio: *ratio,
                a: Box::new(Node::from_layout(a, id_to_idx)),
                b: Box::new(Node::from_layout(b, id_to_idx)),
            },
        }
    }
}

/// Cached geometry for one leaf (for hit-testing/drawing in event/draw).
struct LeafGeom {
    path: Vec<u8>,
    rect: Rect,
    tabs: Vec<usize>,
    active: usize,
    /// Per-tab strip widths (parallel to `tabs`), measured at `arrange` time so
    /// `event`/`tab_at` — which have no fonts — hit-test the *same* geometry
    /// `draw` paints, instead of an equal-width approximation.
    tab_widths: Vec<f32>,
}

struct DividerGeom {
    path: Vec<u8>,
    rect: Rect,
    vertical: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum DragState {
    None,
    Divider(usize),
    MaybeTab { panel: usize, start: Vec2 },
    Tab(usize),
}

/// A docking container.
#[must_use]
pub struct DockArea {
    id: WidgetId,
    panels: Vec<DockPanel>,
    root: Node,
    leaves: Vec<LeafGeom>,
    dividers: Vec<DividerGeom>,
    drag: DragState,
    drag_pos: Vec2,
    rect: Rect,
}

impl DockArea {
    pub fn new() -> Self {
        Self {
            id: next_id(),
            panels: Vec::new(),
            root: Node::leaf(Vec::new()),
            leaves: Vec::new(),
            dividers: Vec::new(),
            drag: DragState::None,
            drag_pos: Vec2::ZERO,
            rect: Rect::ZERO,
        }
    }

    /// Adds a panel into the initial (root) tab group.
    pub fn panel(
        mut self,
        id: u64,
        title: impl Into<String>,
        content: impl Widget + 'static,
    ) -> Self {
        let idx = self.panels.len();
        self.panels.push(DockPanel {
            id,
            title: title.into(),
            content: Box::new(content),
        });
        if let Node::Leaf { tabs, .. } = &mut self.root {
            tabs.push(idx);
        }
        self
    }

    pub fn id(&self) -> WidgetId {
        self.id
    }

    /// The current layout, for saving.
    pub fn save_layout(&self) -> DockLayout {
        self.root.to_layout(&self.panels)
    }

    /// Restores a saved layout (panels not present are dropped).
    pub fn load_layout(&mut self, layout: &DockLayout) {
        let ids: Vec<u64> = self.panels.iter().map(|p| p.id).collect();
        let lookup = |id: u64| ids.iter().position(|&x| x == id);
        self.root = Node::from_layout(layout, &lookup);
    }

    /// Splits the leaf containing `anchor_panel` along `zone`, placing
    /// `new_panel` on that side; `Zone::Center` merges into the leaf's tabs.
    fn dock(&mut self, new_panel: usize, anchor_panel: usize, zone: Zone) {
        let mut path = Vec::new();
        if !self.root.find(anchor_panel, &mut path) {
            return;
        }
        let node = self.root.at_mut(&path);
        if zone == Zone::Center {
            if let Node::Leaf { tabs, active } = node {
                tabs.push(new_panel);
                *active = tabs.len() - 1;
            }
            return;
        }
        let (vertical, new_first) = match zone {
            Zone::Left => (false, true),
            Zone::Right => (false, false),
            Zone::Top => (true, true),
            Zone::Bottom => (true, false),
            Zone::Center => unreachable!(),
        };
        let old = std::mem::replace(node, Node::leaf(Vec::new()));
        let new_leaf = Node::leaf(vec![new_panel]);
        let (a, b) = if new_first {
            (Box::new(new_leaf), Box::new(old))
        } else {
            (Box::new(old), Box::new(new_leaf))
        };
        *node = Node::Split {
            vertical,
            ratio: 0.5,
            a,
            b,
        };
    }

    fn zone_at(rect: Rect, p: Vec2) -> Zone {
        let fx = (p.x - rect.x) / rect.w.max(1.0);
        let fy = (p.y - rect.y) / rect.h.max(1.0);
        if (0.33..0.67).contains(&fx) && (0.33..0.67).contains(&fy) {
            Zone::Center
        } else if fx < fy {
            if fx < 1.0 - fy {
                Zone::Left
            } else {
                Zone::Bottom
            }
        } else if fx < 1.0 - fy {
            Zone::Top
        } else {
            Zone::Right
        }
    }
}

impl Default for DockArea {
    fn default() -> Self {
        Self::new()
    }
}

/// Recursively assigns rects, collecting leaf/divider geometry.
fn arrange_node(
    node: &mut Node,
    rect: Rect,
    path: &mut Vec<u8>,
    panels: &mut [DockPanel],
    leaves: &mut Vec<LeafGeom>,
    dividers: &mut Vec<DividerGeom>,
    ctx: &mut LayoutCtx,
) {
    match node {
        Node::Leaf { tabs, active } => {
            let content = Rect::new(rect.x, rect.y + TAB_H, rect.w, (rect.h - TAB_H).max(0.0));
            if let Some(&p) = tabs.get(*active) {
                panels[p].content.arrange(content, ctx);
            }
            // Measure each tab to the same width `draw` uses, so hit-testing in
            // the font-less `event` pass stays in lockstep with what's painted.
            let px = ctx.theme.font_px(TextRole::Body);
            let pad = ctx.theme.metrics().pad;
            let font = ctx.theme.font();
            let tab_widths = tabs
                .iter()
                .map(|&p| ctx.fonts.measure(font, &panels[p].title, px) + 2.0 * pad)
                .collect();
            leaves.push(LeafGeom {
                path: path.clone(),
                rect,
                tabs: tabs.clone(),
                active: *active,
                tab_widths,
            });
        }
        Node::Split {
            vertical,
            ratio,
            a,
            b,
        } => {
            let (ra, rb, div) = if *vertical {
                let h = (rect.h - DIVIDER) * *ratio;
                (
                    Rect::new(rect.x, rect.y, rect.w, h),
                    Rect::new(rect.x, rect.y + h + DIVIDER, rect.w, rect.h - h - DIVIDER),
                    Rect::new(rect.x, rect.y + h, rect.w, DIVIDER),
                )
            } else {
                let w = (rect.w - DIVIDER) * *ratio;
                (
                    Rect::new(rect.x, rect.y, w, rect.h),
                    Rect::new(rect.x + w + DIVIDER, rect.y, rect.w - w - DIVIDER, rect.h),
                    Rect::new(rect.x + w, rect.y, DIVIDER, rect.h),
                )
            };
            dividers.push(DividerGeom {
                path: path.clone(),
                rect: div,
                vertical: *vertical,
            });
            path.push(0);
            arrange_node(a, ra, path, panels, leaves, dividers, ctx);
            path.pop();
            path.push(1);
            arrange_node(b, rb, path, panels, leaves, dividers, ctx);
            path.pop();
        }
    }
}

impl Widget for DockArea {
    fn measure(&mut self, avail: Size, _ctx: &mut LayoutCtx) -> Size {
        avail
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        self.leaves.clear();
        self.dividers.clear();
        let mut path = Vec::new();
        arrange_node(
            &mut self.root,
            rect,
            &mut path,
            &mut self.panels,
            &mut self.leaves,
            &mut self.dividers,
            ctx,
        );
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if !ctx.is_base() {
            // Forward overlay pass to active panels (nested popups).
            for lg in &self.leaves {
                if let Some(&p) = lg.tabs.get(lg.active) {
                    self.panels[p].content.draw(dl, ctx);
                }
            }
            return;
        }
        let px = ctx.theme.font_px(TextRole::Body);
        let pad = ctx.theme.metrics().pad;
        for lg in &self.leaves {
            ctx.theme.panel(dl, lg.rect);
            // Tab strip.
            let mut x = lg.rect.x;
            for (i, &p) in lg.tabs.iter().enumerate() {
                let title = &self.panels[p].title;
                // Same cached width hit-testing uses (falls back to a fresh
                // measure only if the cache is somehow short).
                let tw =
                    lg.tab_widths.get(i).copied().unwrap_or_else(|| {
                        ctx.fonts.measure(ctx.theme.font(), title, px) + 2.0 * pad
                    });
                let tr = Rect::new(x, lg.rect.y, tw, TAB_H);
                ctx.theme.button(
                    dl,
                    tr,
                    Role::Neutral,
                    WidgetState {
                        selected: i == lg.active,
                        ..Default::default()
                    },
                );
                let baseline = Vec2::new(tr.x + pad, tr.center().y + px * 0.34);
                ctx.theme
                    .text(dl, ctx.fonts, baseline, title, TextRole::Body);
                x += tw;
            }
            if let Some(&p) = lg.tabs.get(lg.active) {
                self.panels[p].content.draw(dl, ctx);
            }
        }
        for dg in &self.dividers {
            ctx.theme.frame(
                dl,
                dg.rect,
                ctx.theme.ink_dim(),
                crate::theme::Bevel::Raised,
            );
        }
        // Drag preview + drop zone highlight.
        if let DragState::Tab(p) = self.drag {
            for lg in &self.leaves {
                if lg.rect.contains(self.drag_pos) {
                    let zone = Self::zone_at(lg.rect, self.drag_pos);
                    let hz = zone_rect(lg.rect, zone);
                    dl.fill_rect(hz, ctx.theme.accent().with_alpha(70));
                }
            }
            let title = &self.panels[p].title;
            let tw = ctx.fonts.measure(ctx.theme.font(), title, px) + 2.0 * pad;
            let pr = Rect::new(self.drag_pos.x, self.drag_pos.y, tw, TAB_H);
            ctx.theme
                .button(dl, pr, Role::Primary, WidgetState::default());
            let baseline = Vec2::new(pr.x + pad, pr.center().y + px * 0.34);
            ctx.theme
                .text(dl, ctx.fonts, baseline, title, TextRole::Body);
        }
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        // Active content first (unless dragging a tab).
        if self.drag == DragState::None || matches!(self.drag, DragState::Divider(_)) {
            for li in 0..self.leaves.len() {
                if let Some(&p) = self.leaves[li].tabs.get(self.leaves[li].active)
                    && self.panels[p].content.event(ev, ctx)
                {
                    return true;
                }
            }
        }
        match ev {
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                ..
            } if ctx.is_target(self.id) => {
                let p = ctx.pointer;
                // Divider?
                if let Some(di) = self.dividers.iter().position(|d| d.rect.contains(p)) {
                    self.drag = DragState::Divider(di);
                    ctx.capture(self.id);
                    ctx.consume_pointer();
                    return true;
                }
                // Tab?
                if let Some((panel, is_active_switch)) = self.tab_at(p) {
                    if is_active_switch {
                        self.activate(panel);
                    }
                    self.drag = DragState::MaybeTab { panel, start: p };
                    ctx.capture(self.id);
                    ctx.consume_pointer();
                    return true;
                }
                if self.rect.contains(p) {
                    ctx.consume_pointer();
                    return true;
                }
                false
            }
            Event::PointerMoved { .. } if ctx.is_target(self.id) => {
                self.drag_pos = ctx.pointer;
                match self.drag {
                    DragState::Divider(di) => {
                        self.resize_divider(di, ctx.pointer);
                        ctx.consume_pointer();
                        true
                    }
                    DragState::MaybeTab { panel, start } => {
                        if (ctx.pointer - start).x.abs() + (ctx.pointer - start).y.abs()
                            > DRAG_THRESHOLD
                        {
                            self.drag = DragState::Tab(panel);
                        }
                        ctx.consume_pointer();
                        true
                    }
                    DragState::Tab(_) => {
                        ctx.consume_pointer();
                        true
                    }
                    DragState::None => false,
                }
            }
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: false,
                ..
            } if self.drag != DragState::None => {
                if let DragState::Tab(panel) = self.drag {
                    self.drop_tab(panel, ctx.pointer);
                }
                self.drag = DragState::None;
                ctx.consume_pointer();
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

    fn child_count(&self) -> usize {
        self.panels.len()
    }

    fn child(&self, i: usize) -> Option<&dyn Widget> {
        self.panels.get(i).map(|p| p.content.as_ref())
    }

    fn child_mut(&mut self, i: usize) -> Option<&mut dyn Widget> {
        self.panels.get_mut(i).map(|p| p.content.as_mut())
    }

    fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
        if !self.rect.contains(pos) {
            return None;
        }
        // Content first, then self (tabs/dividers/drag).
        for lg in &self.leaves {
            if let Some(&p) = lg.tabs.get(lg.active) {
                let content = Rect::new(lg.rect.x, lg.rect.y + TAB_H, lg.rect.w, lg.rect.h - TAB_H);
                if content.contains(pos)
                    && let Some(id) = self.panels[p].content.hit_test(pos)
                {
                    return Some(id);
                }
            }
        }
        Some(self.id)
    }
}

impl DockArea {
    /// `(panel index, true)` if a tab is under `p`.
    fn tab_at(&self, p: Vec2) -> Option<(usize, bool)> {
        for lg in &self.leaves {
            if p.y < lg.rect.y || p.y > lg.rect.y + TAB_H {
                continue;
            }
            // Walk the cached per-tab widths so a click lands on exactly the tab
            // drawn under the cursor, whatever the title lengths.
            let mut x = lg.rect.x;
            for (i, &w) in lg.tab_widths.iter().enumerate() {
                if p.x >= x
                    && p.x < x + w
                    && let Some(&panel) = lg.tabs.get(i)
                {
                    return Some((panel, true));
                }
                x += w;
            }
        }
        None
    }

    fn activate(&mut self, panel: usize) {
        let mut path = Vec::new();
        if self.root.find(panel, &mut path)
            && let Node::Leaf { tabs, active } = self.root.at_mut(&path)
            && let Some(i) = tabs.iter().position(|&t| t == panel)
        {
            *active = i;
        }
    }

    fn resize_divider(&mut self, di: usize, p: Vec2) {
        let dg = &self.dividers[di];
        let path = dg.path.clone();
        let vertical = dg.vertical;
        let node = self.root.at_mut(&path);
        if let Node::Split { ratio, .. } = node {
            // Find the split's rect from its first child's leaf or recompute from
            // the divider rect's containing region.
            let region = enclosing(&self.leaves, &self.dividers, &path);
            *ratio = if vertical {
                ((p.y - region.y) / region.h.max(1.0)).clamp(0.05, 0.95)
            } else {
                ((p.x - region.x) / region.w.max(1.0)).clamp(0.05, 0.95)
            };
        }
    }

    fn drop_tab(&mut self, panel: usize, p: Vec2) {
        // Find the target leaf + zone under the pointer.
        let target = self
            .leaves
            .iter()
            .find(|lg| lg.rect.contains(p))
            .map(|lg| (lg.tabs.clone(), lg.active, Self::zone_at(lg.rect, p)));
        let Some((tabs, active, zone)) = target else {
            return;
        };
        // Anchor on a panel that stays put (not the dragged one).
        let anchor = tabs
            .iter()
            .copied()
            .find(|&t| t != panel)
            .or_else(|| tabs.get(active).copied());
        let Some(anchor) = anchor else {
            return; // dropping a lone panel onto its own leaf: no-op
        };
        if anchor == panel {
            return;
        }
        self.root.remove(panel);
        self.dock(panel, anchor, zone);
    }
}

/// The screen region that encloses a split node, from its descendant geometry.
fn enclosing(leaves: &[LeafGeom], dividers: &[DividerGeom], path: &[u8]) -> Rect {
    let mut r: Option<Rect> = None;
    let mut add = |rect: Rect| {
        r = Some(match r {
            None => rect,
            Some(c) => {
                let x0 = c.x.min(rect.x);
                let y0 = c.y.min(rect.y);
                let x1 = c.right().max(rect.right());
                let y1 = c.bottom().max(rect.bottom());
                Rect::new(x0, y0, x1 - x0, y1 - y0)
            }
        });
    };
    for lg in leaves {
        if lg.path.starts_with(path) {
            add(lg.rect);
        }
    }
    for dg in dividers {
        if dg.path.starts_with(path) {
            add(dg.rect);
        }
    }
    r.unwrap_or(Rect::ZERO)
}

/// The highlight rect for a drop `zone` of `rect`.
fn zone_rect(rect: Rect, zone: Zone) -> Rect {
    match zone {
        Zone::Center => rect.inset(crate::geom::Insets::all(rect.w.min(rect.h) * 0.25)),
        Zone::Left => Rect::new(rect.x, rect.y, rect.w * 0.5, rect.h),
        Zone::Right => Rect::new(rect.x + rect.w * 0.5, rect.y, rect.w * 0.5, rect.h),
        Zone::Top => Rect::new(rect.x, rect.y, rect.w, rect.h * 0.5),
        Zone::Bottom => Rect::new(rect.x, rect.y + rect.h * 0.5, rect.w, rect.h * 0.5),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn three() -> DockArea {
        DockArea::new()
            .panel(1, "A", crate::layout::Spacer::new())
            .panel(2, "B", crate::layout::Spacer::new())
            .panel(3, "C", crate::layout::Spacer::new())
    }

    #[test]
    fn panels_start_in_one_tab_group() {
        let d = three();
        match d.save_layout() {
            DockLayout::Tabs { panels, .. } => assert_eq!(panels, vec![1, 2, 3]),
            _ => panic!("expected a single tab group"),
        }
    }

    #[test]
    fn dock_splits_and_remove_collapses() {
        let mut d = three();
        // Split: move panel C (idx 2) to the right of anchor A (idx 0).
        d.root.remove(2);
        d.dock(2, 0, Zone::Right);
        match d.save_layout() {
            DockLayout::Split { vertical, a, b, .. } => {
                assert!(!vertical);
                assert!(matches!(*a, DockLayout::Tabs { ref panels, .. } if *panels == vec![1, 2]));
                assert!(matches!(*b, DockLayout::Tabs { ref panels, .. } if *panels == vec![3]));
            }
            _ => panic!("expected a split"),
        }
        // Remove C → the split collapses back to a single leaf.
        d.root.remove(2);
        assert!(matches!(d.save_layout(), DockLayout::Tabs { .. }));
    }

    #[test]
    fn layout_round_trips() {
        let mut d = three();
        d.root.remove(2);
        d.dock(2, 0, Zone::Bottom);
        let saved = d.save_layout();
        let mut d2 = three();
        d2.load_layout(&saved);
        assert_eq!(d2.save_layout(), saved);
    }

    /// `DockArea::default()` matches `new()`: an empty root tab group.
    #[test]
    fn default_is_an_empty_tab_group() {
        let d = DockArea::default();
        assert_eq!(
            d.save_layout(),
            DockLayout::Tabs {
                panels: vec![],
                active: 0
            },
            "no panels, one empty leaf"
        );
    }

    /// `Zone::Center` merges the docked panel into the anchor leaf's tabs and
    /// makes it the active tab.
    #[test]
    fn dock_center_merges_into_tabs_and_activates() {
        let mut d = three();
        d.root.remove(2);
        d.dock(2, 0, Zone::Center);
        assert_eq!(
            d.save_layout(),
            DockLayout::Tabs {
                panels: vec![1, 2, 3],
                active: 2
            },
            "re-docked panel joins the tabs as the active one"
        );
    }

    /// `Zone::Left`/`Zone::Top` place the new leaf on the `a` (first) side of
    /// the split; removing that leaf collapses the split back to the anchor.
    #[test]
    fn dock_left_and_top_place_the_new_leaf_first() {
        let mut d = three();
        d.root.remove(2);
        d.dock(2, 0, Zone::Left);
        assert_eq!(
            d.save_layout(),
            DockLayout::Split {
                vertical: false,
                ratio: 0.5,
                a: Box::new(DockLayout::Tabs {
                    panels: vec![3],
                    active: 0
                }),
                b: Box::new(DockLayout::Tabs {
                    panels: vec![1, 2],
                    active: 0
                }),
            },
            "left zone: new leaf first in a horizontal split"
        );
        // Removing the a-side leaf collapses the split onto the b side.
        d.root.remove(2);
        assert_eq!(
            d.save_layout(),
            DockLayout::Tabs {
                panels: vec![1, 2],
                active: 0
            },
            "emptied a-side collapses to the remaining leaf"
        );

        let mut d = three();
        d.root.remove(2);
        d.dock(2, 0, Zone::Top);
        assert_eq!(
            d.save_layout(),
            DockLayout::Split {
                vertical: true,
                ratio: 0.5,
                a: Box::new(DockLayout::Tabs {
                    panels: vec![3],
                    active: 0
                }),
                b: Box::new(DockLayout::Tabs {
                    panels: vec![1, 2],
                    active: 0
                }),
            },
            "top zone: new leaf first in a vertical split"
        );
    }

    /// `dock` reaches anchors on either branch of an existing split (`find`
    /// descends `a` then `b`, and `at_mut` follows the recorded path).
    #[test]
    fn dock_reaches_anchors_on_both_split_branches() {
        let mut d = three();
        d.root.remove(2);
        d.dock(2, 0, Zone::Right); // Split{ a: [A, B], b: [C] }
        // Anchor A lives in branch `a`: splitting it nests a split under `a`.
        d.root.remove(1);
        d.dock(1, 0, Zone::Bottom);
        assert_eq!(
            d.save_layout(),
            DockLayout::Split {
                vertical: false,
                ratio: 0.5,
                a: Box::new(DockLayout::Split {
                    vertical: true,
                    ratio: 0.5,
                    a: Box::new(DockLayout::Tabs {
                        panels: vec![1],
                        active: 0
                    }),
                    b: Box::new(DockLayout::Tabs {
                        panels: vec![2],
                        active: 0
                    }),
                }),
                b: Box::new(DockLayout::Tabs {
                    panels: vec![3],
                    active: 0
                }),
            },
            "anchor in branch a: its leaf splits in place"
        );
        // Anchor C lives in branch `b`: merge into its tabs.
        d.root.remove(1);
        d.dock(1, 2, Zone::Center);
        assert_eq!(
            d.save_layout(),
            DockLayout::Split {
                vertical: false,
                ratio: 0.5,
                a: Box::new(DockLayout::Tabs {
                    panels: vec![1],
                    active: 0
                }),
                b: Box::new(DockLayout::Tabs {
                    panels: vec![3, 2],
                    active: 1
                }),
            },
            "anchor in branch b: panel joins its tabs"
        );
    }

    /// Docking against an anchor that is nowhere in the tree is a no-op —
    /// `find` walks both split branches and reports the miss.
    #[test]
    fn dock_with_unknown_anchor_is_a_noop() {
        let mut d = three();
        d.root.remove(2);
        d.dock(2, 0, Zone::Right);
        let before = d.save_layout();
        d.dock(2, 99, Zone::Left); // 99 is no panel index
        assert_eq!(d.save_layout(), before, "unknown anchor changes nothing");
    }

    /// `at_mut` with a path deeper than the tree stops at the last real node
    /// instead of panicking — stale paths (cached geometry from before a
    /// structural change) must be safe.
    #[test]
    fn at_mut_is_safe_on_stale_deep_paths() {
        let mut d = three();
        assert!(
            matches!(d.root.at_mut(&[0, 1]), Node::Leaf { tabs, .. } if tabs.len() == 3),
            "a leaf root absorbs an a-turn"
        );
        assert!(
            matches!(d.root.at_mut(&[1]), Node::Leaf { tabs, .. } if tabs.len() == 3),
            "a leaf root absorbs a b-turn"
        );
    }

    /// `zone_at` partitions a leaf into five drop zones: the middle-third box
    /// is Center and the four corner-cut triangles pick the nearest edge.
    #[test]
    fn zone_at_partitions_a_leaf_into_five_zones() {
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        assert_eq!(DockArea::zone_at(r, Vec2::new(50.0, 50.0)), Zone::Center);
        assert_eq!(DockArea::zone_at(r, Vec2::new(5.0, 50.0)), Zone::Left);
        assert_eq!(DockArea::zone_at(r, Vec2::new(95.0, 50.0)), Zone::Right);
        assert_eq!(DockArea::zone_at(r, Vec2::new(50.0, 5.0)), Zone::Top);
        assert_eq!(DockArea::zone_at(r, Vec2::new(50.0, 95.0)), Zone::Bottom);
    }

    /// `zone_rect` highlights half the leaf for an edge zone and a
    /// quarter-of-the-short-side inset box for Center.
    #[test]
    fn zone_rect_covers_the_zone_half() {
        let r = Rect::new(10.0, 20.0, 100.0, 80.0);
        assert_eq!(zone_rect(r, Zone::Left), Rect::new(10.0, 20.0, 50.0, 80.0));
        assert_eq!(zone_rect(r, Zone::Right), Rect::new(60.0, 20.0, 50.0, 80.0));
        assert_eq!(zone_rect(r, Zone::Top), Rect::new(10.0, 20.0, 100.0, 40.0));
        assert_eq!(
            zone_rect(r, Zone::Bottom),
            Rect::new(10.0, 60.0, 100.0, 40.0)
        );
        assert_eq!(
            zone_rect(r, Zone::Center),
            Rect::new(30.0, 40.0, 60.0, 40.0),
            "center box is inset by a quarter of the short side"
        );
    }

    /// `enclosing` unions the leaf + divider geometry under a path prefix and
    /// falls back to `Rect::ZERO` when nothing matches.
    #[test]
    fn enclosing_unions_descendant_geometry() {
        let leaf = |path: Vec<u8>, rect| LeafGeom {
            path,
            rect,
            tabs: Vec::new(),
            active: 0,
            tab_widths: Vec::new(),
        };
        let leaves = [
            leaf(vec![0], Rect::new(0.0, 0.0, 40.0, 100.0)),
            leaf(vec![1], Rect::new(45.0, 0.0, 55.0, 100.0)),
        ];
        let dividers = [DividerGeom {
            path: vec![],
            rect: Rect::new(40.0, 0.0, 5.0, 100.0),
            vertical: false,
        }];
        assert_eq!(
            enclosing(&leaves, &dividers, &[]),
            Rect::new(0.0, 0.0, 100.0, 100.0),
            "root region spans both leaves and the divider"
        );
        assert_eq!(
            enclosing(&leaves, &dividers, &[1]),
            Rect::new(45.0, 0.0, 55.0, 100.0),
            "a sub-path selects only its own leaf"
        );
        assert_eq!(
            enclosing(&[], &[], &[]),
            Rect::ZERO,
            "no matching geometry yields the zero rect"
        );
    }
}
