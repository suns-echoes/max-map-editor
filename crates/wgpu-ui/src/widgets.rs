//! Themed widgets. Each owns its behavior (so it acts identically everywhere)
//! and draws through the [`Theme`](crate::theme::Theme) (so it has one
//! visualization). This module grows through Phases 4–8; it starts with the two
//! that prove the interactive core: [`Panel`] and [`Button`].

use crate::color::Rgba;
use crate::draw::{DrawList, TexRect, TextureId};
use crate::event::{Event, Key, PointerButton};
use crate::geom::{Insets, Rect, Size, Vec2};
use crate::icon::Icon;
use crate::interact::{CommitPolicy, WidgetId, WidgetState, next_id};
use crate::textedit::TextAlign;
use crate::theme::{
    Emboss, ROW_FLOOR_ACTIVE, ROW_FLOOR_ACTIVE_HOVER, ROW_FLOOR_HOVER, Role, TextRole,
};
use crate::widget::{DrawCtx, EventCtx, LayoutCtx, Semantics, Widget, kind_of};

/// A clickable button. Behavior is fixed (hover → arm-on-press →
/// fire-on-release-inside by default); look comes entirely from the theme's
/// `button` part. Poll the outcome with [`Ui::fired`](crate::ui::Ui::fired) using
/// [`Button::id`].
#[must_use]
pub struct Button {
    id: WidgetId,
    label: String,
    role: Role,
    /// The em size the label is measured and drawn at — see
    /// [`text_role`](Self::text_role).
    text_role: TextRole,
    commit: CommitPolicy,
    disabled: bool,
    /// Externally-driven "active" state: a command button that reflects current
    /// state (the active tool, current layer). Draws the selected face and an
    /// accent label. Distinct from a [`Toggle`], which owns its on/off latch.
    selected: bool,
    /// Dims the *label* while the button stays live — see [`muted`](Self::muted).
    muted: bool,
    action: Option<u64>,
    focusable: bool,
    armed: bool,
    /// An explicit preferred size — see [`sized`](Self::sized). `None` measures
    /// the label against the theme's metrics.
    size: Option<Size>,
    /// Paints no face at all — see [`flat`](Self::flat).
    flat: bool,
    /// A stencil face instead of the label — see [`icon`](Self::icon).
    icon: Option<Icon>,
    /// A hover tooltip — see [`tooltip`](Self::tooltip).
    tooltip: Option<String>,
    /// A semantics name decoupled from the visible face — see
    /// [`semantics_label`](Self::semantics_label).
    semantics_label: Option<String>,
    rect: Rect,
}

impl Button {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            id: next_id(),
            label: label.into(),
            role: Role::Neutral,
            text_role: TextRole::Body,
            commit: CommitPolicy::ReleaseInside,
            disabled: false,
            selected: false,
            muted: false,
            action: None,
            focusable: false,
            armed: false,
            size: None,
            flat: false,
            icon: None,
            tooltip: None,
            semantics_label: None,
            rect: Rect::ZERO,
        }
    }

    /// The id to poll for clicks (keep this; the widget moves into the tree).
    pub fn id(&self) -> WidgetId {
        self.id
    }

    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    pub fn primary(self) -> Self {
        self.role(Role::Primary)
    }

    /// The secondary / alternate CTA (Cancel, Abort).
    pub fn secondary(self) -> Self {
        self.role(Role::Secondary)
    }

    pub fn danger(self) -> Self {
        self.role(Role::Danger)
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Externally marks the button active (lit face + accent label) — for a
    /// command button that mirrors current state. The host re-syncs it each
    /// frame with [`set_selected`](Button::set_selected).
    pub fn with_selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn set_selected(&mut self, selected: bool) {
        self.selected = selected;
    }

    /// Whether the button currently reads as active. The counterpart of
    /// [`set_selected`](Self::set_selected): a host that pushes state into the
    /// tree every frame can read back what the key is actually showing, the
    /// way [`Checkbox::checked`] and [`Toggle::on`] already allow.
    pub fn selected(&self) -> bool {
        self.selected
    }

    /// The em size the label is measured and drawn at ([`TextRole::Body`] by
    /// default — a dialog's buttons).
    ///
    /// [`TextRole::Small`] is the **dense key bank**: a panel of fixed-width
    /// command keys packed several to a row, where the body face would not fit
    /// the words. It also matches a captioned
    /// [`ColorButton`](ColorButton::label), which is always small — so a key
    /// and a swatch key sitting side by side in the same row read alike.
    pub fn text_role(mut self, role: TextRole) -> Self {
        self.text_role = role;
        self
    }

    /// Compact face — [`text_role`](Self::text_role) with [`TextRole::Small`].
    pub fn small(self) -> Self {
        self.text_role(TextRole::Small)
    }

    /// Pins the button's preferred size, overriding both the measured label
    /// width (with its `button_min_width` floor) and `control_height`.
    ///
    /// This is what a key in a **flow** needs. A [`Linear`](crate::Linear) row
    /// sizes each child with a per-child [`Length`](crate::widget::Length), so a
    /// key bank laid out in rows pins its metrics there; a
    /// [`Wrap`](crate::Wrap) has no such knob — every child takes its *measured*
    /// size — so a wrapping toolbar of compact keys could otherwise only be as
    /// small as the theme's dialog-button minimum. [`ColorButton::new`] has
    /// always taken its cell size for the same reason; this is its plain
    /// counterpart.
    pub fn sized(mut self, w: f32, h: f32) -> Self {
        self.size = Some(Size::new(w, h));
        self
    }

    /// Re-pins the size in place — for a key whose metrics are only known once
    /// there is a [`LayoutCtx`](crate::widget::LayoutCtx) to measure with.
    ///
    /// A bank of keys sharing one width (the widest label's) cannot compute that
    /// width in a constructor: the font and the text px belong to the theme the
    /// host draws with. Its parent measures it, so the size has to be set
    /// *before* that — a host resolves it at `arrange` and pushes it down.
    /// Rebuilding the key instead would mint a new [`WidgetId`] and drop the
    /// hover and the arming hanging off the old one.
    pub fn set_size(&mut self, w: f32, h: f32) {
        self.size = Some(Size::new(w, h));
    }

    /// Drops the face: the button paints only the theme's
    /// [`wash`](crate::Theme::wash) when hovered or pressed, under its label. For
    /// an **icon affordance inside another control** — a tab's close `x`, a
    /// field's clear key — where a face of its own would read as a second,
    /// nested button. It arms, fires, carries its action and answers a hit
    /// exactly like any other button; only the paint changes.
    ///
    /// Pair it with [`sized`](Self::sized): a frameless key is normally as small
    /// as its glyph, and the measured default carries the theme's dialog-button
    /// floor.
    pub fn flat(mut self) -> Self {
        self.flat = true;
        self
    }

    /// A stencil [`Icon`] face instead of the label: the compact square **tool
    /// key** of an icon toolbar. The icon is stamped through
    /// [`Theme::icon`](crate::theme::Theme::icon) in the same state-driven ink
    /// the label would use — accent when selected, dim when muted — at the
    /// largest whole multiple of its 16-cell grid that fits the face, so it
    /// scales chunky-crisp. The label is then **not drawn**; keep it anyway
    /// (or set [`tooltip`](Self::tooltip)) so the key still says what it does —
    /// an icon-only key with neither is mute to semantics *and* to hover.
    ///
    /// Unsized, an icon key measures a `control_height` square instead of the
    /// label-width default.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Replaces the icon in place — a key whose glyph tracks host state.
    pub fn set_icon(&mut self, icon: Icon) {
        self.icon = Some(icon);
    }

    /// A hover tooltip: once the `Ui` reports the pointer has **rested** on
    /// this key, the overlay pass paints `text` on a small plate under it
    /// (through [`Theme::tooltip`](crate::theme::Theme::tooltip), clamped to
    /// the viewport). The standard caption for an [`icon`](Self::icon)-faced
    /// key, whose face no longer says what it does.
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    /// A semantics name decoupled from the visible face, for a **text**-faced
    /// key whose face is a glyph rather than a word — a formatting bar's "B"
    /// key that must still read as "Bold" to queries and assistive tech. The
    /// face keeps painting [`label`](Self::new); `semantics` reports this name
    /// instead. Icon keys don't need it — their label never paints, so it
    /// already *is* the semantics name.
    pub fn semantics_label(mut self, name: impl Into<String>) -> Self {
        self.semantics_label = Some(name.into());
        self
    }

    /// Dims the label while leaving the button **live** — it hovers, arms,
    /// fires and carries its action exactly like any other. For a key that
    /// works but reads as lesser: a placeholder that only echoes what it would
    /// do, a secondary command in a dense group.
    ///
    /// This is *not* [`disabled`](Self::disabled), which paints the disabled
    /// face and swallows the click. [`with_selected`](Self::with_selected)
    /// outranks it — a lit key reads accent even if it was declared muted.
    pub fn muted(mut self, muted: bool) -> Self {
        self.muted = muted;
        self
    }

    /// Mutes/unmutes in place — for a button whose weight tracks host state.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    /// Whether the label currently reads dim — the counterpart of
    /// [`set_muted`](Self::set_muted), and the only way a host can assert that
    /// a key is muted *rather than* [`is_disabled`](Self::is_disabled): the two
    /// look similar and behave oppositely, so "dim but live" is a claim worth
    /// being able to check.
    pub fn is_muted(&self) -> bool {
        self.muted
    }

    /// Whether the button paints the disabled face and swallows its click.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Attaches an action tag emitted (alongside the click) when this fires.
    pub fn action(mut self, tag: u64) -> Self {
        self.action = Some(tag);
        self
    }

    pub fn commit(mut self, commit: CommitPolicy) -> Self {
        self.commit = commit;
        self
    }

    /// Opts the button into Tab focus traversal. Once focused (via Tab or a
    /// click), Enter/Space activates it. Off by default so pointer-first hosts
    /// keep their Tab order to fields and dropdowns only.
    pub fn focusable(mut self) -> Self {
        self.focusable = true;
        self
    }

    /// Replaces the label — a command button whose caption tracks host state
    /// (Start/Stop, Close/Abort), re-synced by the host each frame.
    pub fn set_label(&mut self, label: impl Into<String>) {
        self.label = label.into();
    }

    /// Replaces the role — a button whose face tracks host state (a neutral
    /// Close that becomes an amber Abort while a run is live).
    pub fn set_role(&mut self, role: Role) {
        self.role = role;
    }

    /// Enables/disables in place — a command whose availability tracks host
    /// state (a Paste that greys out until the clipboard holds something).
    pub fn set_disabled(&mut self, disabled: bool) {
        self.disabled = disabled;
        if disabled {
            self.armed = false;
        }
    }

    /// Replaces or clears the hover tooltip in place — for a caption that
    /// tracks host state. A disabled key still hit-tests and still reports its
    /// tooltip, so a host that greys a key out can hang the unmet precondition
    /// here and clear it once the key goes live.
    pub fn set_tooltip(&mut self, tooltip: Option<String>) {
        self.tooltip = tooltip;
    }
}

