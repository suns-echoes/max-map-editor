//! A [`Modal`]: a blocking dialog layer over base content.
//!
//! Unlike the advisory [`Ui::set_blocking`](crate::ui::Ui::set_blocking) flag, a
//! `Modal` is a real widget. When a dialog is open it dims the base with a
//! scrim, centers the dialog, and — because it is the *only* child the tree
//! walkers see while open — routes all hit-testing, focus traversal, and events
//! to the dialog alone. The base stays drawn (dimmed) but inert. It also calls
//! [`EventCtx::block`](crate::widget::EventCtx::block) so the host withholds
//! world input via [`Response::blocking`](crate::interact::Response).
//!
//! The host drives open/close ([`Modal::open`]/[`Modal::close`]) and reads the
//! dialog's widgets back by id with [`Ui::get`](crate::ui::Ui::get) — the same
//! polling model as every other widget.

use crate::color::Rgba;
use crate::draw::DrawList;
use crate::event::{Event, PointerButton};
use crate::geom::{Rect, Size, Vec2};
use crate::interact::{WidgetId, next_id};
use crate::widget::{DrawCtx, EventCtx, LayoutCtx, Widget};

/// The default scrim: a half-opacity black wash over the base content.
const DEFAULT_SCRIM: Rgba = Rgba::rgba(0, 0, 0, 128);

/// A blocking dialog layer wrapping a base widget. Transparent (just the base)
/// until [`open`](Modal::open) installs a dialog; while open the dialog is
/// centered over a scrim and the base is inert.
#[must_use]
pub struct Modal {
    id: WidgetId,
    base: Box<dyn Widget>,
    dialog: Option<Box<dyn Widget>>,
    /// `true` (the default for [`open`](Modal::open)) centers the dialog at its
    /// measured size; `false` ([`open_window`](Modal::open_window)) hands it the
    /// whole viewport so a self-positioning dialog (a [`Window`](crate::Window))
    /// places and drags itself.
    center: bool,
    scrim: Rgba,
    rect: Rect,
}

impl Modal {
    /// Wraps `base` (the always-present content beneath any dialog).
    pub fn new(base: impl Widget + 'static) -> Self {
        Self {
            id: next_id(),
            base: Box::new(base),
            dialog: None,
            center: true,
            scrim: DEFAULT_SCRIM,
            rect: Rect::ZERO,
        }
    }

    /// Sets the scrim color (the wash drawn over the base while a dialog is
    /// open). Use a fully transparent color for a modal with no dimming.
    pub fn scrim(mut self, color: Rgba) -> Self {
        self.scrim = color;
        self
    }

    /// Re-tints the scrim on a live modal — the `&mut` twin of
    /// [`scrim`](Modal::scrim). A persistent modal outlives theme swaps, so a
    /// host whose dim color is theme-derived re-tints in place instead of
    /// rebuilding the widget (and losing its base and open dialog).
    pub fn set_scrim(&mut self, color: Rgba) {
        self.scrim = color;
    }

    /// The modal's own id — the hit-test target for clicks that land on the
    /// scrim (outside the dialog) while open.
    pub fn id(&self) -> WidgetId {
        self.id
    }

    /// Opens `dialog` as the modal layer. It is measured to its content size and
    /// centered on the next layout. Replaces any currently-open dialog.
    pub fn open(&mut self, dialog: impl Widget + 'static) {
        self.dialog = Some(Box::new(dialog));
        self.center = true;
    }

    /// Opens a **self-positioning** dialog (typically a
    /// [`Window`](crate::Window)): instead of centering a fixed sub-rect, the
    /// modal hands the dialog the whole viewport so it places, drags, and resizes
    /// itself. Use `Window::new(title, content).auto_size().centered()` for a
    /// draggable, titled dialog that opens centered and stays put once moved.
    pub fn open_window(&mut self, dialog: impl Widget + 'static) {
        self.dialog = Some(Box::new(dialog));
        self.center = false;
    }

