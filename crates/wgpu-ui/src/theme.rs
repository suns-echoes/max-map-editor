//! Theming: a [`Theme`] paints widget **parts** (panel, frame, button face,
//! well, text) into a [`DrawList`], driven only by [`WidgetState`]. Widgets never
//! choose colors themselves, so one theme = one visualization of every control —
//! this is the structural fix for "the same control drawn several ways".
//!
//! [`Gunmetal`] is the default: the M.A.X. brushed-gunmetal look (flat material +
//! a directional bevel + darken-washes + embossed text + neon-green accent),
//! built programmatically. A theme could instead skin parts from RGBA sprites
//! without changing any widget.

use crate::color::Rgba;
use crate::draw::DrawList;
use crate::geom::{Rect, Vec2};
use crate::icon::Stencil;
use crate::interact::WidgetState;
use crate::text::{self, FontId, Fonts};

/// Numeric layout tokens, in logical pixels / em pixels. Available at layout time
/// (so widgets size themselves) and draw time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// Default inner padding for containers.
    pub pad: f32,
    /// Default gap between items.
    pub gap: f32,
    /// Bevel ring thickness.
    pub bevel: f32,
    /// Scrollbar width.
    pub scrollbar: f32,
    /// Minimum scrollbar thumb length (floored at the track height, so a short
    /// track never demands a thumb taller than itself).
    pub scrollbar_min_thumb: f32,
    /// Window/dialog titlebar height.
    pub titlebar: f32,
    /// Modal frame thickness.
    pub modal_frame: f32,
    /// Standard control height (button, field).
    pub control_height: f32,
    /// Minimum button width.
    pub button_min_width: f32,
    /// Body / small / title em sizes.
    pub font_body: f32,
    pub font_small: f32,
    pub font_title: f32,
    /// Monospace em size — see [`TextRole::Mono`].
    pub font_mono: f32,
}

impl Default for Metrics {
    fn default() -> Self {
        // Values mirror the M.A.X. editor's theme metrics.
        Self {
            pad: 8.0,
            gap: 6.0,
            bevel: 1.0,
            scrollbar: 8.0,
            scrollbar_min_thumb: 24.0,
            titlebar: 22.0,
            modal_frame: 2.0,
            control_height: 24.0,
            button_min_width: 90.0,
            font_body: 16.0,
            font_small: 12.0,
            font_title: 16.0,
            font_mono: 14.0,
        }
    }
}

/// The visual role of a clickable control.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Role {
    /// Neutral chrome — the default face (toolbar, tabs, selects, list boxes).
    #[default]
    Neutral,
    /// Primary / forward CTA (OK, Create, Save).
    Primary,
    /// Secondary / alternate CTA (Cancel, Abort) — a distinct accent from neutral.
    Secondary,
    /// Destructive CTA (Delete, Discard).
    Danger,
}

/// A semantic text size — and, for [`Mono`](TextRole::Mono), a second face.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextRole {
    #[default]
    Body,
    Small,
    Title,
    /// Fixed-pitch text: a console/terminal line, a hex dump, a code snippet —
    /// content whose *columns* mean something. The only role that can name a
    /// different font ([`Theme::font_for`]); a theme with no second face just
    /// inherits [`Theme::font`] and the role degrades to a size.
    Mono,
}

/// How chrome text is embossed against its surface. `Raised` text sits on a
/// raised face (a button, titlebar, heading, menubar title) and gets a highlight
/// *and* shadow; `Engraved` text is plain content or sits in a well (body copy,
/// fields, text areas, list rows) and gets a shadow only. The default for a role
/// is `Raised` for [`TextRole::Title`] and `Engraved` otherwise (see
/// [`Theme::text`]); widgets on raised faces request `Raised` explicitly via
/// [`Theme::text_em`].
///
/// `Flat` is neither: **no engraving pass at all**, just the glyphs (or the
/// stencil) in the ink. It exists for surfaces where the engraving is not
/// wanted or not readable — a diagnostic that must show the raster alone, ink
/// on a ground the shadow colour would vanish into — and it is the only variant
/// that emits a single pass, so what you see is exactly one rasterization.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Emboss {
    #[default]
    Raised,
    Engraved,
    Flat,
}

/// How a frame's bevel is shaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Bevel {
    #[default]
    Raised,
    Inset,
    Flat,
}

/// [`Theme::accent_row`] tint floors shared by the stock row highlights — a
/// select's option list, menu cascade rows, menu bar headers: plain hover.
pub const ROW_FLOOR_HOVER: f32 = 0.30;
/// [`ROW_FLOOR_HOVER`]'s active step: the selected/open row.
pub const ROW_FLOOR_ACTIVE: f32 = 0.40;
/// [`ROW_FLOOR_HOVER`]'s strongest step: the selected row under the pointer.
pub const ROW_FLOOR_ACTIVE_HOVER: f32 = 0.70;

/// The 1px outset frame weight of a [`Theme::popup`] surface. Row highlights
/// inside a popup inset by this so they stay inside the frame — only a button
/// covers its own bevel.
pub const POPUP_FRAME: f32 = 1.0;

/// Paints widget chrome. Implement once per look; widgets call these and pass
/// only [`WidgetState`], never colors.
pub trait Theme {
    /// Upcast to [`Any`](std::any::Any) so a host's *custom* widget can recover
    /// the concrete theme (e.g. to call look-specific drawing the trait doesn't
    /// expose) via `theme.as_any().downcast_ref::<MyTheme>()`. Implement as
    /// `fn as_any(&self) -> &dyn std::any::Any { self }`.
    fn as_any(&self) -> &dyn std::any::Any;

    fn metrics(&self) -> Metrics;