impl Widget for Button {
    fn semantics(&self) -> Semantics<'_> {
        let name = self.semantics_label.as_deref().unwrap_or(&self.label);
        Semantics::labeled(kind_of::<Self>(), name)
    }

    fn measure(&mut self, _avail: Size, ctx: &mut LayoutCtx) -> Size {
        if let Some(size) = self.size {
            return size;
        }
        let m = ctx.theme.metrics();
        // An icon key measures a square tool-palette cell; a label key is as
        // wide as its words.
        if self.icon.is_some() {
            return Size::new(m.control_height, m.control_height);
        }
        let px = ctx.theme.font_px(self.text_role);
        let tw = ctx.fonts.measure(ctx.theme.font(), &self.label, px);
        Size::new((tw + 2.0 * m.pad).max(m.button_min_width), m.control_height)
    }

    fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if !ctx.is_base() {
            return;
        }
        let state = WidgetState {
            hovered: ctx.is_hovered(self.id),
            pressed: self.armed,
            focused: ctx.is_focused(self.id),
            disabled: self.disabled,
            selected: self.selected,
        };
        // A frameless key marks itself with the theme's wash alone; everything
        // below (the label's ink, the clip) is the same either way.
        if self.flat {
            ctx.theme.wash(dl, self.rect, state);
        } else {
            ctx.theme.button(dl, self.rect, self.role, state);
        }
        // The face's ink is the same four-way for a glyph and for words: dim
        // when disabled, accent when selected, dim when muted, the theme's ink
        // otherwise. **Disabled outranks the rest** - a face material alone is a
        // thin signal (on a dense band of small keys it is nearly invisible),
        // and a key that swallows its click has to say so in the one part of it
        // anybody reads.
        if let Some(icon) = self.icon {
            let ink = if self.disabled || self.muted {
                ctx.theme.ink_dim()
            } else if self.selected {
                ctx.theme.accent()
            } else {
                ctx.theme.ink()
            };
            // The art drawn for this key at this UI scale, in a box where one
            // cell is one physical pixel — a 24px key gets the 16-cell stencil
            // at 100%, the 20-cell at 125%, the 24-cell at 150%.
            let (stencil, cell) = crate::icon::fit(icon, self.rect, ctx.scale);
            dl.push_clip(self.rect);
            ctx.theme.icon(dl, cell, stencil, Emboss::Raised, ink);
            dl.pop_clip();
            return;
        }
        let role = self.text_role;
        let px = ctx.theme.font_px(role);
        let tw = ctx.fonts.measure(ctx.theme.font(), &self.label, px);
        let c = self.rect.center();
        let baseline = Vec2::new(c.x - tw * 0.5, c.y + px * 0.34);
        // Clipped to the face: a label too long for its key is cut off at its own
        // edge instead of painting across the button beside it (a packed key bank
        // of fixed-width keys is the case that finds this).
        dl.push_clip(self.rect);
        // A button is a raised face: full emboss (hilite + shadow). An active
        // (selected) command button lights its label in the accent ink; a muted
        // one dims it without touching the face (the button stays live), and a
        // disabled one dims it *as well as* wearing the disabled face.
        // Unmuted-unselected goes through `text_em` so a theme that varies ink
        // by role still decides this button's ink.
        if self.selected && !self.disabled {
            ctx.theme
                .text_accent(dl, ctx.fonts, baseline, &self.label, role, Emboss::Raised);
        } else if self.muted || self.disabled {
            ctx.theme.text_colored(
                dl,
                ctx.fonts,
                baseline,
                &self.label,
                role,
                Emboss::Raised,
                ctx.theme.ink_dim(),
            );
        } else {
            ctx.theme
                .text_em(dl, ctx.fonts, baseline, &self.label, role, Emboss::Raised);
        }
        dl.pop_clip();
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        click_machine(
            ev,
            ctx,
            self.id,
            self.rect,
            self.disabled,
            self.commit,
            self.action,
            &mut self.armed,
            || {},
        )
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn id(&self) -> WidgetId {
        self.id
    }

    fn accepts_focus(&self) -> bool {
        self.focusable && !self.disabled
    }

    fn tooltip(&self) -> Option<&str> {
        self.tooltip.as_deref()
    }
}

/// A themed container: paints the theme's `panel` background behind a single
/// padded child.
#[must_use]
pub struct Panel {
    child: Box<dyn Widget>,
    padding: Insets,
    rect: Rect,
}

impl Panel {
    pub fn new(child: impl Widget + 'static) -> Self {
        Self {
            child: Box::new(child),
            padding: Insets::all(8.0),
            rect: Rect::ZERO,
        }
    }

    pub fn padding(mut self, padding: Insets) -> Self {
        self.padding = padding;
        self
    }
}

impl Widget for Panel {
    fn measure(&mut self, avail: Size, ctx: &mut LayoutCtx) -> Size {
        let inner = Size::new(
            (avail.w - self.padding.horizontal()).max(0.0),
            (avail.h - self.padding.vertical()).max(0.0),
        );
        let c = self.child.measure(inner, ctx);
        Size::new(
            c.w + self.padding.horizontal(),
            c.h + self.padding.vertical(),
        )
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        self.child.arrange(rect.inset(self.padding), ctx);
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        // Background is chrome: paint it in the base pass only. The overlay pass
        // exists to layer popups on top, so repainting the opaque background
        // there would erase any child that only draws in the base pass (e.g. a
        // closed `Select`). Children still draw in both passes (so their own
        // popups reach the overlay).
        if ctx.is_base() {
            ctx.theme.panel(dl, self.rect);
        }
        self.child.draw(dl, ctx);
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        if self.child.event(ev, ctx) {
            return true;
        }
        // Panels are opaque: a pointer event landing on the panel is consumed so
        // it can't fall through to the host's world behind it.
        if ev.is_pointer() && ev.pos().is_some_and(|p| self.rect.contains(p)) {
            ctx.consume_pointer();
            return true;
        }
        false
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn child_count(&self) -> usize {
        1
    }

    fn child(&self, i: usize) -> Option<&dyn Widget> {
        (i == 0).then_some(self.child.as_ref())
    }

    fn child_mut(&mut self, i: usize) -> Option<&mut dyn Widget> {
        (i == 0).then_some(self.child.as_mut())
    }

    fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
        if self.rect.contains(pos) {
            self.child.hit_test(pos)
        } else {
            None
        }
    }
}

/// Centers a single line of `text` vertically in `rect` and draws it left-
/// aligned starting at `x`, via the theme. `muted` draws in the secondary ink.
fn draw_label(
    dl: &mut DrawList,
    ctx: &DrawCtx,
    rect: Rect,
    x: f32,
    text: &str,
    role: TextRole,
    muted: bool,
) {
    let px = ctx.theme.font_px(role);
    let baseline = Vec2::new(x, rect.center().y + px * 0.34);
    if muted {
        ctx.theme.text_muted(dl, ctx.fonts, baseline, text, role);
    } else {
        ctx.theme.text(dl, ctx.fonts, baseline, text, role);
    }
}

fn text_width(
    ctx_fonts: &crate::text::Fonts,
    theme: &dyn crate::theme::Theme,
    s: &str,
    role: TextRole,
) -> f32 {
    ctx_fonts.measure(theme.font(), s, theme.font_px(role))
}

/// Black or white, whichever reads on `bg` — for a caption drawn over a
/// **host-supplied** color (a [`ColorButton`]'s swatch), where no fixed chrome
/// ink is legible across the whole range. Uses the Rec. 709 luma of the sRGB
/// values directly; the extra accuracy of linearizing first would not move the
/// black/white decision.
fn contrast_ink(bg: Rgba) -> Rgba {
    let luma = 0.2126 * f32::from(bg.r) + 0.7152 * f32::from(bg.g) + 0.0722 * f32::from(bg.b);
    if luma > 140.0 {
        Rgba::BLACK
    } else {
        Rgba::WHITE
    }
}

/// A non-interactive line of text. Give it an id with [`Label::with_id`] when the
/// host needs to rewrite the text each frame (a status line, a coordinate
/// readout) via [`Ui::get_mut`](crate::ui::Ui::get_mut).
#[must_use]
pub struct Label {
    id: WidgetId,
    text: String,
    role: TextRole,
    muted: bool,
    /// An explicit ink override (e.g. a red warning). `None` uses the theme's
    /// role/muted ink; `Some` draws every line in this color via `text_colored`.
    color: Option<Rgba>,
    /// When set, the label word-wraps to its arranged width and measures to the
    /// resulting multi-line height; `wrapped` caches the lines (recomputed at
    /// arrange against the final width).
    wrap: bool,
    /// With [`wrap`](Self::wrap): wrap at this width instead of the available /
    /// arranged one, so the measured size stays put while the text changes.
    wrap_w: Option<f32>,
    /// With [`wrap`](Self::wrap): measure exactly this many lines tall,
    /// whatever the text — see [`fixed_lines`](Self::fixed_lines).
    fixed_lines: Option<usize>,
    wrapped: Vec<String>,
    /// An explicit emboss, overriding the role's default — see
    /// [`emboss`](Self::emboss).
    emboss: Option<Emboss>,
    /// Where the text sits inside the arranged rect — see [`align`](Self::align).
    align: TextAlign,
    /// When set, text wider than the arranged rect is truncated with a trailing
    /// `...` instead of running past it — see [`ellipsize`](Self::ellipsize).
    ellipsize: bool,
    rect: Rect,
}

impl Label {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            id: WidgetId::NONE,
            text: text.into(),
            role: TextRole::Body,
            muted: false,
            color: None,
            wrap: false,
            wrap_w: None,
            fixed_lines: None,
            wrapped: Vec::new(),
            emboss: None,
            align: TextAlign::Left,
            ellipsize: false,
            rect: Rect::ZERO,
        }
    }

    /// Assigns a stable id so the host can update the text via
    /// [`Ui::get_mut`](crate::ui::Ui::get_mut). The label stays decorative (never
    /// a pointer target). Keep the returned [`Label::id`].
    pub fn with_id(mut self) -> Self {
        self.id = next_id();
        self
    }

    pub fn id(&self) -> WidgetId {
        self.id
    }

    pub fn role(mut self, role: TextRole) -> Self {
        self.role = role;
        self
    }

    pub fn small(self) -> Self {
        self.role(TextRole::Small)
    }

    pub fn title(self) -> Self {
        self.role(TextRole::Title)
    }

    /// Draws in the theme's secondary / dim ink (hints, readouts, captions).
    pub fn muted(mut self) -> Self {
        self.muted = true;
        self
    }

    /// Switches the ink between live and dim in place (the id stays valid across
    /// frames), so a retained readout whose row is live in one selection and
    /// read-only in the next is *synced* rather than rebuilt — rebuilding is what
    /// a tree with stable ids cannot do.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    /// Whether this label currently reads as secondary ink.
    pub fn is_muted(&self) -> bool {
        self.muted
    }

    /// Draws every line in an explicit `ink` (e.g. a red warning) via the theme's
    /// `text_colored`, overriding the role/muted ink. The role still sets the em
    /// size + emboss.
    pub fn color(mut self, ink: Rgba) -> Self {
        self.color = Some(ink);
        self
    }

    /// Repaints the ink in place — `None` returns to the theme's role/muted ink.
    /// The counterpart of [`set_muted`](Self::set_muted), for a caption whose
    /// ink *is* state: a tab that is active, dirty, or an open save file. A
    /// retained tree cannot answer that by rebuilding — the rebuild mints a new
    /// id and hover, arming and focus hang off the old one.
    pub fn set_color(&mut self, ink: Option<Rgba>) {
        self.color = ink;
    }

    /// The explicit ink, if any — the reader for [`set_color`](Self::set_color),
    /// so a host can assert which of the three states a caption is showing.
    pub fn ink(&self) -> Option<Rgba> {
        self.color
    }

    /// Draws with an explicit [`Emboss`] instead of the one the **role** implies
    /// (`Raised` for [`TextRole::Title`], `Engraved` otherwise).
    ///
    /// The chrome's rule is that raised text sits on a raised face — a button, a
    /// titlebar, a heading — and engraved text sits on content. A label placed
    /// *on* a face (the caption of a tab, of a custom key) is raised text by that
    /// rule, and this is the only way to say so; without it, the host has to draw
    /// the line by hand, which is the per-panel drawing the toolkit exists to
    /// remove.
    pub fn emboss(mut self, emboss: Emboss) -> Self {
        self.emboss = Some(emboss);
        self
    }

    /// [`emboss`](Self::emboss)`(Emboss::Raised)` — a caption on a raised face.
    pub fn raised(self) -> Self {
        self.emboss(Emboss::Raised)
    }

    /// Word-wraps the text to the label's arranged width across multiple lines
    /// (for paragraphs, error bodies, hints). The label then measures to the
    /// wrapped height, so give it a bounded width — a `CrossAlign::Stretch`
    /// child of a column, or a fixed/flex width — not `Length::Fit`.
    pub fn wrap(mut self) -> Self {
        self.wrap = true;
        self
    }

    /// Like [`wrap`](Self::wrap), but wraps at width `w` instead of the
    /// available/arranged one, so the measured size never follows the text.
    /// For *changing* text (a hint, a validation error) inside an auto-sized
    /// window: a plain `wrap` measures against the whole available width, so
    /// a longer line still widens the window — `wrap_at` pins the wrap width
    /// (normally the dialog's content width) and the window stays put. Pair
    /// it with a fixed-height slot tall enough for the worst case.
    pub fn wrap_at(mut self, w: f32) -> Self {
        self.wrap = true;
        self.wrap_w = Some(w);
        self
    }

    /// With [`wrap_at`](Self::wrap_at): measure **exactly** `n` lines tall at
    /// this label's own role and font, whatever the text — a reserved *slot*
    /// for changing text (a hover hint, a validation error), so an auto-sized
    /// window never wobbles as the text changes. Overflow clips at the slot's
    /// edge, exactly as it does in a host-fixed slot — and this is what
    /// replaces that host's `n × hardcoded-line-height` arithmetic, which
    /// drifts the moment the font or the role's px changes.
    pub fn fixed_lines(mut self, n: usize) -> Self {
        self.fixed_lines = Some(n);
        self
    }

    /// Where the text sits inside the label's **arranged** rect (default
    /// [`TextAlign::Left`]). Only a label given more width than its text has
    /// slack to distribute — a [`Length::Fit`](crate::widget::Length) child is
    /// arranged at exactly its measured width, so align it by giving it a flex
    /// or fixed slot to sit in (a right-aligned readout at the end of a header
    /// row).
    pub fn align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    /// Truncates text wider than the arranged rect with a trailing `...`
    /// instead of letting it run over whatever sits beside it — a **changing**
    /// readout (a file name, a selected item's tag) in a slot that cannot grow.
    /// The draw is clipped to the rect too, so the marker is the last thing
    /// drawn rather than the first thing cut off. A plain label is not clipped:
    /// it measures to its own text, so its rect only fails to hold it when a
    /// host deliberately gave it less, and shortening that silently would hide
    /// text the host meant to show.
    ///
    /// The marker is three ASCII dots rather than `…` (U+2026) so it renders in
    /// a font that carries only the ASCII range.
    pub fn ellipsize(mut self) -> Self {
        self.ellipsize = true;
        self
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }

    /// Where `line` starts, given the label's alignment and the slack its
    /// arranged rect leaves. A line wider than the rect has no slack and starts
    /// at the left edge whatever the alignment (the clip takes the overflow).
    fn line_x(&self, ctx: &DrawCtx, line: &str) -> f32 {
        let free = (self.rect.w - text_width(ctx.fonts, ctx.theme, line, self.role)).max(0.0);
        self.rect.x + free * self.align.factor()
    }
}