    /// Closes the dialog (no-op if none is open).
    pub fn close(&mut self) {
        self.dialog = None;
    }

    /// True while a dialog is open (the layer is blocking).
    pub fn is_open(&self) -> bool {
        self.dialog.is_some()
    }

    /// The base content (always present).
    pub fn base(&self) -> &dyn Widget {
        self.base.as_ref()
    }

    /// The base content, mutably.
    pub fn base_mut(&mut self) -> &mut dyn Widget {
        self.base.as_mut()
    }

    /// The active layer (the open dialog, else the base) — what hit-testing,
    /// focus, and `get`/`get_mut` recursion see.
    fn active(&self) -> &dyn Widget {
        match &self.dialog {
            Some(d) => d.as_ref(),
            None => self.base.as_ref(),
        }
    }

    fn active_mut(&mut self) -> &mut dyn Widget {
        match &mut self.dialog {
            Some(d) => d.as_mut(),
            None => self.base.as_mut(),
        }
    }
}

impl Widget for Modal {
    fn measure(&mut self, avail: Size, ctx: &mut LayoutCtx) -> Size {
        self.base.measure(avail, ctx);
        if let Some(d) = &mut self.dialog {
            d.measure(avail, ctx);
        }
        avail
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        self.base.arrange(rect, ctx);
        let center = self.center;
        if let Some(d) = &mut self.dialog {
            if center {
                // Center the dialog at its desired size, clamped to the viewport.
                let want = d.measure(rect.size(), ctx);
                let w = want.w.min(rect.w);
                let h = want.h.min(rect.h);
                let x = rect.x + (rect.w - w) * 0.5;
                let y = rect.y + (rect.h - h) * 0.5;
                d.arrange(Rect::new(x, y, w, h), ctx);
            } else {
                // Self-positioning dialog (a Window): give it the whole viewport
                // and let it place/drag/resize itself within.
                d.measure(rect.size(), ctx);
                d.arrange(rect, ctx);
            }
        }
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        // Base draws in both passes (its own popups reach the overlay).
        self.base.draw(dl, ctx);
        if let Some(d) = &self.dialog {
            // Scrim sits above the base, below the dialog — base pass only.
            if ctx.is_base() {
                dl.fill_rect(self.rect, self.scrim);
            }
            d.draw(dl, ctx);
        }
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        if self.dialog.is_some() {
            // Blocking: declare it, route to the dialog, and swallow anything
            // that fell on the scrim so nothing reaches the base.
            ctx.block();
            let handled = self.active_mut().event(ev, ctx);
            if !handled && ev.is_pointer() && ctx.is_target(self.id) {
                // A primary press on the scrim is reported before being
                // swallowed (`Ui::fired` on the modal's id): a host that
                // dismisses on backdrop clicks polls it and closes both its
                // own state and the dialog; every other host ignores it.
                if let Event::PointerButton {
                    button: PointerButton::Primary,
                    pressed: true,
                    ..
                } = ev
                {
                    ctx.fire(self.id, None);
                }
                ctx.consume_pointer();
            }
            return true;
        }
        self.base.event(ev, ctx)
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn id(&self) -> WidgetId {
        self.id
    }

    // Only the active layer is exposed to the tree walkers (focus traversal,
    // `get`/`get_mut`), so an open dialog fully owns input and the base can't be
    // tabbed into or clicked while blocked.
    fn child_count(&self) -> usize {
        1
    }

    fn child(&self, i: usize) -> Option<&dyn Widget> {
        (i == 0).then(|| self.active())
    }

    fn child_mut(&mut self, i: usize) -> Option<&mut dyn Widget> {
        (i == 0).then(|| self.active_mut())
    }

    fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
        if self.dialog.is_some() {
            // The scrim swallows the rest of the viewport, so clicks outside the
            // dialog never reach the base.
            return self
                .active()
                .hit_test(pos)
                .or_else(|| self.rect.contains(pos).then_some(self.id));
        }
        self.base.hit_test(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Modifiers, PointerButton};
    use crate::text::Fonts;
    use crate::theme::Gunmetal;
    use crate::{Button, TextInput, Ui};

    const DEJAVU: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");

    fn press(x: f32, y: f32) -> Event {
        Event::PointerButton {
            button: PointerButton::Primary,
            pressed: true,
            pos: Vec2::new(x, y),
            mods: Modifiers::NONE,
        }
    }

    fn release(x: f32, y: f32) -> Event {
        Event::PointerButton {
            button: PointerButton::Primary,
            pressed: false,
            pos: Vec2::new(x, y),
            mods: Modifiers::NONE,
        }
    }

    fn moved(x: f32, y: f32) -> Event {
        Event::PointerMoved {
            pos: Vec2::new(x, y),
        }
    }

    fn lay(ui: &mut Ui) {
        let mut fonts = Fonts::new();
        let font = fonts.add(DEJAVU.to_vec()).unwrap();
        let theme = Gunmetal::new(font);
        ui.layout(Size::new(200.0, 200.0), &theme, &fonts);
    }

    #[test]
    fn closed_modal_is_transparent_to_the_base() {
        let base = Button::new("base");
        let base_id = base.id();
        let modal = Modal::new(base);
        let mut ui = Ui::new(modal);
        lay(&mut ui);
        // No dialog: nothing blocks, and the base button fires (on release).
        let resp = ui.dispatch(&[press(40.0, 12.0)]);
        assert!(!resp.blocking, "closed modal does not block");
        ui.dispatch(&[release(40.0, 12.0)]);
        assert!(ui.fired(base_id), "base button is reachable when closed");
    }

    #[test]
    fn open_modal_blocks_the_base_and_routes_to_the_dialog() {
        let base = Button::new("base");
        let base_id = base.id();
        let dialog = Button::new("ok");
        let dialog_id = dialog.id();

        let mut modal = Modal::new(base);
        modal.open(dialog);
        assert!(modal.is_open());
        let mut ui = Ui::new(modal);
        lay(&mut ui);

        // A press anywhere reports blocking; the base never fires.
        let resp = ui.dispatch(&[press(40.0, 12.0)]);
        assert!(resp.blocking, "open modal blocks world input");
        assert!(resp.wants_pointer() && resp.wants_keyboard());
        assert!(!ui.fired(base_id), "base is inert under an open dialog");

        // The dialog's own widgets are reachable by id (centered in the
        // viewport) and fire normally.
        let r = ui.get::<Button>(dialog_id).map(|b| b.rect()).unwrap();
        ui.dispatch(&[press(r.x + r.w * 0.5, r.y + r.h * 0.5)]);
        // Button fires on release-inside; press arms it, release fires.
        ui.dispatch(&[Event::PointerButton {
            button: PointerButton::Primary,
            pressed: false,
            pos: Vec2::new(r.x + r.w * 0.5, r.y + r.h * 0.5),
            mods: Modifiers::NONE,
        }]);
        assert!(ui.fired(dialog_id), "dialog button fires");
    }

    #[test]
    fn scrim_press_is_reported_and_still_swallowed() {
        let base = Button::new("base");
        let base_id = base.id();
        let dialog = Button::new("ok");
        let dialog_id = dialog.id();
        let mut modal = Modal::new(base);
        let modal_id = modal.id();
        modal.open(dialog);
        let mut ui = Ui::new(modal);
        lay(&mut ui);

        // The dialog centers in the 200x200 viewport, so a corner press
        // lands on the scrim: reported on the modal's id, swallowed before
        // the base.
        ui.dispatch(&[press(2.0, 2.0)]);
        assert!(ui.fired(modal_id), "a scrim press reports on the modal");
        assert!(!ui.fired(base_id), "the press never reaches the base");

        // A press ON the dialog is not a scrim press (per-dispatch poll).
        let r = ui.get::<Button>(dialog_id).map(|b| b.rect()).unwrap();
        ui.dispatch(&[press(r.center().x, r.center().y)]);
        assert!(!ui.fired(modal_id), "a dialog press reports nothing");

        // Neither is a bare release on the scrim.
        ui.dispatch(&[release(2.0, 2.0)]);
        assert!(!ui.fired(modal_id), "a release alone is not a press");
    }

    #[test]
    fn windowed_dialog_opens_centered_drags_and_blocks() {
        use crate::Window;
        let dlg = Window::new("Settings", Button::new("ok"))
            .auto_size()
            .centered();
        let win_id = dlg.id();
        let mut modal = Modal::new(Button::new("base"));
        modal.open_window(dlg);
        let mut ui = Ui::new(modal);
        lay(&mut ui); // 200x200 viewport

        // A self-positioning dialog opens centered in the viewport (not at the
        // window's default 20,20 corner).
        let p0 = ui.get::<Window>(win_id).unwrap().position();
        assert!(p0.x > 10.0 && p0.y > 10.0, "opened centered, got {p0:?}");

        // Press the titlebar and drag down-right: the window follows and the
        // modal reports blocking the whole time.
        let resp = ui.dispatch(&[press(p0.x + 10.0, p0.y + 5.0)]);
        assert!(resp.blocking, "windowed modal blocks world input");
        ui.dispatch(&[moved(p0.x + 40.0, p0.y + 25.0)]);
        ui.dispatch(&[release(p0.x + 40.0, p0.y + 25.0)]);
        let p1 = ui.get::<Window>(win_id).unwrap().position();
        assert!(p1.x > p0.x && p1.y > p0.y, "dragged from {p0:?} to {p1:?}");
    }

    #[test]
    fn set_scrim_retints_a_live_modal() {
        use crate::draw::DrawCmd;
        let old = Rgba::rgba(10, 20, 30, 200);
        let new = Rgba::rgba(200, 30, 20, 90);
        let mut modal = Modal::new(Button::new("base")).scrim(old);
        let modal_id = modal.id();
        modal.open(Button::new("ok"));
        let mut ui = Ui::new(modal);
        lay(&mut ui);

        let scrim_fills = |ui: &Ui, color: Rgba| {
            let mut fonts = Fonts::new();
            let font = fonts.add(DEJAVU.to_vec()).unwrap();
            let theme = Gunmetal::new(font);
            let mut dl = crate::draw::DrawList::new();
            ui.draw(&mut dl, &theme, &fonts);
            dl.cmds
                .iter()
                .any(|cmd| matches!(cmd, DrawCmd::Solid { color: c, .. } if *c == color))
        };
        assert!(scrim_fills(&ui, old), "the builder scrim draws");

        // A theme swap re-tints in place: the modal keeps its base and its
        // open dialog, only the wash changes.
        ui.get_mut::<Modal>(modal_id).unwrap().set_scrim(new);
        assert!(scrim_fills(&ui, new), "the new scrim draws");
        assert!(!scrim_fills(&ui, old), "the old scrim is gone");
        assert!(
            ui.get::<Modal>(modal_id).unwrap().is_open(),
            "the dialog survived the re-tint"
        );
    }

    #[test]
    fn focus_traversal_targets_the_active_layer() {
        // Closed: focus lands on the base field.
        let base = TextInput::new();
        let base_field = base.id();
        let mut ui = Ui::new(Modal::new(base));
        lay(&mut ui);
        ui.focus_first();
        assert_eq!(ui.focused(), base_field, "closed → the base field focuses");

        // Open: focus lands on the dialog's field, never the base's.
        let base2 = TextInput::new();
        let base2_field = base2.id();
        let dlg = TextInput::new();
        let dlg_field = dlg.id();
        let mut modal2 = Modal::new(base2);
        modal2.open(dlg);
        let mut ui2 = Ui::new(modal2);
        lay(&mut ui2);
        ui2.focus_first();
        assert_eq!(ui2.focused(), dlg_field, "open → the dialog field focuses");
        assert_ne!(
            ui2.focused(),
            base2_field,
            "the base field is out of the cycle"
        );
    }
}