    /// The UI logical→physical scale this theme is drawing at — 1.0 unless the
    /// theme tracks it. Everything that *engraves* (the text shadow/hilite, the
    /// icon emboss) reads it, so those passes land on whole physical pixels
    /// instead of blurring across a fractional one; see [`emboss_offset`].
    fn scale(&self) -> f32 {
        1.0
    }

    /// The font used for chrome text.
    fn font(&self) -> FontId;

    /// The font for one text role. Chrome is one face, so the default answers
    /// [`font`](Self::font) for every role; a theme that ships a second face
    /// overrides this for [`TextRole::Mono`] (and must register that font in the
    /// same [`Fonts`] the host draws with). **Every text path resolves its face
    /// through this**, so an override reaches measurement and drawing alike.
    fn font_for(&self, _role: TextRole) -> FontId {
        self.font()
    }

    /// Em size (px) for a text role — for measuring before drawing.
    fn font_px(&self, role: TextRole) -> f32 {
        let m = self.metrics();
        match role {
            TextRole::Body => m.font_body,
            TextRole::Small => m.font_small,
            TextRole::Title => m.font_title,
            TextRole::Mono => m.font_mono,
        }
    }

    // Semantic colors, for content widgets draw themselves (e.g. swatch rings).
    fn accent(&self) -> Rgba;
    fn ink(&self) -> Rgba;
    fn ink_dim(&self) -> Rgba;

    /// A window/panel background.
    fn panel(&self, dl: &mut DrawList, rect: Rect);

    /// A floating popup/menu surface — a select's open option list, a dropdown
    /// menu. Like [`panel`](Self::panel) but framed as a thin 1px outset (a panel
    /// is a heavier window frame). The default delegates to `panel`; a theme
    /// whose panel frame is thicker overrides this to draw the lighter 1px frame.
    fn popup(&self, dl: &mut DrawList, rect: Rect) {
        self.panel(dl, rect);
    }

    /// Fills `rect` as a list / option-row **state highlight**, tinted toward the
    /// accent by `floor` (0..1: a light hover, a stronger active/selected). The
    /// default is a translucent accent wash so flat themes work; a material theme
    /// overrides this to own a tinted crop of its surface, so the highlight's
    /// texture stays continuous with the list around it (not a flat overlay).
    fn accent_row(&self, dl: &mut DrawList, rect: Rect, floor: f32) {
        let a = (floor.clamp(0.0, 1.0) * 220.0) as u8;
        dl.fill_rect(rect, self.accent().with_alpha(a));
    }

    /// The [`accent_row`](Self::accent_row) variant for a row **inside a
    /// [`well`](Self::well)** (a list box, a saved-items pane): a theme whose
    /// well surface differs from its popup/panel surface overrides this so the
    /// tinted row is a crop of the *well* material, keeping the highlight's
    /// texture continuous with the well around it. Default: `accent_row`.
    fn accent_well_row(&self, dl: &mut DrawList, rect: Rect, floor: f32) {
        self.accent_row(dl, rect, floor);
    }

    /// A window/dialog titlebar band over `rect`. The default is a raised
    /// secondary band (a button face). A theme whose window is a single framed
    /// surface — the titlebar enclosed by the same panel bevel, with a
    /// continuous background — overrides this to blend the titlebar into the
    /// panel (e.g. draw nothing, leaving just the title text).
    fn titlebar(&self, dl: &mut DrawList, rect: Rect) {
        self.button(dl, rect, Role::Neutral, WidgetState::default());
    }

    /// A `fill`ed rectangle with a `bevel` ring.
    fn frame(&self, dl: &mut DrawList, rect: Rect, fill: Rgba, bevel: Bevel);

    /// A clickable button face for `role` in `state`.
    fn button(&self, dl: &mut DrawList, rect: Rect, role: Role, state: WidgetState);

    /// The hover / press feedback of a **frameless** control — one that paints no
    /// face of its own (a [`Button::flat`](crate::widgets::Button::flat) icon key
    /// like a tab's close `x`), so the only thing marking it as clickable is this
    /// wash over whatever it sits on.
    ///
    /// The default darkens: nothing at rest or disabled, black at 20% hovered and
    /// 28% pressed. A theme whose chrome lightens instead overrides it. Drawn
    /// *under* the control's label, and it must stay inside `rect` — a frameless
    /// key sits inside another control's face, and a wash bleeding past its own
    /// rect would read as that control lighting up.
    fn wash(&self, dl: &mut DrawList, rect: Rect, state: WidgetState) {
        if state.disabled {
            return;
        }
        let a = if state.pressed {
            71
        } else if state.hovered {
            51
        } else {
            return;
        };
        dl.fill_rect(rect, Rgba::rgba(0, 0, 0, a));
    }

    /// An inset well (text field, checkbox box, track background) in `state`.
    fn well(&self, dl: &mut DrawList, rect: Rect, state: WidgetState);

    /// A vertical scrollbar: the `track` column and the `thumb` riding it, in
    /// `state` (hovered/pressed describe the *thumb*). Every scrolling widget
    /// paints through this — [`Scroller`](crate::Scroller), and so
    /// [`ScrollArea`](crate::ScrollArea) and [`TextArea`](crate::TextArea) —
    /// so one theme is one bar everywhere.
    ///
    /// The default is a [`well`](Self::well) track under a neutral
    /// [`button`](Self::button) thumb. A theme whose bar is flatter (an inset
    /// slab, no bevel) overrides this; the geometry is already resolved, so an
    /// override only chooses the paint.
    fn scrollbar(&self, dl: &mut DrawList, track: Rect, thumb: Rect, state: WidgetState) {
        self.well(dl, track, WidgetState::default());
        self.button(dl, thumb, Role::Neutral, state);
    }