impl Widget for Label {
    fn semantics(&self) -> Semantics<'_> {
        Semantics::labeled(kind_of::<Self>(), &self.text)
    }

    fn measure(&mut self, avail: Size, ctx: &mut LayoutCtx) -> Size {
        let px = ctx.theme.font_px(self.role);
        if self.wrap {
            let font = ctx.theme.font();
            let max_w = self.wrap_w.map_or(avail.w, |w| w.min(avail.w)).max(0.0);
            let lines = ctx.fonts.wrap(font, &self.text, px, max_w);
            let w = lines
                .iter()
                .map(|l| ctx.fonts.measure(font, l, px))
                .fold(0.0_f32, f32::max);
            // A fixed-lines label is a slot: its height is the slot's, not the
            // text's, so the text changing can never resize the tree.
            let n = self.fixed_lines.unwrap_or(lines.len().max(1));
            let h = ctx.fonts.line_height(font, px) * n as f32;
            return Size::new(w.min(max_w), h);
        }
        let w = text_width(ctx.fonts, ctx.theme, &self.text, self.role);
        Size::new(w, px * 1.3)
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        if self.wrap {
            let px = ctx.theme.font_px(self.role);
            let w = self.wrap_w.map_or(rect.w, |w| w.min(rect.w)).max(0.0);
            self.wrapped = ctx.fonts.wrap(ctx.theme.font(), &self.text, px, w);
        }
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if !ctx.is_base() {
            return;
        }
        // An explicit-ink label draws every line via `text_colored` (the role's
        // default emboss unless one was named); the role/muted path handles the
        // rest.
        let emboss = self.emboss.unwrap_or(if self.role == TextRole::Title {
            Emboss::Raised
        } else {
            Emboss::Engraved
        });
        if self.wrap {
            let px = ctx.theme.font_px(self.role);
            let lh = ctx.fonts.line_height(ctx.theme.font(), px);
            // Top-aligned: first baseline ~one em below the top edge, then step
            // by the line height for each wrapped line. Clipped to the arranged
            // rect, so text taller than a fixed slot never paints over the
            // content below it.
            dl.push_clip(self.rect);
            let mut y = self.rect.y + px;
            for line in &self.wrapped {
                let baseline = Vec2::new(self.line_x(ctx, line), y);
                if let Some(ink) = self.color {
                    ctx.theme
                        .text_colored(dl, ctx.fonts, baseline, line, self.role, emboss, ink);
                } else if self.muted {
                    ctx.theme
                        .text_muted(dl, ctx.fonts, baseline, line, self.role);
                } else if self.emboss.is_some() {
                    ctx.theme
                        .text_em(dl, ctx.fonts, baseline, line, self.role, emboss);
                } else {
                    ctx.theme.text(dl, ctx.fonts, baseline, line, self.role);
                }
                y += lh;
            }
            dl.pop_clip();
            return;
        }
        // The line as it is actually drawn: cut to the rect (and clipped to it)
        // when the label ellipsizes, then placed by `align` in whatever slack is
        // left. A plain label draws its text whole, wherever the host's rect
        // put it.
        // Borrowed when the text is drawn whole — which is the common case, and
        // this is a per-frame draw path with dozens of labels in a panel.
        let shown: std::borrow::Cow<'_, str> = if self.ellipsize {
            std::borrow::Cow::Owned(ctx.theme.ellipsized(
                ctx.fonts,
                &self.text,
                self.role,
                self.rect.w,
            ))
        } else {
            std::borrow::Cow::Borrowed(self.text.as_str())
        };
        let x = self.line_x(ctx, &shown);
        if self.ellipsize {
            dl.push_clip(self.rect);
        }
        if let Some(ink) = self.color {
            let px = ctx.theme.font_px(self.role);
            let baseline = Vec2::new(x, self.rect.center().y + px * 0.34);
            ctx.theme
                .text_colored(dl, ctx.fonts, baseline, &shown, self.role, emboss, ink);
        } else if self.emboss.is_some() && !self.muted {
            // Only an *explicit* emboss takes this path: `text` already applies
            // the role's default, and a theme is free to vary ink between the two
            // calls, so routing every label through `text_em` would repaint every
            // existing one.
            let px = ctx.theme.font_px(self.role);
            let baseline = Vec2::new(x, self.rect.center().y + px * 0.34);
            ctx.theme
                .text_em(dl, ctx.fonts, baseline, &shown, self.role, emboss);
        } else {
            draw_label(dl, ctx, self.rect, x, &shown, self.role, self.muted);
        }
        if self.ellipsize {
            dl.pop_clip();
        }
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn id(&self) -> WidgetId {
        self.id
    }

    /// Decorative: an id (for `get_mut`) must not make the label a pointer
    /// target, so it never claims a hit.
    fn hit_test(&self, _pos: Vec2) -> Option<WidgetId> {
        None
    }
}

/// Side of the checkbox/radio box, in logical px.
const BOX: f32 = 16.0;

/// A labelled checkbox. Toggles on click; read the state with
/// [`Checkbox::checked`] via [`Ui::get`](crate::ui::Ui::get), or react to
/// [`Ui::fired`](crate::ui::Ui::fired) / the [`action`](Checkbox::action) tag.
///
/// **Unlabelled**, it is a bare box: it measures to the box alone and centers
/// it in whatever rect it is arranged into, so a grid of anonymous toggles
/// (a bitmask editor) reads as a grid — while the *whole* cell stays the
/// clickable target, which is what a coarse grid wants.
#[must_use]
pub struct Checkbox {
    id: WidgetId,
    label: String,
    checked: bool,
    disabled: bool,
    action: Option<u64>,
    armed: bool,
    rect: Rect,
}

impl Checkbox {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            id: next_id(),
            label: label.into(),
            checked: false,
            disabled: false,
            action: None,
            armed: false,
            rect: Rect::ZERO,
        }
    }

    /// Attaches an action tag emitted (alongside the toggle) when this fires —
    /// so a host polls one `Ui::actions` channel for a whole panel instead of
    /// reading each box's `checked` back by id.
    pub fn action(mut self, tag: u64) -> Self {
        self.action = Some(tag);
        self
    }

    pub fn id(&self) -> WidgetId {
        self.id
    }

    pub fn checked(&self) -> bool {
        self.checked
    }

    pub fn set_checked(&mut self, checked: bool) {
        self.checked = checked;
    }

    pub fn with_checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Widget for Checkbox {
    fn semantics(&self) -> Semantics<'_> {
        Semantics::labeled(kind_of::<Self>(), &self.label)
    }

    fn measure(&mut self, _avail: Size, ctx: &mut LayoutCtx) -> Size {
        let m = ctx.theme.metrics();
        if self.label.is_empty() {
            return Size::new(BOX, m.control_height);
        }
        let tw = text_width(ctx.fonts, ctx.theme, &self.label, TextRole::Body);
        Size::new(BOX + m.gap + tw, m.control_height)
    }

    fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if !ctx.is_base() {
            return;
        }
        // Labelled: the box leads the caption. Bare: it centers in the cell.
        let bx = if self.label.is_empty() {
            self.rect.x + (self.rect.w - BOX) * 0.5
        } else {
            self.rect.x
        };
        let by = self.rect.y + (self.rect.h - BOX) * 0.5;
        let box_rect = Rect::new(bx, by, BOX, BOX);
        let state = WidgetState {
            hovered: ctx.is_hovered(self.id),
            pressed: self.armed,
            focused: ctx.is_focused(self.id),
            disabled: self.disabled,
            selected: self.checked,
        };
        ctx.theme.well(dl, box_rect, state);
        if self.checked {
            dl.fill_rect(box_rect.inset(Insets::all(4.0)), ctx.theme.accent());
        }
        if !self.label.is_empty() {
            let m = ctx.theme.metrics();
            draw_label(
                dl,
                ctx,
                self.rect,
                self.rect.x + BOX + m.gap,
                &self.label,
                TextRole::Body,
                false,
            );
        }
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        click_machine(
            ev,
            ctx,
            self.id,
            self.rect,
            self.disabled,
            CommitPolicy::ReleaseInside,
            self.action,
            &mut self.armed,
            || {
                self.checked = !self.checked;
            },
        )
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn id(&self) -> WidgetId {
        self.id
    }
}

/// A labelled radio button. Selecting it sets it on (and fires); the host
/// clears the other members of the group.
#[must_use]
pub struct Radio {
    id: WidgetId,
    label: String,
    selected: bool,
    disabled: bool,
    armed: bool,
    rect: Rect,
}

impl Radio {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            id: next_id(),
            label: label.into(),
            selected: false,
            disabled: false,
            armed: false,
            rect: Rect::ZERO,
        }
    }

    pub fn id(&self) -> WidgetId {
        self.id
    }

    pub fn selected(&self) -> bool {
        self.selected
    }

    pub fn set_selected(&mut self, selected: bool) {
        self.selected = selected;
    }

    pub fn with_selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Widget for Radio {
    fn semantics(&self) -> Semantics<'_> {
        Semantics::labeled(kind_of::<Self>(), &self.label)
    }

    fn measure(&mut self, _avail: Size, ctx: &mut LayoutCtx) -> Size {
        let m = ctx.theme.metrics();
        let tw = text_width(ctx.fonts, ctx.theme, &self.label, TextRole::Body);
        Size::new(BOX + m.gap + tw, m.control_height)
    }

    fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if !ctx.is_base() {
            return;
        }
        let by = self.rect.y + (self.rect.h - BOX) * 0.5;
        let box_rect = Rect::new(self.rect.x, by, BOX, BOX);
        let state = WidgetState {
            hovered: ctx.is_hovered(self.id),
            pressed: self.armed,
            focused: ctx.is_focused(self.id),
            disabled: self.disabled,
            selected: self.selected,
        };
        ctx.theme.well(dl, box_rect, state);
        if self.selected {
            // A smaller inset dot distinguishes the radio from the checkbox.
            dl.fill_rect(box_rect.inset(Insets::all(5.0)), ctx.theme.accent());
        }
        let m = ctx.theme.metrics();
        draw_label(
            dl,
            ctx,
            self.rect,
            self.rect.x + BOX + m.gap,
            &self.label,
            TextRole::Body,
            false,
        );
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        // Radios select on click and never deselect themselves; the host clears
        // the rest of the group when this one fires.
        click_machine(
            ev,
            ctx,
            self.id,
            self.rect,
            self.disabled,
            CommitPolicy::ReleaseInside,
            None,
            &mut self.armed,
            || {
                self.selected = true;
            },
        )
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn id(&self) -> WidgetId {
        self.id
    }
}

/// A toggle button: a button that latches on/off and shows the on state.
#[must_use]
pub struct Toggle {
    id: WidgetId,
    label: String,
    on: bool,
    role: Role,
    disabled: bool,
    armed: bool,
    rect: Rect,
}

impl Toggle {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            id: next_id(),
            label: label.into(),
            on: false,
            role: Role::Neutral,
            disabled: false,
            armed: false,
            rect: Rect::ZERO,
        }
    }

    pub fn id(&self) -> WidgetId {
        self.id
    }

    pub fn on(&self) -> bool {
        self.on
    }

    pub fn set_on(&mut self, on: bool) {
        self.on = on;
    }

    pub fn with_on(mut self, on: bool) -> Self {
        self.on = on;
        self
    }

    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Widget for Toggle {
    fn semantics(&self) -> Semantics<'_> {
        Semantics::labeled(kind_of::<Self>(), &self.label)
    }

    fn measure(&mut self, _avail: Size, ctx: &mut LayoutCtx) -> Size {
        let m = ctx.theme.metrics();
        let tw = text_width(ctx.fonts, ctx.theme, &self.label, TextRole::Body);
        Size::new((tw + 2.0 * m.pad).max(m.button_min_width), m.control_height)
    }

    fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if !ctx.is_base() {
            return;
        }
        let state = WidgetState {
            hovered: ctx.is_hovered(self.id),
            pressed: self.armed,
            focused: ctx.is_focused(self.id),
            disabled: self.disabled,
            selected: self.on,
        };
        ctx.theme.button(dl, self.rect, self.role, state);
        let tw = text_width(ctx.fonts, ctx.theme, &self.label, TextRole::Body);
        // A toggle is a raised (button) face: full emboss.
        let px = ctx.theme.font_px(TextRole::Body);
        let baseline = Vec2::new(
            self.rect.center().x - tw * 0.5,
            self.rect.center().y + px * 0.34,
        );
        ctx.theme.text_em(
            dl,
            ctx.fonts,
            baseline,
            &self.label,
            TextRole::Body,
            Emboss::Raised,
        );
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        click_machine(
            ev,
            ctx,
            self.id,
            self.rect,
            self.disabled,
            CommitPolicy::ReleaseInside,
            None,
            &mut self.armed,
            || {
                self.on = !self.on;
            },
        )
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn id(&self) -> WidgetId {
        self.id
    }
}

/// Width of the slider thumb, in logical px.
const THUMB_W: f32 = 10.0;

/// Right-side column (logical px) reserved for a slider's value readout.
const READOUT_W: f32 = 40.0;

/// One end of a [`Slider`] drag — the edges a host needs to bracket a whole
/// gesture instead of treating each value change as its own edit (open an undo
/// stroke on `Begin`, close it on `End`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragPhase {
    /// The press landed: the drag owns the pointer from here.
    Begin,
    /// The drag ended — the button came up, or the window lost focus so the
    /// release will never arrive. Always paired with an earlier `Begin`.
    End,
}

/// A horizontal slider over `[min, max]`. Drag the thumb or click the track;
/// read [`Slider::value`]. Optionally snaps to a [`step`](Slider::step) and
/// shows a numeric [`readout`](Slider::readout).
#[must_use]
pub struct Slider {
    id: WidgetId,
    min: f32,
    max: f32,
    value: f32,
    /// Snap increment; `0` is continuous.
    step: f32,
    /// Decimal places for the inline value readout; `None` hides it.
    readout: Option<usize>,
    disabled: bool,
    dragging: bool,
    /// Drag edges recorded since the last poll, oldest first — see
    /// [`take_drag`](Slider::take_drag).
    drags: Vec<DragPhase>,
    rect: Rect,
}

impl Slider {
    pub fn new(min: f32, max: f32, value: f32) -> Self {
        let mut s = Self {
            id: next_id(),
            min,
            max,
            value: 0.0,
            step: 0.0,
            readout: None,
            disabled: false,
            dragging: false,
            drags: Vec::new(),
            rect: Rect::ZERO,
        };
        s.value = s.quantize(value);
        s
    }

    pub fn id(&self) -> WidgetId {
        self.id
    }

    /// True while a drag is in flight (the pointer is captured).
    pub fn dragging(&self) -> bool {
        self.dragging
    }

    /// Pops the oldest un-reported drag edge — the polled channel that lets one
    /// gesture be one host-side transaction (the [`Select::take_pick`] /
    /// [`TextInput::take_commit`] precedent).
    ///
    /// **Poll in a loop**, not once:
    /// ```ignore
    /// while let Some(phase) = slider.take_drag() {
    ///     match phase {
    ///         DragPhase::Begin => undo.open_stroke(),
    ///         DragPhase::End => undo.close_stroke(),
    ///     }
    /// }
    /// ```
    /// A press and its release can land in the *same* dispatch batch (a fast
    /// click), so both edges are queued; polling once would report the `End` of
    /// a stroke the host never opened.
    ///
    /// [`Select::take_pick`]: crate::Select::take_pick
    /// [`TextInput::take_commit`]: crate::TextInput::take_commit
    pub fn take_drag(&mut self) -> Option<DragPhase> {
        if self.drags.is_empty() {
            None
        } else {
            Some(self.drags.remove(0))
        }
    }

    pub fn value(&self) -> f32 {
        self.value
    }

    pub fn set_value(&mut self, value: f32) {
        self.value = self.quantize(value);
    }

    /// Snaps the value to multiples of `step` (relative to `min`). `0` (default)
    /// is continuous.
    pub fn step(mut self, step: f32) -> Self {
        self.step = step.max(0.0);
        self.value = self.quantize(self.value);
        self
    }

    /// Shows the current value as inline text with `decimals` places at the right
    /// end of the slider (the track shrinks to make room).
    pub fn readout(mut self, decimals: usize) -> Self {
        self.readout = Some(decimals);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Clamps to range and snaps to `step` (if any).
    fn quantize(&self, v: f32) -> f32 {
        let v = v.clamp(self.min, self.max);
        if self.step > 0.0 {
            let snapped = self.min + ((v - self.min) / self.step).round() * self.step;
            snapped.clamp(self.min, self.max)
        } else {
            v
        }
    }

    fn fraction(&self) -> f32 {
        if self.max > self.min {
            (self.value - self.min) / (self.max - self.min)
        } else {
            0.0
        }
    }

    /// The track width (the slider width less any readout column).
    fn track_w(&self) -> f32 {
        let readout = if self.readout.is_some() {
            READOUT_W
        } else {
            0.0
        };
        (self.rect.w - readout).max(1.0)
    }

    fn set_from_x(&mut self, x: f32) {
        let usable = (self.track_w() - THUMB_W).max(1.0);
        let t = ((x - self.rect.x - THUMB_W * 0.5) / usable).clamp(0.0, 1.0);
        self.value = self.quantize(self.min + t * (self.max - self.min));
    }
}

impl Widget for Slider {
    fn measure(&mut self, _avail: Size, ctx: &mut LayoutCtx) -> Size {
        let m = ctx.theme.metrics();
        let readout = if self.readout.is_some() {
            READOUT_W
        } else {
            0.0
        };
        Size::new(m.button_min_width + readout, m.control_height)
    }

    fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if !ctx.is_base() {
            return;
        }
        let track_h = 6.0;
        let ty = self.rect.y + (self.rect.h - track_h) * 0.5;
        let track_w = self.track_w();
        let track = Rect::new(self.rect.x, ty, track_w, track_h);
        let bevel = ctx.theme.metrics().bevel;
        ctx.theme.well(
            dl,
            track,
            WidgetState {
                focused: ctx.is_focused(self.id),
                disabled: self.disabled,
                ..Default::default()
            },
        );
        let thumb_x = self.rect.x + self.fraction() * (track_w - THUMB_W);
        // Accent fill from the track start to the thumb.
        let fill_w = (thumb_x + THUMB_W * 0.5 - track.x - bevel).max(0.0);
        dl.fill_rect(
            Rect::new(
                track.x + bevel,
                track.y + bevel,
                fill_w,
                track_h - 2.0 * bevel,
            ),
            ctx.theme.accent(),
        );
        let thumb = Rect::new(thumb_x, self.rect.y, THUMB_W, self.rect.h);
        ctx.theme.button(
            dl,
            thumb,
            Role::Neutral,
            WidgetState {
                hovered: ctx.is_hovered(self.id),
                pressed: self.dragging,
                focused: ctx.is_focused(self.id),
                disabled: self.disabled,
                selected: false,
            },
        );
        if let Some(decimals) = self.readout {
            let text = format!("{:.*}", decimals, self.value);
            let px = ctx.theme.font_px(TextRole::Small);
            let tw = text_width(ctx.fonts, ctx.theme, &text, TextRole::Small);
            // Right-aligned within the reserved readout column.
            let x = self.rect.right() - tw;
            let baseline = Vec2::new(x, self.rect.center().y + px * 0.34);
            ctx.theme
                .text_muted(dl, ctx.fonts, baseline, &text, TextRole::Small);
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
                ..
            } if ctx.is_target(self.id) => {
                ctx.consume_pointer();
                ctx.request_focus(self.id);
                self.dragging = true;
                self.drags.push(DragPhase::Begin);
                self.set_from_x(ctx.pointer.x);
                ctx.capture(self.id);
                ctx.fire(self.id, None);
                true
            }
            Event::PointerMoved { .. } if self.dragging && ctx.is_target(self.id) => {
                self.set_from_x(ctx.pointer.x);
                ctx.fire(self.id, None);
                ctx.consume_pointer();
                true
            }
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: false,
                ..
            } if self.dragging => {
                self.dragging = false;
                self.drags.push(DragPhase::End);
                ctx.consume_pointer();
                true
            }
            // Window focus loss: the drag's release will never arrive, so end it
            // here or the host's stroke stays open forever (`Scroller` does the
            // same for its thumb).
            Event::Focus(false) if self.dragging => {
                self.dragging = false;
                self.drags.push(DragPhase::End);
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
}

/// A non-interactive progress bar; `fraction` is clamped to `0.0..=1.0`. Give it
/// an id with [`ProgressBar::with_id`] to drive it from the host each frame.
#[must_use]
pub struct ProgressBar {
    id: WidgetId,
    fraction: f32,
    /// `true` overlays the rounded percentage; a custom `label` takes priority.
    percent: bool,
    label: Option<String>,
    rect: Rect,
}

impl ProgressBar {
    pub fn new(fraction: f32) -> Self {
        Self {
            id: WidgetId::NONE,
            fraction: fraction.clamp(0.0, 1.0),
            percent: false,
            label: None,
            rect: Rect::ZERO,
        }
    }

    /// Assigns a stable id so the host can update the fraction via
    /// [`Ui::get_mut`](crate::ui::Ui::get_mut). Keep the returned [`ProgressBar::id`].
    pub fn with_id(mut self) -> Self {
        self.id = next_id();
        self
    }

    /// Overlays the rounded percentage (e.g. `60%`) centered on the bar. The bar
    /// grows to a full control height to fit the text. A custom
    /// [`label`](ProgressBar::label) overrides this.
    pub fn percent(mut self) -> Self {
        self.percent = true;
        self
    }

    /// Overlays custom centered text (e.g. `Generating… 60%`), updatable per
    /// frame via [`set_label`](ProgressBar::set_label). The bar grows to a full
    /// control height to fit it.
    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.label = Some(text.into());
        self
    }

    pub fn id(&self) -> WidgetId {
        self.id
    }

    pub fn fraction(&self) -> f32 {
        self.fraction
    }

    pub fn set_fraction(&mut self, fraction: f32) {
        self.fraction = fraction.clamp(0.0, 1.0);
    }

    /// Replaces the overlaid label text (no-op styling if the bar shows percent
    /// or nothing). Pass an empty string to clear back to no custom label.
    pub fn set_label(&mut self, text: impl Into<String>) {
        let t = text.into();
        self.label = if t.is_empty() { None } else { Some(t) };
    }

    /// The text overlaid this frame, if any (custom label, else percent).
    fn overlay(&self) -> Option<String> {
        match (&self.label, self.percent) {
            (Some(t), _) => Some(t.clone()),
            (None, true) => Some(format!("{}%", (self.fraction * 100.0).round() as i32)),
            (None, false) => None,
        }
    }

    fn labeled(&self) -> bool {
        self.label.is_some() || self.percent
    }
}

impl Widget for ProgressBar {
    fn semantics(&self) -> Semantics<'_> {
        Semantics {
            kind: kind_of::<Self>(),
            label: self.label.as_deref(),
        }
    }

    fn measure(&mut self, _avail: Size, ctx: &mut LayoutCtx) -> Size {
        let m = ctx.theme.metrics();
        // A labeled bar needs room for the text; a bare bar is a slim track.
        let h = if self.labeled() {
            m.control_height
        } else {
            (m.control_height * 0.55).round()
        };
        Size::new(m.button_min_width, h)
    }

    fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if !ctx.is_base() {
            return;
        }
        ctx.theme.well(dl, self.rect, WidgetState::default());
        let bevel = ctx.theme.metrics().bevel;
        let inner = self.rect.inset(Insets::all(bevel));
        dl.fill_rect(
            Rect::new(inner.x, inner.y, inner.w * self.fraction, inner.h),
            ctx.theme.accent(),
        );
        if let Some(text) = self.overlay() {
            let px = ctx.theme.font_px(TextRole::Small);
            let tw = text_width(ctx.fonts, ctx.theme, &text, TextRole::Small);
            let c = self.rect.center();
            let baseline = Vec2::new(c.x - tw * 0.5, c.y + px * 0.34);
            ctx.theme
                .text(dl, ctx.fonts, baseline, &text, TextRole::Small);
        }
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn id(&self) -> WidgetId {
        self.id
    }

    /// Decorative: never a pointer target even when it carries an id.
    fn hit_test(&self, _pos: Vec2) -> Option<WidgetId> {
        None
    }
}