    /// Fills `rect` with the theme's neutral surface material — the backdrop a
    /// menu bar, toolbar, or [`Well`](crate::widgets::Well) sits on, *without* a
    /// panel's border ring. A textured theme (a brushed-metal skin, say)
    /// overrides this to paint its material; the default is a flat fill of the
    /// dim-ink color. This is the trait-level hook for material fills — what a
    /// host previously had to reach around the trait to do.
    fn surface(&self, dl: &mut DrawList, rect: Rect) {
        self.frame(dl, rect, self.ink_dim(), Bevel::Flat);
    }

    /// A panel-header / status strip band — the emphasized bar a panel title
    /// row, toolbar, or status line sits on, *without* a frame. The default
    /// paints the neutral [`surface`](Self::surface); a material theme
    /// overrides it with its header material — the trait-level hook a custom
    /// header widget previously had to reach around the trait for.
    fn header_band(&self, dl: &mut DrawList, rect: Rect) {
        self.surface(dl, rect);
    }

    /// The draggable divider a [`Split`](crate::Split) paints between its two
    /// children — the full grab band, not a hairline. `vertical` is the bar's
    /// own orientation (`true` = a vertical bar between side-by-side
    /// children); `state`'s hovered/pressed light it while the pointer rests
    /// on it or drags it. The default is the neutral
    /// [`surface`](Self::surface) material under the standard
    /// [`wash`](Self::wash) — a flat theme gets a flat seam that answers the
    /// pointer; a material theme overrides for grip texture.
    fn divider(&self, dl: &mut DrawList, rect: Rect, vertical: bool, state: WidgetState) {
        let _ = vertical;
        self.surface(dl, rect);
        self.wash(dl, rect, state);
    }

    /// A thin horizontal rule between groups of rows (a menu separator, a
    /// dialog section break), centered vertically in `rect`. The default is a
    /// 1px dim-ink hairline; a material theme overrides it with its etched
    /// treatment (e.g. an engraved dark-over-light groove).
    fn separator(&self, dl: &mut DrawList, rect: Rect) {
        let y = (rect.center().y - 0.5).floor();
        dl.fill_rect(Rect::new(rect.x, y, rect.w, 1.0), self.ink_dim());
    }

    /// [`separator`](Self::separator)'s vertical counterpart — a thin rule
    /// between columns (a columns submenu, a toolbar group break), centered
    /// horizontally in `rect`. The default is a 1px dim-ink hairline; a
    /// material theme overrides it with the same etched treatment as its
    /// horizontal rule.
    fn vseparator(&self, dl: &mut DrawList, rect: Rect) {
        let x = (rect.center().x - 0.5).floor();
        dl.fill_rect(Rect::new(x, rect.y, 1.0, rect.h), self.ink_dim());
    }

    /// Draws a `kind` bevel ring of thickness `px` inside `rect` (the
    /// raised/inset edge treatment the theme uses for panels and buttons), so a
    /// custom widget can request the same edges at any weight (a 1px control
    /// ring, a 2px window frame). Widgets pass `metrics().bevel` for the stock
    /// weight. Default: nothing (a flat theme draws no bevel).
    fn bevel(&self, dl: &mut DrawList, rect: Rect, kind: Bevel, px: f32) {
        let _ = (dl, rect, kind, px);
    }

    /// Themed text with its baseline pen origin at `baseline`, embossed to match
    /// the role (headings/titles `Raised`, body/small `Engraved`). Returns the
    /// advance width. Use [`text_em`](Self::text_em) to force an emboss.
    fn text(
        &self,
        dl: &mut DrawList,
        fonts: &Fonts,
        baseline: Vec2,
        s: &str,
        role: TextRole,
    ) -> f32 {
        self.text_em(dl, fonts, baseline, s, role, role_emboss(role))
    }

    /// Themed text with an explicit [`Emboss`] — e.g. `Raised` text on a button
    /// face that is otherwise `Body`-sized. Returns the advance width. The
    /// default draws in the main [`ink`](Self::ink); a theme whose chrome ink
    /// varies by role (an amber title, a dim small) overrides this.
    fn text_em(
        &self,
        dl: &mut DrawList,
        fonts: &Fonts,
        baseline: Vec2,
        s: &str,
        role: TextRole,
        emboss: Emboss,
    ) -> f32 {
        self.text_colored(dl, fonts, baseline, s, role, emboss, self.ink())
    }

    /// Themed text in the **secondary / dim ink** — hints, readouts, placeholder
    /// and disabled-ish copy — with the role's default emboss. Returns the
    /// advance width. The default draws in [`ink_dim`](Self::ink_dim), so a
    /// theme that only implements [`text_colored`](Self::text_colored) mutes
    /// correctly by construction.
    fn text_muted(
        &self,
        dl: &mut DrawList,
        fonts: &Fonts,
        baseline: Vec2,
        s: &str,
        role: TextRole,
    ) -> f32 {
        self.text_colored(
            dl,
            fonts,
            baseline,
            s,
            role,
            role_emboss(role),
            self.ink_dim(),
        )
    }

    /// Themed text in the **accent color** with an explicit [`Emboss`] — the
    /// "active / selected" emphasis a lit control's label uses (an active tool
    /// key, the current tab, the open menu title). Returns the advance width.
    /// The default draws in [`accent`](Self::accent), so a theme that only
    /// implements [`text_colored`](Self::text_colored) accents correctly by
    /// construction.
    fn text_accent(
        &self,
        dl: &mut DrawList,
        fonts: &Fonts,
        baseline: Vec2,
        s: &str,
        role: TextRole,
        emboss: Emboss,
    ) -> f32 {
        self.text_colored(dl, fonts, baseline, s, role, emboss, self.accent())
    }