/// A static image: paints a host-registered texture ([`TextureId`]) as content
/// (an icon, a sprite) or a background. Register the texture once via
/// [`UiRenderer::register_texture`](crate::gpu::UiRenderer::register_texture) and
/// keep the returned id. Non-interactive — pair it with a [`Button`] or place it
/// behind other widgets in a [`Stack`](crate::layout::Stack) for a clickable or
/// decorated image.
#[must_use]
pub struct Image {
    id: WidgetId,
    tex: TextureId,
    /// Desired logical size for `measure`; `Size::ZERO` means "fill the arranged
    /// rect" (use with `Length::Flex`, `Fill`, or `CrossAlign::Stretch`).
    size: Size,
    uv: TexRect,
    tint: Rgba,
    rect: Rect,
}

impl Image {
    /// An image that fills whatever rectangle it is arranged into.
    pub fn new(tex: TextureId) -> Self {
        Self {
            id: WidgetId::NONE,
            tex,
            size: Size::ZERO,
            uv: TexRect::FULL,
            tint: Rgba::WHITE,
            rect: Rect::ZERO,
        }
    }

    /// Assigns a stable id so the host can swap the texture/uv via
    /// [`Ui::get_mut`](crate::ui::Ui::get_mut) (live preview strips). The image
    /// stays decorative (never a pointer target). Keep the returned
    /// [`Image::id`].
    pub fn with_id(mut self) -> Self {
        self.id = next_id();
        self
    }

    pub fn id(&self) -> WidgetId {
        self.id
    }

    /// An image that measures to a fixed logical `w`x`h` (an icon, a swatch).
    pub fn sized(tex: TextureId, w: f32, h: f32) -> Self {
        Self {
            size: Size::new(w, h),
            ..Self::new(tex)
        }
    }

    /// Modulates the image by `tint` (default opaque white = untouched). Useful
    /// for coverage/mask textures or tinted icons.
    pub fn tint(mut self, tint: Rgba) -> Self {
        self.tint = tint;
        self
    }

    /// Selects a sub-region of the texture (e.g. one cell of a sprite sheet).
    pub fn uv(mut self, uv: TexRect) -> Self {
        self.uv = uv;
        self
    }

    /// Swaps the texture drawn (the id stays valid across frames).
    pub fn set_tex(&mut self, tex: TextureId) {
        self.tex = tex;
    }

    /// Swaps the sub-region sampled (sprite-sheet paging).
    pub fn set_uv(&mut self, uv: TexRect) {
        self.uv = uv;
    }
}

impl Widget for Image {
    fn measure(&mut self, _avail: Size, _ctx: &mut LayoutCtx) -> Size {
        self.size
    }

    fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if !ctx.is_base() {
            return;
        }
        dl.image(self.tex, self.rect, self.uv, self.tint);
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn id(&self) -> WidgetId {
        self.id
    }

    /// Decorative even when id'd: `with_id` exists for `set_tex`/`set_uv`
    /// re-syncs, and the doc there promises the image never becomes a pointer
    /// target — which the default (answer "me" for the whole rect) would break
    /// the moment an id is attached. `Label` and `ProgressBar` make the same
    /// promise the same way.
    fn hit_test(&self, _pos: Vec2) -> Option<WidgetId> {
        None
    }
}

/// A clickable button whose face shows an image — an icon, a tool glyph, a tile
/// swatch — instead of (or as well as) a text label. Behaves like
/// [`Button`] (hover → arm-on-press → fire-on-release-inside) and is polled the
/// same way, via [`Ui::fired`](crate::ui::Ui::fired) with [`ImageButton::id`].
/// The image is drawn inset within the themed button face, so the face's hover /
/// pressed / [`selected`](ImageButton::selected) states still read around it —
/// e.g. a tool palette where the active tool latches on.
#[must_use]
pub struct ImageButton {
    id: WidgetId,
    tex: TextureId,
    uv: TexRect,
    size: Size,
    tint: Rgba,
    /// Padding (logical px) between the button face edge and the image.
    inset: f32,
    role: Role,
    disabled: bool,
    selected: bool,
    action: Option<u64>,
    commit: CommitPolicy,
    focusable: bool,
    armed: bool,
    /// A hover tooltip — see [`Button::tooltip`].
    tooltip: Option<String>,
    rect: Rect,
}

impl ImageButton {
    /// A `w`×`h` (logical px) button showing `tex`.
    pub fn new(tex: TextureId, w: f32, h: f32) -> Self {
        Self {
            id: next_id(),
            tex,
            uv: TexRect::FULL,
            size: Size::new(w, h),
            tint: Rgba::WHITE,
            inset: 3.0,
            role: Role::Neutral,
            disabled: false,
            selected: false,
            action: None,
            commit: CommitPolicy::ReleaseInside,
            focusable: false,
            armed: false,
            tooltip: None,
            rect: Rect::ZERO,
        }
    }

    /// A hover tooltip painted by the overlay pass once the pointer has rested
    /// here — see [`Button::tooltip`]. The standard caption for a picture key.
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    /// The id to poll for clicks (keep this; the widget moves into the tree).
    pub fn id(&self) -> WidgetId {
        self.id
    }

    /// Selects a sub-region of the texture (one cell of a sprite sheet).
    pub fn uv(mut self, uv: TexRect) -> Self {
        self.uv = uv;
        self
    }

    /// Modulates the image by `tint` (default opaque white = untouched).
    pub fn tint(mut self, tint: Rgba) -> Self {
        self.tint = tint;
        self
    }

    /// Padding between the button face and the image (default `3.0`).
    pub fn inset(mut self, inset: f32) -> Self {
        self.inset = inset;
        self
    }

    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Latches the button on (the active tool / selected swatch) so the theme
    /// paints its `selected` face.
    pub fn with_selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn set_selected(&mut self, selected: bool) {
        self.selected = selected;
    }

    /// Whether the button currently reads as active — see
    /// [`Button::selected`].
    pub fn selected(&self) -> bool {
        self.selected
    }

    /// Swaps the image drawn (the id stays valid across frames).
    pub fn set_tex(&mut self, tex: TextureId) {
        self.tex = tex;
    }

    /// Attaches an action tag emitted (alongside the click) when this fires.
    pub fn action(mut self, tag: u64) -> Self {
        self.action = Some(tag);
        self
    }

    /// Fire on press instead of release-inside — an immediate selection (a
    /// tool/tile pick) rather than a confirmable command.
    pub fn commit(mut self, commit: CommitPolicy) -> Self {
        self.commit = commit;
        self
    }

    /// Opts into Tab focus traversal; a focused button activates on Enter/Space.
    pub fn focusable(mut self) -> Self {
        self.focusable = true;
        self
    }
}

impl Widget for ImageButton {
    fn measure(&mut self, _avail: Size, _ctx: &mut LayoutCtx) -> Size {
        self.size
    }

    fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if !ctx.is_base() {
            return;
        }
        let state = WidgetState {
            hovered: ctx.is_hovered(self.id),
            pressed: self.armed,
            focused: ctx.is_focused(self.id),
            disabled: self.disabled,
            selected: self.selected,
        };
        ctx.theme.button(dl, self.rect, self.role, state);
        dl.image(
            self.tex,
            self.rect.inset(Insets::all(self.inset)),
            self.uv,
            self.tint,
        );
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        click_machine(
            ev,
            ctx,
            self.id,
            self.rect,
            self.disabled,
            self.commit,
            self.action,
            &mut self.armed,
            || {},
        )
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn id(&self) -> WidgetId {
        self.id
    }

    fn accepts_focus(&self) -> bool {
        self.focusable && !self.disabled
    }

    fn tooltip(&self) -> Option<&str> {
        self.tooltip.as_deref()
    }
}

/// A clickable solid-color swatch — a palette cell, a "current color" chip, a
/// color choice in a picker. The swatch color is the button's *content* and is
/// always drawn true (a swatch must show its real color, so state never washes
/// over it); the themed face around it carries hover / pressed /
/// [`selected`](ColorButton::selected) / focus, exactly as [`ImageButton`]
/// frames an image. A `selected` swatch also gains an accent ring so the choice
/// reads at a glance in a dense grid. Behaves like [`Button`] (arm-on-press →
/// fire-on-release-inside); poll it via [`Ui::fired`](crate::ui::Ui::fired) with
/// [`ColorButton::id`].
///
/// With a [`label`](ColorButton::label) it becomes a **color key**: a command
/// button whose face happens to be a color (a team swatch, a pass-type key)
/// rather than an anonymous cell. That is the only difference between the two —
/// a labelled `ColorButton` is a [`Button`] whose face is the color it means.
#[must_use]
pub struct ColorButton {
    id: WidgetId,
    color: Rgba,
    /// An optional caption over the swatch — see [`label`](Self::label).
    label: String,
    size: Size,
    /// Padding (logical px) between the button face edge and the color fill.
    inset: f32,
    role: Role,
    disabled: bool,
    selected: bool,
    action: Option<u64>,
    commit: CommitPolicy,
    focusable: bool,
    armed: bool,
    /// A hover tooltip — see [`Button::tooltip`].
    tooltip: Option<String>,
    rect: Rect,
}

impl ColorButton {
    /// A `w`×`h` (logical px) swatch of `color`.
    pub fn new(color: Rgba, w: f32, h: f32) -> Self {
        Self {
            id: next_id(),
            color,
            label: String::new(),
            size: Size::new(w, h),
            inset: 2.0,
            role: Role::Neutral,
            disabled: false,
            selected: false,
            action: None,
            commit: CommitPolicy::ReleaseInside,
            focusable: false,
            armed: false,
            tooltip: None,
            rect: Rect::ZERO,
        }
    }

    /// A hover tooltip painted by the overlay pass once the pointer has rested
    /// here — see [`Button::tooltip`]. The standard caption for an uncaptioned
    /// square swatch, whose face is a color and nothing else.
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    /// The id to poll for clicks (keep this; the widget moves into the tree).
    pub fn id(&self) -> WidgetId {
        self.id
    }

    /// Padding between the button face and the color fill (default `2.0`).
    pub fn inset(mut self, inset: f32) -> Self {
        self.inset = inset;
        self
    }

    /// Captions the swatch, turning it into a **color key** — a command button
    /// whose face is the color it means (a team swatch labelled `red`, a
    /// pass-type key). Drawn centered over the fill at
    /// [`TextRole::Small`].
    ///
    /// The ink is chosen from the swatch's own luminance, not from the theme:
    /// a caption sits on an arbitrary host-supplied color, so a fixed chrome
    /// ink would vanish on half the palette. Empty (the default) draws nothing
    /// and leaves the swatch a pure cell.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Replaces the caption in place (the id stays valid across frames).
    pub fn set_label(&mut self, label: impl Into<String>) {
        self.label = label.into();
    }

    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Latches the swatch on (the chosen color) so the theme paints its
    /// `selected` face and the widget adds an accent ring.
    pub fn with_selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn set_selected(&mut self, selected: bool) {
        self.selected = selected;
    }

    /// Whether the swatch currently reads as chosen — see [`Button::selected`].
    pub fn selected(&self) -> bool {
        self.selected
    }

    /// The color drawn (for hosts that key choices by color).
    pub fn color(&self) -> Rgba {
        self.color
    }

    /// Swaps the swatch color (the id stays valid across frames).
    pub fn set_color(&mut self, color: Rgba) {
        self.color = color;
    }

    /// Attaches an action tag emitted (alongside the click) when this fires.
    pub fn action(mut self, tag: u64) -> Self {
        self.action = Some(tag);
        self
    }

    /// Fire on press instead of release-inside — an immediate selection (the
    /// palette-swatch gesture) rather than a confirmable command.
    pub fn commit(mut self, commit: CommitPolicy) -> Self {
        self.commit = commit;
        self
    }

    /// Opts into Tab focus traversal; a focused swatch activates on Enter/Space.
    pub fn focusable(mut self) -> Self {
        self.focusable = true;
        self
    }
}