    /// Themed text in an **arbitrary ink** with an explicit [`Emboss`] — **the
    /// one required text primitive**: how this theme renders a line of text in a
    /// given color (typically the em size for `role`, a shadow/hilite treatment
    /// per `emboss`, the glyphs in exactly `ink`). Returns the advance width.
    /// Every semantic helper (`text`/`text_em`/`text_muted`/`text_accent`)
    /// derives from it with the right ink by default, so a minimal theme renders
    /// all text variants correctly by implementing just this.
    #[allow(clippy::too_many_arguments)]
    fn text_colored(
        &self,
        dl: &mut DrawList,
        fonts: &Fonts,
        baseline: Vec2,
        s: &str,
        role: TextRole,
        emboss: Emboss,
        ink: Rgba,
    ) -> f32;

    /// [`text_colored`](Self::text_colored) in an explicit **face and em size**
    /// rather than a role — for a content widget whose *domain* dictates the
    /// size (a zoomable canvas's labels, a rasterizer diagnostic sweeping em
    /// sizes). Ordinary chrome must not reach for this: sizes are roles, so the
    /// theme stays in charge of how big text is.
    ///
    /// The default is the stock engraving: a down-right shadow at
    /// [`emboss_offset`] whole physical pixels, an up-left hilite for `Raised`,
    /// then the glyphs in `ink` — `Flat` draws the glyphs alone. A theme with
    /// its own engraving inks overrides this one method and gets both paths.
    #[allow(clippy::too_many_arguments)]
    fn text_run(
        &self,
        dl: &mut DrawList,
        fonts: &Fonts,
        font: FontId,
        baseline: Vec2,
        s: &str,
        px: f32,
        emboss: Emboss,
        ink: Rgba,
    ) -> f32 {
        if emboss != Emboss::Flat {
            let o = emboss_offset(self.scale());
            text::draw_line(
                dl,
                fonts,
                font,
                s,
                baseline + Vec2::new(o, o),
                px,
                TEXT_SHADOW,
            );
            if emboss == Emboss::Raised {
                text::draw_line(
                    dl,
                    fonts,
                    font,
                    s,
                    baseline - Vec2::new(o, o),
                    px,
                    TEXT_HILITE,
                );
            }
        }
        text::draw_line(dl, fonts, font, s, baseline, px, ink)
    }

    // --- text in a *rect*, for a widget that draws its own -------------------
    //
    // The five calls above take a baseline, which is what a widget wants once it
    // knows where the line goes. These three answer the question before that —
    // *how does this string sit in this box* — and are what a widget drawing its
    // own domain text (a cell caption, an empty-state note) would otherwise
    // hand-roll from `font_px` and the font metrics. `Label` is the same
    // behaviors as a widget; these are them without one.

    /// Cuts `s` to `max_w` with a trailing `...` marker, or returns it whole
    /// when it already fits.
    ///
    /// The marker is three ASCII dots rather than `…` (U+2026) so it renders in
    /// a font that carries only the ASCII range.
    fn ellipsized(&self, fonts: &Fonts, s: &str, role: TextRole, max_w: f32) -> String {
        let font = self.font_for(role);
        let px = self.font_px(role);
        if fonts.measure(font, s, px) <= max_w {
            return s.to_string();
        }
        const DOTS: &str = "...";
        let dots = fonts.measure(font, DOTS, px);
        let mut out = String::new();
        let mut acc = 0.0;
        // The per-character width is measured through a stack buffer rather than
        // a fresh `String` per char: this runs per ellipsizing label per frame,
        // and the allocation bought nothing.
        let mut buf = [0u8; 4];
        for ch in s.chars() {
            let cw = fonts.measure(font, ch.encode_utf8(&mut buf), px);
            if acc + cw + dots > max_w {
                break;
            }
            out.push(ch);
            acc += cw;
        }
        out.push_str(DOTS);
        out
    }

    /// One line whose **top-left** is `at` — for text laid out from a cell's
    /// corner rather than from a baseline. Returns the advance width.
    #[allow(clippy::too_many_arguments)]
    fn text_top(
        &self,
        dl: &mut DrawList,
        fonts: &Fonts,
        at: Vec2,
        s: &str,
        role: TextRole,
        emboss: Emboss,
        ink: Rgba,
    ) -> f32 {
        // Cell top → baseline: the middle of a `px`-tall cell, plus the same
        // 0.34 drop every rect-centred line here uses.
        let px = self.font_px(role);
        self.text_colored(
            dl,
            fonts,
            Vec2::new(at.x, at.y + px * 0.84),
            s,
            role,
            emboss,
            ink,
        )
    }

    /// One line centred in `rect` and inset `pad` from both sides,
    /// [`ellipsized`](Self::ellipsized) to fit and **clipped** to the rect — a
    /// caption in a box that cannot grow (a file name, a cell's tag).
    #[allow(clippy::too_many_arguments)]
    fn text_fit(
        &self,
        dl: &mut DrawList,
        fonts: &Fonts,
        rect: Rect,
        pad: f32,
        s: &str,
        role: TextRole,
        emboss: Emboss,
        ink: Rgba,
    ) {
        let px = self.font_px(role);
        let fitted = self.ellipsized(fonts, s, role, (rect.w - 2.0 * pad).max(0.0));
        dl.push_clip(rect);
        let baseline = Vec2::new(rect.x + pad, rect.y + rect.h * 0.5 + px * 0.34);
        self.text_colored(dl, fonts, baseline, &fitted, role, emboss, ink);
        dl.pop_clip();
    }