impl Widget for ColorButton {
    fn semantics(&self) -> Semantics<'_> {
        Semantics::labeled(kind_of::<Self>(), &self.label)
    }

    fn measure(&mut self, _avail: Size, _ctx: &mut LayoutCtx) -> Size {
        self.size
    }

    fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if !ctx.is_base() {
            return;
        }
        let state = WidgetState {
            hovered: ctx.is_hovered(self.id),
            pressed: self.armed,
            focused: ctx.is_focused(self.id),
            disabled: self.disabled,
            selected: self.selected,
        };
        // The themed face carries the state (bevel, hover/press/focus); the color
        // is painted true over it, inset so the face's frame still reads around it.
        ctx.theme.button(dl, self.rect, self.role, state);
        let fill = self.rect.inset(Insets::all(self.inset));
        dl.fill_rect(fill, self.color);
        // A caption reads over the swatch in whichever of black/white contrasts
        // with it — the fill is host data, so no chrome ink is safe for all of it.
        // Clipped to the fill, so a caption wider than its swatch stops at the
        // swatch edge rather than running over the key beside it.
        if !self.label.is_empty() {
            let px = ctx.theme.font_px(TextRole::Small);
            let tw = text_width(ctx.fonts, ctx.theme, &self.label, TextRole::Small);
            let c = fill.center();
            let baseline = Vec2::new(c.x - tw * 0.5, c.y + px * 0.34);
            dl.push_clip(fill);
            ctx.theme.text_colored(
                dl,
                ctx.fonts,
                baseline,
                &self.label,
                TextRole::Small,
                Emboss::Raised,
                contrast_ink(self.color),
            );
            dl.pop_clip();
        }
        // A selected swatch is marked by an accent ring (the color stays pure —
        // no wash over the content), on the fill edge so it never spills outside.
        if self.selected {
            dl.stroke_rect(fill, ctx.theme.metrics().bevel.max(1.0), ctx.theme.accent());
        }
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        click_machine(
            ev,
            ctx,
            self.id,
            self.rect,
            self.disabled,
            self.commit,
            self.action,
            &mut self.armed,
            || {},
        )
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn id(&self) -> WidgetId {
        self.id
    }

    fn accepts_focus(&self) -> bool {
        self.focusable && !self.disabled
    }

    fn tooltip(&self) -> Option<&str> {
        self.tooltip.as_deref()
    }
}

/// An inset well wrapping a single child — the framed, sunken background for a
/// text field, list, or custom content. Draws [`Theme::well`](crate::theme::Theme::well)
/// in the base pass; the child is inset by `padding`. A first-class version of
/// the well a host previously had to hand-build.
#[must_use]
pub struct Well {
    child: Box<dyn Widget>,
    padding: Insets,
    rect: Rect,
}

impl Well {
    pub fn new(child: impl Widget + 'static) -> Self {
        Self {
            child: Box::new(child),
            padding: Insets::all(4.0),
            rect: Rect::ZERO,
        }
    }

    pub fn padding(mut self, padding: Insets) -> Self {
        self.padding = padding;
        self
    }
}

impl Widget for Well {
    fn measure(&mut self, avail: Size, ctx: &mut LayoutCtx) -> Size {
        let inner = Size::new(
            (avail.w - self.padding.horizontal()).max(0.0),
            (avail.h - self.padding.vertical()).max(0.0),
        );
        let c = self.child.measure(inner, ctx);
        Size::new(
            c.w + self.padding.horizontal(),
            c.h + self.padding.vertical(),
        )
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        self.child.arrange(rect.inset(self.padding), ctx);
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        // Background is chrome — base pass only (see `Panel::draw`).
        if ctx.is_base() {
            ctx.theme.well(dl, self.rect, WidgetState::default());
        }
        self.child.draw(dl, ctx);
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        self.child.event(ev, ctx)
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn child_count(&self) -> usize {
        1
    }

    fn child(&self, i: usize) -> Option<&dyn Widget> {
        (i == 0).then_some(self.child.as_ref())
    }

    fn child_mut(&mut self, i: usize) -> Option<&mut dyn Widget> {
        (i == 0).then_some(self.child.as_mut())
    }

    fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
        if self.rect.contains(pos) {
            self.child.hit_test(pos)
        } else {
            None
        }
    }
}

/// A rule between sections — the theme's [`separator`](crate::theme::Theme::separator)
/// (or [`vseparator`](crate::theme::Theme::vseparator)) drawn across the middle
/// of its arranged rect, so a host stops hand-filling a 1px strip in whichever
/// ink it guessed the chrome used. Give it a [`Length::Fixed`](crate::widget::Length)
/// slot of the thickness the rule should sit in; the theme decides what a rule
/// *is* (the steel skin engraves a groove rather than painting a line).
#[must_use]
pub struct Separator {
    vertical: bool,
    rect: Rect,
}

/// The slot a rule asks for when nothing else sizes it — the theme's groove
/// plus a hair of margin.
const SEPARATOR_THICKNESS: f32 = 2.0;

impl Separator {
    /// A horizontal rule (the usual one: a line between stacked sections).
    pub fn new() -> Self {
        Self {
            vertical: false,
            rect: Rect::ZERO,
        }
    }

    /// A vertical rule, for a column divider in a row.
    pub fn vertical() -> Self {
        Self {
            vertical: true,
            rect: Rect::ZERO,
        }
    }
}

impl Default for Separator {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Separator {
    fn measure(&mut self, avail: Size, _ctx: &mut LayoutCtx) -> Size {
        if self.vertical {
            Size::new(SEPARATOR_THICKNESS, avail.h)
        } else {
            Size::new(avail.w, SEPARATOR_THICKNESS)
        }
    }

    fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if !ctx.is_base() {
            return;
        }
        if self.vertical {
            ctx.theme.vseparator(dl, self.rect);
        } else {
            ctx.theme.separator(dl, self.rect);
        }
    }

    fn rect(&self) -> Rect {
        self.rect
    }
}

/// A vertical group of mutually-exclusive [`Radio`]s. Owns the exclusivity that
/// a host otherwise enforces by hand: clicking one option selects it and clears
/// the rest. Read the choice with [`selected`](RadioGroup::selected); the
/// per-option ids stay pollable via [`id`](RadioGroup::id) if you want the
/// fired-event instead.
#[must_use]
pub struct RadioGroup {
    id: WidgetId,
    radios: Vec<Radio>,
    spacing: f32,
    selected: Option<usize>,
    rect: Rect,
}

impl RadioGroup {
    pub fn new() -> Self {
        Self {
            id: next_id(),
            radios: Vec::new(),
            spacing: 0.0,
            selected: None,
            rect: Rect::ZERO,
        }
    }

    /// The group's own id — read the choice back with
    /// `ui.get::<RadioGroup>(id).selected()`.
    pub fn id(&self) -> WidgetId {
        self.id
    }

    /// Appends an option.
    pub fn option(mut self, label: impl Into<String>) -> Self {
        self.radios.push(Radio::new(label));
        self
    }

    /// Vertical gap between options.
    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    /// Pre-selects option `i`.
    pub fn with_selected(mut self, i: usize) -> Self {
        self.select(i);
        self
    }

    /// The selected option index, if any.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// The number of options.
    pub fn len(&self) -> usize {
        self.radios.len()
    }

    pub fn is_empty(&self) -> bool {
        self.radios.is_empty()
    }

    /// The widget id of option `i` (to poll [`Ui::fired`](crate::ui::Ui::fired)).
    pub fn option_id(&self, i: usize) -> Option<WidgetId> {
        self.radios.get(i).map(|r| r.id())
    }

    /// Selects option `i`, clearing the rest. No-op if `i` is out of range.
    pub fn select(&mut self, i: usize) {
        if i >= self.radios.len() {
            return;
        }
        for (j, r) in self.radios.iter_mut().enumerate() {
            r.set_selected(j == i);
        }
        self.selected = Some(i);
    }
}

impl Default for RadioGroup {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for RadioGroup {
    fn measure(&mut self, avail: Size, ctx: &mut LayoutCtx) -> Size {
        let n = self.radios.len();
        let mut w = 0.0f32;
        let mut h = 0.0f32;
        for (i, r) in self.radios.iter_mut().enumerate() {
            let s = r.measure(avail, ctx);
            w = w.max(s.w);
            h += s.h;
            if i + 1 < n {
                h += self.spacing;
            }
        }
        Size::new(w, h)
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        let mut y = rect.y;
        for r in &mut self.radios {
            let h = r.measure(rect.size(), ctx).h;
            r.arrange(Rect::new(rect.x, y, rect.w, h), ctx);
            y += h + self.spacing;
        }
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        for r in &self.radios {
            r.draw(dl, ctx);
        }
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        let mut handled = false;
        for r in &mut self.radios {
            handled |= r.event(ev, ctx);
        }
        // A radio selects itself on click; adopt the newly-selected one and
        // clear the rest so exactly one option is ever on.
        let newly = self
            .radios
            .iter()
            .enumerate()
            .find(|&(i, r)| r.selected() && Some(i) != self.selected)
            .map(|(i, _)| i);
        if let Some(i) = newly {
            self.select(i);
        }
        handled
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn id(&self) -> WidgetId {
        self.id
    }

    fn child_count(&self) -> usize {
        self.radios.len()
    }

    fn child(&self, i: usize) -> Option<&dyn Widget> {
        self.radios.get(i).map(|r| r as &dyn Widget)
    }

    fn child_mut(&mut self, i: usize) -> Option<&mut dyn Widget> {
        self.radios.get_mut(i).map(|r| r as &mut dyn Widget)
    }

    fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
        if !self.rect.contains(pos) {
            return None;
        }
        self.radios.iter().rev().find_map(|r| r.hit_test(pos))
    }
}

/// Left text inset for list rows, in logical px.
const LIST_PAD_X: f32 = 6.0;

/// A vertical list of selectable text rows with single selection and per-row
/// hover. It reports its full content height and stretches to the available
/// width, so wrap it in a [`ScrollArea`](crate::scroll::ScrollArea) for a
/// scrollable picker. Clicking a row (or arrow-key navigation while focused)
/// selects it and fires; read the choice with [`selected`](List::selected) via
/// [`Ui::get`](crate::ui::Ui::get) or react to [`Ui::fired`](crate::ui::Ui::fired).
#[must_use]
pub struct List {
    id: WidgetId,
    items: Vec<String>,
    selected: Option<usize>,
    hover_row: Option<usize>,
    row_h: f32,
    rect: Rect,
}

impl List {
    pub fn new() -> Self {
        Self {
            id: next_id(),
            items: Vec::new(),
            selected: None,
            hover_row: None,
            row_h: 22.0,
            rect: Rect::ZERO,
        }
    }

    /// Appends a row.
    pub fn item(mut self, text: impl Into<String>) -> Self {
        self.items.push(text.into());
        self
    }

    /// Replaces all rows (clearing the selection if it falls out of range).
    pub fn set_items<I, S>(&mut self, items: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.items = items.into_iter().map(Into::into).collect();
        if self.selected.is_some_and(|i| i >= self.items.len()) {
            self.selected = None;
        }
    }

    /// Pre-selects row `i`.
    pub fn with_selected(mut self, i: usize) -> Self {
        self.select(i);
        self
    }

    pub fn id(&self) -> WidgetId {
        self.id
    }

    /// The selected row index, if any.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// The text of the selected row, if any.
    pub fn selected_text(&self) -> Option<&str> {
        self.selected
            .and_then(|i| self.items.get(i))
            .map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Selects row `i` (no-op if out of range).
    pub fn select(&mut self, i: usize) {
        if i < self.items.len() {
            self.selected = Some(i);
        }
    }

    /// Clears the selection.
    pub fn clear_selection(&mut self) {
        self.selected = None;
    }

    /// The row index at screen `y` (within the list), if any.
    fn row_at(&self, y: f32) -> Option<usize> {
        if self.items.is_empty() || self.row_h <= 0.0 {
            return None;
        }
        let i = ((y - self.rect.y) / self.row_h).floor();
        if i < 0.0 {
            return None;
        }
        let i = i as usize;
        (i < self.items.len()).then_some(i)
    }

    /// Moves the selection by `delta` rows (clamped), firing if it changed.
    fn move_selection(&mut self, delta: isize, ctx: &mut EventCtx) {
        if self.items.is_empty() {
            return;
        }
        let cur = self.selected.unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, self.items.len() as isize - 1) as usize;
        if Some(next) != self.selected {
            self.selected = Some(next);
            ctx.fire(self.id, None);
        }
    }
}

impl Default for List {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for List {
    fn measure(&mut self, avail: Size, ctx: &mut LayoutCtx) -> Size {
        self.row_h = ctx.theme.metrics().control_height;
        Size::new(avail.w, self.items.len() as f32 * self.row_h)
    }

    fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        self.row_h = ctx.theme.metrics().control_height;
    }

    fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
        if !ctx.is_base() {
            return;
        }
        let px = ctx.theme.font_px(TextRole::Body);
        for (i, text) in self.items.iter().enumerate() {
            let row = Rect::new(
                self.rect.x,
                self.rect.y + i as f32 * self.row_h,
                self.rect.w,
                self.row_h,
            );
            // The row states go through the theme, at the same floors a
            // `Select`'s option list uses — a material skin owns its highlight
            // as a tinted crop of the well it sits in, and a host filling a rect
            // with a translucent accent could not know that.
            let (is_sel, is_hov) = (self.selected == Some(i), self.hover_row == Some(i));
            if is_sel || is_hov {
                let floor = match (is_sel, is_hov) {
                    (true, true) => ROW_FLOOR_ACTIVE_HOVER,
                    (true, false) => ROW_FLOOR_ACTIVE,
                    _ => ROW_FLOOR_HOVER,
                };
                ctx.theme.accent_well_row(dl, row, floor);
            }
            let baseline = Vec2::new(row.x + LIST_PAD_X, row.center().y + px * 0.34);
            ctx.theme
                .text(dl, ctx.fonts, baseline, text, TextRole::Body);
        }
    }

    fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
        match ev {
            // Track per-row hover (the central hover is a single id, so the list
            // resolves which row itself). Never consumes — hover is passive.
            Event::PointerMoved { .. } => {
                self.hover_row = if self.rect.contains(ctx.pointer) {
                    self.row_at(ctx.pointer.y)
                } else {
                    None
                };
                false
            }
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                ..
            } if ctx.is_target(self.id) => {
                ctx.consume_pointer();
                ctx.request_focus(self.id);
                if let Some(i) = self.row_at(ctx.pointer.y) {
                    self.selected = Some(i);
                    ctx.fire(self.id, None);
                }
                true
            }
            Event::Key {
                key, pressed: true, ..
            } if ctx.is_target(self.id) => {
                match key {
                    Key::Up => self.move_selection(-1, ctx),
                    Key::Down => self.move_selection(1, ctx),
                    Key::Home => self.move_selection(isize::MIN / 2, ctx),
                    Key::End => self.move_selection(isize::MAX / 2, ctx),
                    _ => return false,
                }
                ctx.consume_keyboard();
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
}

/// The shared click state machine for every clickable face — [`Button`],
/// [`ImageButton`], [`ColorButton`], and the toggle-style controls
/// ([`Checkbox`], [`Radio`], [`Toggle`]). One machine ⇒ one behavior:
///
/// - **Pointer**: under [`CommitPolicy::ReleaseInside`] a primary press arms
///   and captures; releasing while still over the face fires, elsewhere
///   cancels. Under [`CommitPolicy::PressFire`] the press fires immediately.
///   A disabled face still swallows its pointer events so the world behind
///   never sees them.
/// - **Keyboard**: when the widget is focused, Enter or Space fires — the one
///   place keyboard (and host-mapped gamepad) activation lives.
/// - **Window focus loss**: a live arm is cancelled (the release will never
///   arrive); the event is left unconsumed so every other armed widget can
///   cancel too.
///
/// `on_fire` runs before the [`fired`](crate::ui::Ui::fired)/`action` record,
/// so a toggle flips its state first. Returns whether the event was consumed.
#[allow(clippy::too_many_arguments)]
fn click_machine(
    ev: &Event,
    ctx: &mut EventCtx,
    id: WidgetId,
    rect: Rect,
    disabled: bool,
    commit: CommitPolicy,
    action: Option<u64>,
    armed: &mut bool,
    mut on_fire: impl FnMut(),
) -> bool {
    match ev {
        Event::PointerButton {
            button: PointerButton::Primary,
            pressed: true,
            ..
        } if ctx.is_target(id) => {
            ctx.consume_pointer();
            if disabled {
                return true;
            }
            ctx.request_focus(id);
            match commit {
                CommitPolicy::PressFire => {
                    on_fire();
                    ctx.fire(id, action);
                }
                CommitPolicy::ReleaseInside => {
                    *armed = true;
                    ctx.capture(id);
                }
            }
            true
        }
        Event::PointerButton {
            button: PointerButton::Primary,
            pressed: false,
            ..
        } if *armed => {
            *armed = false;
            if rect.contains(ctx.pointer) {
                on_fire();
                ctx.fire(id, action);
            }
            ctx.consume_pointer();
            true
        }
        Event::Focus(false) => {
            *armed = false;
            false
        }
        Event::Key {
            key: Key::Enter | Key::Space,
            pressed: true,
            ..
        } if ctx.is_target(id) && !disabled => {
            on_fire();
            ctx.fire(id, action);
            ctx.consume_keyboard();
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::DrawCmd;
    use crate::text::Fonts;
    use crate::theme::{Gunmetal, Theme};
    use crate::ui::Ui;
    use crate::widget::DrawPass;

    const DEJAVU: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");

    /// The standard test theme + font set.
    fn theme_fonts() -> (Gunmetal, Fonts) {
        let mut fonts = Fonts::new();
        let font = fonts.add(DEJAVU.to_vec()).unwrap();
        (Gunmetal::new(font), fonts)
    }

    /// A text-faced key may decouple its semantics name from its painted
    /// face ("B" on the face, "Bold" to queries and assistive tech); without
    /// the override the label is the name.
    #[test]
    fn semantics_label_overrides_the_face_name() {
        let plain = Button::new("B");
        assert_eq!(plain.semantics().label, Some("B"));
        let named = Button::new("B").semantics_label("Bold");
        assert_eq!(named.semantics().label, Some("Bold"));
        assert_eq!(named.label, "B", "the face keeps painting the label");
    }

    /// A host can replace or clear the tooltip in place, and disabling a key
    /// neither drops nor hides it — the tooltip walk never consults
    /// `disabled`, which is what lets a greyed-out key explain *why* it is
    /// grey (the disabled-dead header-key convention).
    #[test]
    fn set_tooltip_survives_disable_and_clears_on_none() {
        let mut b = Button::new("delete");
        assert_eq!(Widget::tooltip(&b), None);
        b.set_tooltip(Some("needs a selected tile".into()));
        b.set_disabled(true);
        assert_eq!(Widget::tooltip(&b), Some("needs a selected tile"));
        b.set_tooltip(None);
        assert_eq!(Widget::tooltip(&b), None);
    }

    /// An icon key measures a square tool cell (not the label-width default),
    /// stamps its stencil inside its own face, and inks it by the same
    /// three-way its label would use: accent when selected, dim when muted,
    /// the theme's ink otherwise.
    #[test]
    fn an_icon_key_measures_square_and_inks_by_state() {
        let (theme, fonts) = theme_fonts();
        let m = theme.metrics();
        let mut ctx = LayoutCtx {
            fonts: &fonts,
            theme: &theme,
            scale: 1.0,
            viewport: Rect::ZERO,
        };
        let mut b = Button::new("pencil").icon(crate::icon::PENCIL);
        let size = b.measure(Size::new(500.0, 500.0), &mut ctx);
        assert_eq!(
            size,
            Size::new(m.control_height, m.control_height),
            "an icon key is a square cell, not a label-wide bar"
        );

        let face = Rect::new(10.0, 10.0, 24.0, 24.0);
        b.arrange(face, &mut ctx);
        let solids_in = |b: &Button, ink: Rgba| -> usize {
            let mut dl = DrawList::new();
            b.draw(
                &mut dl,
                &DrawCtx {
                    fonts: &fonts,
                    theme: &theme,
                    scale: 1.0,
                    hovered: WidgetId::NONE,
                    focused: WidgetId::NONE,
                    pass: DrawPass::Base,
                },
            );
            dl.cmds
                .iter()
                .filter(|c| match c {
                    DrawCmd::Solid { rect, color } => {
                        assert!(
                            rect.x >= face.x - 1.0
                                && rect.y >= face.y - 1.0
                                && rect.right() <= face.right() + 1.0
                                && rect.bottom() <= face.bottom() + 1.0,
                            "an icon quad leaks off the face: {rect:?}"
                        );
                        *color == ink
                    }
                    _ => false,
                })
                .count()
        };

        assert!(
            solids_in(&b, theme.ink()) > 0,
            "at rest the stencil draws in the theme's ink"
        );
        b.set_selected(true);
        assert!(
            solids_in(&b, theme.accent()) > 0,
            "selected, it lights in the accent"
        );
        b.set_selected(false);
        let mut b = Button::new("todo").icon(crate::icon::PENCIL).muted(true);
        b.arrange(face, &mut ctx);
        assert!(
            solids_in(&b, theme.ink_dim()) > 0,
            "muted, it dims without dying"
        );
    }

    /// A zero-width range has no proportional thumb position; the fraction
    /// pins to the track start instead of dividing by zero.
    #[test]
    fn slider_fraction_is_zero_for_a_degenerate_range() {
        let s = Slider::new(3.0, 3.0, 3.0);
        assert_eq!(
            s.value(),
            3.0,
            "the value clamps into the single-point range"
        );
        assert_eq!(
            s.fraction(),
            0.0,
            "a degenerate range pins the thumb to the track start"
        );
    }

    /// `row_at` maps a `y` above the list's top edge to no row — a negative
    /// index must not wrap into the rows.
    #[test]
    fn list_row_at_rejects_points_above_the_list() {
        let (theme, fonts) = theme_fonts();
        let mut ctx = LayoutCtx {
            fonts: &fonts,
            theme: &theme,
            scale: 1.0,
            viewport: Rect::ZERO,
        };
        let mut list = List::new().item("a").item("b");
        list.measure(Size::new(100.0, 100.0), &mut ctx);
        list.arrange(Rect::new(0.0, 50.0, 100.0, 48.0), &mut ctx);
        assert_eq!(list.row_at(40.0), None, "above the top edge is no row");
        assert_eq!(
            list.row_at(50.0 + list.row_h * 1.5),
            Some(1),
            "inside the second row resolves normally"
        );
    }

    /// `set_tex` swaps which texture the quad samples while the widget stays
    /// put in the tree — `Image` and `ImageButton` alike.
    #[test]
    fn set_tex_swaps_the_sampled_texture() {
        let (theme, fonts) = theme_fonts();
        let drawn_tex = |ui: &Ui| -> TextureId {
            let mut dl = DrawList::new();
            ui.draw(&mut dl, &theme, &fonts);
            dl.cmds
                .iter()
                .find_map(|c| match c {
                    DrawCmd::Image { tex, .. } => Some(*tex),
                    _ => None,
                })
                .expect("an image quad is drawn")
        };

        let img = Image::new(TextureId(1)).with_id();
        let id = img.id();
        let mut ui = Ui::new(img);
        ui.layout(Size::new(40.0, 20.0), &theme, &fonts);
        ui.get_mut::<Image>(id).unwrap().set_tex(TextureId(2));
        assert_eq!(
            drawn_tex(&ui),
            TextureId(2),
            "the image quad samples the swapped texture"
        );

        let ib = ImageButton::new(TextureId(1), 24.0, 24.0);
        let bid = ib.id();
        let mut ui = Ui::new(ib);
        ui.layout(Size::new(40.0, 30.0), &theme, &fonts);
        ui.get_mut::<ImageButton>(bid)
            .unwrap()
            .set_tex(TextureId(2));
        assert_eq!(
            drawn_tex(&ui),
            TextureId(2),
            "the image button's icon follows set_tex too"
        );
    }

    /// The three latched command buttons read their own `selected` back. A host
    /// that pushes state into a retained tree every frame (an editor panel
    /// lighting the active tool's key) otherwise has no way to ask what the
    /// widget is showing — `Checkbox::checked`, `Toggle::on` and
    /// `Radio::selected` all answer that already.
    #[test]
    fn a_latched_button_reads_its_own_selected_state() {
        let (theme, fonts) = theme_fonts();

        let b = Button::new("place");
        let id = b.id();
        let mut ui = Ui::new(b);
        ui.layout(Size::new(80.0, 24.0), &theme, &fonts);
        assert!(
            !ui.get::<Button>(id).unwrap().selected(),
            "keys start unlit"
        );
        ui.get_mut::<Button>(id).unwrap().set_selected(true);
        assert!(
            ui.get::<Button>(id).unwrap().selected(),
            "the pushed state reads back"
        );

        let cb = ColorButton::new(Rgba::rgb(200, 40, 40), 24.0, 18.0).with_selected(true);
        assert!(cb.selected(), "a swatch declared chosen reads chosen");
        let ib = ImageButton::new(TextureId(1), 24.0, 24.0);
        assert!(!ib.selected(), "and an icon key starts unlit like the rest");
    }

    /// A button's label takes the em size its `text_role` names — the small face
    /// being what a packed key bank needs, and what a captioned `ColorButton`
    /// beside it already uses.
    #[test]
    fn a_buttons_label_takes_its_text_role() {
        let (theme, fonts) = theme_fonts();
        let mut ctx = LayoutCtx {
            fonts: &fonts,
            theme: &theme,
            scale: 1.0,
            viewport: Rect::ZERO,
        };
        // Long enough that the text, not `button_min_width`, sets the width.
        let label = "rubble S, rubble L, cones";
        let wide = Button::new(label).measure(Size::new(400.0, 40.0), &mut ctx);
        let small = Button::new(label)
            .small()
            .measure(Size::new(400.0, 40.0), &mut ctx);
        assert!(
            small.w < wide.w,
            "the small face measures narrower ({} vs {})",
            small.w,
            wide.w
        );
        assert_eq!(
            small.h, wide.h,
            "the key's height is the theme's control height either way"
        );
    }

    /// A label longer than its key is cut off at the key's own edge. Without the
    /// clip a fixed-width key bank paints each overlong label across the button
    /// beside it — every key in the row ends up unreadable, not just the long one.
    #[test]
    fn a_button_clips_its_label_to_its_own_face() {
        let (theme, fonts) = theme_fonts();
        let mut b = Button::new("a very long command label indeed");
        let mut ctx = LayoutCtx {
            fonts: &fonts,
            theme: &theme,
            scale: 1.0,
            viewport: Rect::ZERO,
        };
        let face = Rect::new(10.0, 10.0, 40.0, 18.0);
        b.measure(Size::new(40.0, 18.0), &mut ctx);
        b.arrange(face, &mut ctx);
        let mut dl = DrawList::new();
        b.draw(
            &mut dl,
            &DrawCtx {
                fonts: &fonts,
                theme: &theme,
                scale: 1.0,
                hovered: WidgetId::NONE,
                focused: WidgetId::NONE,
                pass: DrawPass::Base,
            },
        );
        let clip = dl
            .cmds
            .iter()
            .find_map(|c| match c {
                DrawCmd::PushClip(r) => Some(*r),
                _ => None,
            })
            .expect("the label is drawn inside a clip");
        assert_eq!(clip, face, "and the clip is the button's own face");
    }

    /// A frameless key paints **no face** — nothing at all at rest — and marks
    /// itself with the theme's wash alone when the pointer is on it. That is what
    /// lets one sit inside another control (a tab's close `x`) without reading as
    /// a second, nested button. It still fires: only the paint changed.
    #[test]
    fn a_flat_button_paints_only_the_wash() {
        let (theme, fonts) = theme_fonts();
        let face = Rect::new(10.0, 10.0, 13.0, 13.0);
        // An empty label so the only commands are the face (or its absence).
        let solids = |flat: bool, hovered: bool| -> Vec<(Rect, Rgba)> {
            let mut b = if flat {
                Button::new("").flat()
            } else {
                Button::new("")
            }
            .sized(face.w, face.h);
            let id = b.id();
            let mut ctx = LayoutCtx {
                fonts: &fonts,
                theme: &theme,
                scale: 1.0,
                viewport: Rect::ZERO,
            };
            b.arrange(face, &mut ctx);
            let mut dl = DrawList::new();
            b.draw(
                &mut dl,
                &DrawCtx {
                    fonts: &fonts,
                    theme: &theme,
                    scale: 1.0,
                    hovered: if hovered { id } else { WidgetId::NONE },
                    focused: WidgetId::NONE,
                    pass: DrawPass::Base,
                },
            );
            dl.cmds
                .iter()
                .filter_map(|c| match c {
                    DrawCmd::Solid { rect, color } => Some((*rect, *color)),
                    _ => None,
                })
                .collect()
        };
        assert!(
            solids(true, false).is_empty(),
            "at rest a frameless key paints nothing"
        );
        assert!(
            !solids(false, false).is_empty(),
            "an ordinary key does paint its face at rest"
        );
        assert_eq!(
            solids(true, true),
            vec![(face, Rgba::rgba(0, 0, 0, 51))],
            "hovered, it is exactly one wash over its own rect"
        );
    }

    /// A caption's ink is state — the active tab, the dirty one, the open save
    /// file — so it has to change **in place**: a rebuild would mint a new id and
    /// take hover, arming and focus with it. `set_color(None)` hands the ink back
    /// to the theme.
    #[test]
    fn a_labels_ink_is_syncable_in_place() {
        let (theme, fonts) = theme_fonts();
        let red = Rgba::rgb(255, 0, 0);
        let mut l = Label::new("mars.wrl").with_id();
        let id = l.id();
        assert_eq!(l.ink(), None, "a plain label takes the theme's ink");
        l.set_color(Some(red));
        assert_eq!(l.ink(), Some(red));
        assert_eq!(l.id(), id, "and syncing the ink is not a rebuild");

        let ink_of = |l: &Label| -> Rgba {
            let mut ctx = LayoutCtx {
                fonts: &fonts,
                theme: &theme,
                scale: 1.0,
                viewport: Rect::ZERO,
            };
            let mut l2 = Label::new(l.text());
            l2.set_color(l.ink());
            l2.arrange(Rect::new(0.0, 0.0, 200.0, 20.0), &mut ctx);
            let mut dl = DrawList::new();
            l2.draw(
                &mut dl,
                &DrawCtx {
                    fonts: &fonts,
                    theme: &theme,
                    scale: 1.0,
                    hovered: WidgetId::NONE,
                    focused: WidgetId::NONE,
                    pass: DrawPass::Base,
                },
            );
            // The ink pass is the last glyph run (the theme lays shadow, then
            // hilite, then ink).
            match dl
                .cmds
                .iter()
                .rev()
                .find(|c| matches!(c, DrawCmd::Glyph { .. }))
            {
                Some(DrawCmd::Glyph { color, .. }) => *color,
                _ => panic!("the label drew no glyphs"),
            }
        };
        assert_eq!(ink_of(&l), red, "the explicit ink reaches the glyphs");
        l.set_color(None);
        assert_eq!(
            ink_of(&l),
            crate::theme::Theme::ink(&theme),
            "and clearing it returns the theme's"
        );
    }

    /// A caption sitting on a **raised face** (a tab's, a custom key's) is raised
    /// text by the chrome's own rule, and a label picks its emboss from its role
    /// alone. `raised()` is how it says so — observable here because the theme
    /// adds a hilite pass for raised text and not for engraved.
    #[test]
    fn a_raised_label_gets_the_faces_emboss() {
        let (theme, fonts) = theme_fonts();
        let runs = |l: Label| -> usize {
            let mut l = l;
            let mut ctx = LayoutCtx {
                fonts: &fonts,
                theme: &theme,
                scale: 1.0,
                viewport: Rect::ZERO,
            };
            l.arrange(Rect::new(0.0, 0.0, 200.0, 20.0), &mut ctx);
            let mut dl = DrawList::new();
            l.draw(
                &mut dl,
                &DrawCtx {
                    fonts: &fonts,
                    theme: &theme,
                    scale: 1.0,
                    hovered: WidgetId::NONE,
                    focused: WidgetId::NONE,
                    pass: DrawPass::Base,
                },
            );
            // One "run" per baseline the theme drew the string at.
            let mut pens: Vec<f32> = dl
                .cmds
                .iter()
                .filter_map(|c| match c {
                    DrawCmd::Glyph { pen, .. } => Some(pen.y),
                    _ => None,
                })
                .collect();
            pens.dedup();
            pens.len()
        };
        let plain = runs(Label::new("mars.wrl"));
        assert_eq!(
            runs(Label::new("mars.wrl").raised()),
            plain + 1,
            "raised adds the hilite pass engraved text does not have"
        );
        assert_eq!(
            runs(Label::new("mars.wrl").color(Rgba::rgb(9, 9, 9)).raised()),
            plain + 1,
            "and an explicit ink keeps it"
        );
    }

    /// A bare (unlabelled) checkbox centers its box in whatever cell it is
    /// arranged into and measures to the box alone — a labelled one still leads
    /// with the box at the left edge. Without this a grid of anonymous toggles
    /// reads as a left-hugging staircase.
    #[test]
    fn a_bare_checkbox_centers_its_box_in_its_cell() {
        let (theme, fonts) = theme_fonts();
        let mut ctx = LayoutCtx {
            fonts: &fonts,
            theme: &theme,
            scale: 1.0,
            viewport: Rect::ZERO,
        };
        let mut box_x = |label: &str| -> f32 {
            let mut cb = Checkbox::new(label);
            let cell = Rect::new(100.0, 50.0, 32.0, 32.0);
            cb.measure(cell.size(), &mut ctx);
            cb.arrange(cell, &mut ctx);
            let mut dl = DrawList::new();
            cb.draw(
                &mut dl,
                &DrawCtx {
                    fonts: &fonts,
                    theme: &theme,
                    scale: 1.0,
                    hovered: WidgetId::NONE,
                    focused: WidgetId::NONE,
                    pass: DrawPass::Base,
                },
            );
            dl.cmds
                .iter()
                .find_map(|c| match c {
                    DrawCmd::Solid { rect, .. } => Some(rect.x),
                    _ => None,
                })
                .expect("the box is filled")
        };
        assert_eq!(box_x(""), 108.0, "bare: centered in the 32px cell");
        assert_eq!(box_x("on"), 100.0, "labelled: at the cell's left edge");

        let bare = Checkbox::new("").measure(Size::new(200.0, 32.0), &mut ctx);
        let labelled = Checkbox::new("on").measure(Size::new(200.0, 32.0), &mut ctx);
        assert!(
            bare.w < labelled.w,
            "a bare box measures to the box, not to box + gap + nothing",
        );
    }

    /// A checkbox carries an action tag like every other key, so a panel that
    /// polls one `Ui::actions` channel hears its toggle there too. The whole
    /// arranged rect is the target — the cell, not just the box.
    #[test]
    fn a_checkbox_fires_its_action_tag_from_anywhere_in_its_cell() {
        let (theme, fonts) = theme_fonts();
        let cb = Checkbox::new("").action(0x2a);
        let id = cb.id();
        let mut ui = Ui::new(cb);
        ui.layout_in(Rect::new(0.0, 0.0, 32.0, 32.0), &theme, &fonts);
        let press = |pressed: bool| Event::PointerButton {
            button: PointerButton::Primary,
            pressed,
            pos: Vec2::new(2.0, 2.0), // the cell's corner, well outside the box
            mods: crate::event::Modifiers::NONE,
        };
        ui.dispatch(&[press(true)]);
        assert!(ui.actions().is_empty(), "a press only arms");
        ui.dispatch(&[press(false)]);
        assert_eq!(ui.actions(), [0x2a], "the release fires the tag");
        assert!(
            ui.get::<Checkbox>(id).is_some_and(Checkbox::checked),
            "and the box toggled with it",
        );
    }

    /// A `Separator` is a rule the *theme* draws — the host names the slot, the
    /// skin decides what a rule looks like in it.
    #[test]
    fn a_separator_draws_the_themes_rule_across_its_slot() {
        let (theme, fonts) = theme_fonts();
        let mut ctx = LayoutCtx {
            fonts: &fonts,
            theme: &theme,
            scale: 1.0,
            viewport: Rect::ZERO,
        };
        let mut sep = Separator::new();
        assert_eq!(
            sep.measure(Size::new(120.0, 40.0), &mut ctx),
            Size::new(120.0, SEPARATOR_THICKNESS),
            "a horizontal rule spans the width and asks for the groove's thickness",
        );
        let slot = Rect::new(10.0, 60.0, 120.0, 2.0);
        sep.arrange(slot, &mut ctx);
        let mut dl = DrawList::new();
        sep.draw(
            &mut dl,
            &DrawCtx {
                fonts: &fonts,
                theme: &theme,
                scale: 1.0,
                hovered: WidgetId::NONE,
                focused: WidgetId::NONE,
                pass: DrawPass::Base,
            },
        );
        let rule = dl
            .cmds
            .iter()
            .find_map(|c| match c {
                DrawCmd::Solid { rect, .. } => Some(*rect),
                _ => None,
            })
            .expect("the rule is drawn");
        assert_eq!(rule.x, slot.x, "it spans the slot it was given");
        assert_eq!(rule.w, slot.w);
        assert!(
            rule.y >= slot.y - 1.0 && rule.bottom() <= slot.bottom() + 1.0,
            "and sits on the slot's centre line ({rule:?} in {slot:?})",
        );
    }
}