    /// A word-wrapped block filling `rect` from the top, inset by `pad` — an
    /// explanatory note in a panel body. Returns the height drawn, so a caller
    /// can lay out under it. Not clipped: the caller owns the box it chose.
    #[allow(clippy::too_many_arguments)]
    fn text_wrapped(
        &self,
        dl: &mut DrawList,
        fonts: &Fonts,
        rect: Rect,
        pad: f32,
        s: &str,
        role: TextRole,
        emboss: Emboss,
        ink: Rgba,
    ) -> f32 {
        let px = self.font_px(role);
        let font = self.font_for(role);
        let lh = fonts.line_height(font, px);
        let lines = fonts.wrap(font, s, px, (rect.w - 2.0 * pad).max(0.0));
        let mut y = rect.y + pad + px;
        for line in &lines {
            self.text_colored(
                dl,
                fonts,
                Vec2::new(rect.x + pad, y),
                line,
                role,
                emboss,
                ink,
            );
            y += lh;
        }
        lines.len() as f32 * lh
    }

    // --- icons ---------------------------------------------------------------

    /// A themed [`Icon`] stamped into `rect` in `ink` — the icon counterpart of
    /// [`text_colored`](Self::text_colored), and the one call every icon-bearing
    /// widget makes, so a theme re-treats all icons in one place. The default
    /// gives the stencil the same emboss treatment as the default text: a
    /// down-right shadow always, an up-left hilite for `Raised`, then the ink —
    /// and for [`Emboss::Flat`], the ink alone.
    fn icon(&self, dl: &mut DrawList, rect: Rect, stencil: Stencil, emboss: Emboss, ink: Rgba) {
        // One *physical* pixel: the stencil is stamped 1:1 (see `icon::fit`),
        // so one pixel IS one cell of the art drawn for this size, and the
        // engraving stays proportional instead of fattening with the scale.
        let o = 1.0 / self.scale().max(1e-4);
        if emboss != Emboss::Flat {
            stencil.draw(dl, rect.translate(Vec2::new(o, o)), TEXT_SHADOW);
            if emboss == Emboss::Raised {
                stencil.draw(dl, rect.translate(Vec2::new(-o, -o)), TEXT_HILITE);
            }
        }
        stencil.draw(dl, rect, ink);
    }

    // --- tooltips ------------------------------------------------------------

    /// A hover tooltip for the control at `anchor`: a small captioned plate
    /// the `Ui` paints at the end of the **overlay pass** once the pointer has
    /// rested on a widget carrying one
    /// ([`Widget::tooltip`](crate::widget::Widget::tooltip)). Placed centred
    /// under the anchor, flipped above it when the viewport's bottom edge is
    /// in the way, and shifted clear of the sides; an **empty** viewport (the
    /// default) means unconstrained, like a popup's.
    fn tooltip(&self, dl: &mut DrawList, fonts: &Fonts, anchor: Rect, viewport: Rect, text: &str) {
        let px = self.font_px(TextRole::Small);
        let (pad, gap) = (5.0, 4.0);
        // Measure through the `Fonts` facade — `Fonts::get` is the
        // hand-rolled escape hatch and panics on any other backend.
        let w = fonts.measure(self.font_for(TextRole::Small), text, px) + 2.0 * pad;
        let h = px + 2.0 * pad;
        let mut x = anchor.center().x - w * 0.5;
        let mut y = anchor.bottom() + gap;
        if !viewport.is_empty() {
            x = x.clamp(viewport.x, (viewport.right() - w).max(viewport.x));
            if y + h > viewport.bottom() {
                y = anchor.y - gap - h;
            }
        }
        let rect = Rect::new(x, y, w, h);
        self.popup(dl, rect);
        let baseline = Vec2::new(rect.x + pad, rect.y + rect.h * 0.5 + px * 0.34);
        self.text_colored(
            dl,
            fonts,
            baseline,
            text,
            TextRole::Small,
            Emboss::Engraved,
            self.ink(),
        );
    }
}

/// The default emboss for a role's plain text: titles read as `Raised` chrome,
/// body/small as `Engraved` content (see [`Emboss`]).
fn role_emboss(role: TextRole) -> Emboss {
    match role {
        TextRole::Title => Emboss::Raised,
        TextRole::Body | TextRole::Small | TextRole::Mono => Emboss::Engraved,
    }
}

// --- Gunmetal: the default programmatic theme -------------------------------

// sRGB palette evoking the M.A.X. shell (brushed gunmetal + neon-green accent).
const INK: Rgba = Rgba::rgb(210, 214, 220);
const INK_DIM: Rgba = Rgba::rgb(128, 133, 143);
const ACCENT: Rgba = Rgba::rgb(68, 255, 0);
const PANEL_BASE: Rgba = Rgba::rgb(42, 45, 51);
const BUTTON_BASE: Rgba = Rgba::rgb(60, 62, 66);
const BUTTON_PRIMARY: Rgba = Rgba::rgb(120, 86, 34);
const BUTTON_SECONDARY: Rgba = Rgba::rgb(120, 100, 40);
const BUTTON_DANGER: Rgba = Rgba::rgb(120, 52, 42);
const DISABLED_BASE: Rgba = Rgba::rgb(46, 48, 52);
const WELL_BASE: Rgba = Rgba::rgb(22, 24, 28);
const BORDER: Rgba = Rgba::rgb(78, 84, 92);

// Bevel overlays (top-left light, bottom-right dark).
const BEVEL_TOP: Rgba = Rgba::rgba(255, 255, 255, 41);
const BEVEL_LEFT: Rgba = Rgba::rgba(255, 255, 255, 61);
const BEVEL_BOTTOM: Rgba = Rgba::rgba(0, 0, 0, 107);
const BEVEL_RIGHT: Rgba = Rgba::rgba(0, 0, 0, 140);

// Interaction washes (darken the chrome; selection uses the accent).
const HOVER_WASH: Rgba = Rgba::rgba(0, 0, 0, 51);
const PRESS_WASH: Rgba = Rgba::rgba(0, 0, 0, 71);
const SELECT_WASH: Rgba = Rgba::rgba(68, 255, 0, 64);

// Embossed text overlays.
const TEXT_SHADOW: Rgba = Rgba::rgba(0, 0, 0, 140);
const TEXT_HILITE: Rgba = Rgba::rgba(255, 255, 255, 41);

/// The baseline offset an engraved (embossed) text pass draws at, in **logical**
/// px, for a UI at `scale`: `ceil(scale)` whole *physical* pixels.
///
/// The shadow and hilite are copies of the glyph offset by this much, drawn
/// under it — so the offset has to clear the glyph's own anti-aliased edge. A
/// glyph rasterized at a fractional multiple of its design grid has a fringe a
/// full pixel wide, and a one-pixel offset drops the shadow *inside* that
/// fringe: the stroke it was meant to engrave comes out grey and smudged
/// instead. Rounding the offset **up** keeps the shadow clear of the ink at
/// every scale, and leaves 100% (where it is one pixel) exactly as it was.
///
/// Themes apply it against the scale the `Ui` is drawing at
/// ([`DrawCtx::scale`](crate::widget::DrawCtx::scale)).
pub fn emboss_offset(scale: f32) -> f32 {
    let s = scale.max(1e-4);
    s.ceil() / s
}

/// The default brushed-gunmetal theme.
pub struct Gunmetal {
    metrics: Metrics,
    font: FontId,
    /// The UI logical→physical scale, for [`emboss_offset`]. Set each frame via
    /// [`set_scale`](Self::set_scale).
    scale: f32,
}

impl Gunmetal {
    /// Creates the theme with the font to use for chrome text (register it in a
    /// [`Fonts`] first and pass its id).
    pub fn new(font: FontId) -> Self {
        Self {
            metrics: Metrics::default(),
            font,
            scale: 1.0,
        }
    }

    pub fn with_metrics(mut self, metrics: Metrics) -> Self {
        self.metrics = metrics;
        self
    }

    /// Sets the UI logical→physical scale (call each frame before `Ui::draw`),
    /// so the text/icon engraving lands on whole physical pixels — see
    /// [`emboss_offset`].
    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale.max(1e-4);
    }
}

impl Theme for Gunmetal {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn metrics(&self) -> Metrics {
        self.metrics
    }

    fn scale(&self) -> f32 {
        self.scale
    }

    fn font(&self) -> FontId {
        self.font
    }

    fn accent(&self) -> Rgba {
        ACCENT
    }

    fn ink(&self) -> Rgba {
        INK
    }

    fn ink_dim(&self) -> Rgba {
        INK_DIM
    }

    fn panel(&self, dl: &mut DrawList, rect: Rect) {
        dl.fill_rect(rect, PANEL_BASE);
        dl.stroke_rect(rect, self.metrics.bevel, BORDER);
        self.bevel(dl, rect, Bevel::Raised, self.metrics.bevel);
    }

    fn frame(&self, dl: &mut DrawList, rect: Rect, fill: Rgba, bevel: Bevel) {
        dl.fill_rect(rect, fill);
        self.bevel(dl, rect, bevel, self.metrics.bevel);
    }

    fn button(&self, dl: &mut DrawList, rect: Rect, role: Role, state: WidgetState) {
        let base = if state.disabled {
            DISABLED_BASE
        } else {
            match role {
                Role::Neutral => BUTTON_BASE,
                Role::Primary => BUTTON_PRIMARY,
                Role::Secondary => BUTTON_SECONDARY,
                Role::Danger => BUTTON_DANGER,
            }
        };
        dl.fill_rect(rect, base);
        // Pressed faces read as inset; everything else as raised.
        let bevel = if state.pressed && !state.disabled {
            Bevel::Inset
        } else {
            Bevel::Raised
        };
        self.bevel(dl, rect, bevel, self.metrics.bevel);
        if state.selected {
            dl.fill_rect(rect, SELECT_WASH);
        }
        if !state.disabled {
            if state.pressed {
                dl.fill_rect(rect, PRESS_WASH);
            } else if state.hovered {
                dl.fill_rect(rect, HOVER_WASH);
            }
        }
    }

    fn well(&self, dl: &mut DrawList, rect: Rect, state: WidgetState) {
        dl.fill_rect(rect, WELL_BASE);
        self.bevel(dl, rect, Bevel::Inset, self.metrics.bevel);
        if state.focused {
            dl.stroke_rect(rect, self.metrics.bevel, ACCENT);
        }
    }

    fn surface(&self, dl: &mut DrawList, rect: Rect) {
        dl.fill_rect(rect, PANEL_BASE);
    }

    /// Draws a bevel ring inside `rect` (top-left light, bottom-right dark for
    /// `Raised`; swapped for `Inset`; nothing for `Flat`).
    fn bevel(&self, dl: &mut DrawList, rect: Rect, kind: Bevel, px: f32) {
        let t = px;
        if rect.is_empty() || t <= 0.0 || matches!(kind, Bevel::Flat) {
            return;
        }
        let (top, left, bottom, right) = match kind {
            Bevel::Raised => (BEVEL_TOP, BEVEL_LEFT, BEVEL_BOTTOM, BEVEL_RIGHT),
            // Inset swaps the light and dark edges.
            Bevel::Inset => (BEVEL_BOTTOM, BEVEL_RIGHT, BEVEL_TOP, BEVEL_LEFT),
            Bevel::Flat => unreachable!(),
        };
        dl.fill_rect(Rect::new(rect.x, rect.y, rect.w, t), top);
        dl.fill_rect(Rect::new(rect.x, rect.bottom() - t, rect.w, t), bottom);
        dl.fill_rect(Rect::new(rect.x, rect.y + t, t, rect.h - 2.0 * t), left);
        dl.fill_rect(
            Rect::new(rect.right() - t, rect.y + t, t, rect.h - 2.0 * t),
            right,
        );
    }

    /// Draws `s` embossed against its surface in `ink`: a down-right shadow
    /// always, an up-left hilite for raised faces, then the main ink. The one
    /// text primitive — every semantic text helper derives from it.
    #[allow(clippy::too_many_arguments)]
    fn text_colored(
        &self,
        dl: &mut DrawList,
        fonts: &Fonts,
        baseline: Vec2,
        s: &str,
        role: TextRole,
        emboss: Emboss,
        ink: Rgba,
    ) -> f32 {
        // Through `font_for`, so a theme that registers a second face for
        // `TextRole::Mono` draws with it here and measures with it everywhere.
        // The engraving itself is `text_run`'s — one implementation, whether the
        // size came from a role or from a content widget's domain.
        self.text_run(
            dl,
            fonts,
            self.font_for(role),
            baseline,
            s,
            self.font_px(role),
            emboss,
            ink,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::DrawCmd;

    #[test]
    fn default_metrics_match_max_look() {
        let m = Metrics::default();
        assert_eq!(m.scrollbar, 8.0);
        assert_eq!(m.titlebar, 22.0);
        assert_eq!(m.font_body, 16.0);
    }

    /// The engraving offset is a whole number of *physical* pixels, rounded up
    /// — one at 100% (so that look is untouched), two at any fractional scale,
    /// where a one-pixel shadow would fall inside the glyph's own AA fringe.
    #[test]
    fn emboss_offset_is_whole_physical_pixels_rounded_up() {
        for (scale, want) in [(1.0, 1.0), (1.25, 2.0), (1.5, 2.0), (2.0, 2.0)] {
            let phys = emboss_offset(scale) * scale;
            assert!(
                (phys - want).abs() < 1e-3,
                "{scale}x engraves {phys} physical px, want {want}"
            );
        }
        assert!(
            emboss_offset(0.0).is_finite(),
            "a degenerate scale is clamped, not divided by"
        );
    }

    /// `Emboss::Flat` emits exactly one pass, `Engraved` two and `Raised`
    /// three — for text and for icons alike. Anything measuring a raster has to
    /// be able to ask for one rasterization and get one.
    #[test]
    fn flat_emboss_draws_a_single_pass() {
        use crate::draw::DrawCmd;
        use crate::icon;

        let g = Gunmetal::new(FontId(0));
        // Icons: a stencil run is a solid quad, so the passes are countable
        // without a font — each pass repeats the same run count.
        let stencil = icon::PENCIL.pick(16).0;
        let quads = |emboss| {
            let mut dl = DrawList::new();
            g.icon(
                &mut dl,
                Rect::new(0.0, 0.0, 16.0, 16.0),
                stencil,
                emboss,
                Rgba::WHITE,
            );
            dl.cmds
                .iter()
                .filter(|c| matches!(c, DrawCmd::Solid { .. }))
                .count()
        };
        let flat = quads(Emboss::Flat);
        assert!(flat > 0, "a flat icon still draws");
        assert_eq!(quads(Emboss::Engraved), flat * 2, "shadow + ink");
        assert_eq!(quads(Emboss::Raised), flat * 3, "shadow + hilite + ink");
    }

    #[test]
    fn font_px_by_role() {
        let g = Gunmetal::new(FontId(0));
        assert_eq!(g.font_px(TextRole::Body), 16.0);
        assert_eq!(g.font_px(TextRole::Small), 12.0);
    }

    #[test]
    fn as_any_recovers_the_concrete_theme() {
        let g = Gunmetal::new(FontId(7));
        let dynamic: &dyn Theme = &g;
        let back = dynamic.as_any().downcast_ref::<Gunmetal>();
        assert!(
            back.is_some(),
            "a custom widget can recover the concrete theme"
        );
        assert_eq!(back.unwrap().font(), FontId(7));
    }

    /// A theme that implements ONLY the required methods must render every
    /// semantic text variant in the right ink by construction — the defaults
    /// derive from `text_colored`, they never silently drop the color.
    #[test]
    fn minimal_theme_text_defaults_use_the_right_inks() {
        use crate::draw::DrawCmd;

        /// Records the ink each text call resolves to (as a solid fill).
        struct Probe;
        impl Theme for Probe {
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
            fn metrics(&self) -> Metrics {
                Metrics::default()
            }
            fn font(&self) -> FontId {
                FontId(0)
            }
            fn accent(&self) -> Rgba {
                Rgba::rgb(1, 0, 0)
            }
            fn ink(&self) -> Rgba {
                Rgba::rgb(2, 0, 0)
            }
            fn ink_dim(&self) -> Rgba {
                Rgba::rgb(3, 0, 0)
            }
            fn panel(&self, _: &mut DrawList, _: Rect) {}
            fn frame(&self, _: &mut DrawList, _: Rect, _: Rgba, _: Bevel) {}
            fn button(&self, _: &mut DrawList, _: Rect, _: Role, _: WidgetState) {}
            fn well(&self, _: &mut DrawList, _: Rect, _: WidgetState) {}
            fn text_colored(
                &self,
                dl: &mut DrawList,
                _: &Fonts,
                _: Vec2,
                _: &str,
                _: TextRole,
                _: Emboss,
                ink: Rgba,
            ) -> f32 {
                dl.fill_rect(Rect::new(0.0, 0.0, 1.0, 1.0), ink);
                0.0
            }
        }

        let ink_of = |draw: &dyn Fn(&Probe, &mut DrawList)| {
            let mut dl = DrawList::new();
            draw(&Probe, &mut dl);
            match dl.cmds.as_slice() {
                [DrawCmd::Solid { color, .. }] => *color,
                other => panic!("expected one recorded ink, got {other:?}"),
            }
        };

        let fonts = Fonts::new();
        let at = Vec2::new(0.0, 0.0);
        let t = |p: &Probe, dl: &mut DrawList| {
            p.text(dl, &fonts, at, "x", TextRole::Body);
        };
        assert_eq!(ink_of(&t), Rgba::rgb(2, 0, 0), "text draws in ink()");
        let em = |p: &Probe, dl: &mut DrawList| {
            p.text_em(dl, &fonts, at, "x", TextRole::Body, Emboss::Raised);
        };
        assert_eq!(ink_of(&em), Rgba::rgb(2, 0, 0), "text_em draws in ink()");
        let muted = |p: &Probe, dl: &mut DrawList| {
            p.text_muted(dl, &fonts, at, "x", TextRole::Body);
        };
        assert_eq!(
            ink_of(&muted),
            Rgba::rgb(3, 0, 0),
            "text_muted draws in ink_dim()"
        );
        let accent = |p: &Probe, dl: &mut DrawList| {
            p.text_accent(dl, &fonts, at, "x", TextRole::Body, Emboss::Raised);
        };
        assert_eq!(
            ink_of(&accent),
            Rgba::rgb(1, 0, 0),
            "text_accent draws in accent()"
        );
    }

    /// The three rect-taking text helpers, on a real font: a caption cuts to its
    /// box with an ASCII `...` and paints inside a clip of that box; a note wraps
    /// to the width it is given and reports the height it drew; and a top-anchored
    /// line sits a cell below its anchor rather than on it.
    #[test]
    fn text_in_a_rect_fits_wraps_and_anchors() {
        let mut fonts = Fonts::new();
        let font = fonts
            .add(include_bytes!("../assets/DejaVuSans.ttf").to_vec())
            .unwrap();
        let g = Gunmetal::new(font);
        let ink = Rgba::rgb(9, 9, 9);
        let role = TextRole::Body;
        let px = g.font_px(role);
        let long = "a rather long caption that will not fit";

        // Ellipsized: whole when it fits, marked when it does not, and never
        // wider than the budget.
        let whole = fonts.measure(font, long, px);
        assert_eq!(g.ellipsized(&fonts, long, role, whole + 1.0), long);
        let cut = g.ellipsized(&fonts, long, role, whole * 0.5);
        assert!(cut.ends_with("..."), "the cut is marked: {cut:?}");
        assert!(
            long.starts_with(&cut[..cut.len() - 3]),
            "an in-order prefix"
        );
        assert!(
            fonts.measure(font, &cut, px) <= whole * 0.5 + 0.5,
            "inside the budget"
        );

        // `text_fit` draws that string, and only inside its own box.
        let box_ = Rect::new(10.0, 20.0, whole * 0.5 + 8.0, 20.0);
        let mut dl = DrawList::new();
        g.text_fit(
            &mut dl,
            &fonts,
            box_,
            4.0,
            long,
            role,
            Emboss::Engraved,
            ink,
        );
        assert!(
            matches!(dl.cmds.first(), Some(DrawCmd::PushClip(r)) if *r == box_),
            "the caption is clipped to its box"
        );
        assert!(matches!(dl.cmds.last(), Some(DrawCmd::PopClip)));

        // `text_wrapped` reports what it drew: one line's worth per wrapped line.
        let lh = fonts.line_height(font, px);
        let wide = Rect::new(0.0, 0.0, 4000.0, 100.0);
        let mut dl = DrawList::new();
        let one = g.text_wrapped(
            &mut dl,
            &fonts,
            wide,
            2.0,
            long,
            role,
            Emboss::Engraved,
            ink,
        );
        assert_eq!(one, lh, "it all fits on one line");
        let narrow = Rect::new(0.0, 0.0, whole * 0.5, 100.0);
        let mut dl = DrawList::new();
        let many = g.text_wrapped(
            &mut dl,
            &fonts,
            narrow,
            2.0,
            long,
            role,
            Emboss::Engraved,
            ink,
        );
        assert!(
            many > one,
            "a narrower box takes more lines ({many} vs {one})"
        );
        assert_eq!(many % lh, 0.0, "and the height is whole lines");

        // `text_top` anchors by the cell's top-left: its baseline sits a cell
        // below, where a baseline call would have put the ink on the anchor.
        let mut top = DrawList::new();
        g.text_top(
            &mut top,
            &fonts,
            Vec2::new(5.0, 7.0),
            "x",
            role,
            Emboss::Engraved,
            ink,
        );
        let mut base = DrawList::new();
        g.text_colored(
            &mut base,
            &fonts,
            Vec2::new(5.0, 7.0 + px * 0.84),
            "x",
            role,
            Emboss::Engraved,
            ink,
        );
        assert_eq!(format!("{:?}", top.cmds), format!("{:?}", base.cmds));
    }

    #[test]
    fn surface_fills_and_bevel_rings() {
        use crate::draw::DrawCmd;
        let g = Gunmetal::new(FontId(0));
        let r = Rect::new(0.0, 0.0, 20.0, 20.0);

        // `surface` is a single material fill (no border ring).
        let mut dl = DrawList::new();
        g.surface(&mut dl, r);
        assert!(
            matches!(dl.cmds.as_slice(), [DrawCmd::Solid { .. }]),
            "surface is one fill, no bevel"
        );

        // A raised/inset bevel is four edge quads; a flat bevel draws nothing.
        let mut raised = DrawList::new();
        g.bevel(&mut raised, r, Bevel::Raised, 1.0);
        assert_eq!(raised.cmds.len(), 4, "a raised bevel is four edges");
        let mut flat = DrawList::new();
        g.bevel(&mut flat, r, Bevel::Flat, 1.0);
        assert!(flat.is_empty(), "a flat bevel is empty");
    }
}
