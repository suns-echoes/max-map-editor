//! A `wgpu-ui` [`Theme`] that paints with the editor's brushed-steel skin, so the
//! toolkit's dialogs read as the same machined surface as the rest of the chrome.
//!
//! It carries `crate::theme`'s steel compositing - `base` fill, a tinted
//! exposure of the steel sheet, and a directional bevel - as `wgpu-ui`
//! `DrawList` commands. (It once *mirrored* a parallel app-side quad collector;
//! that collector is gone, and this is now the only steel painter.) The sheet is
//! the same `resources/images/steel.png`, registered as a `wgpu-ui` texture and
//! sampled stretched across the viewport (one continuous grain).
//!
//! Colors come from `crate::theme` (authored linear); `wgpu-ui` colors are
//! 8-bit sRGB decoded to linear in its shader, so each is sRGB-encoded here. The
//! steel grain multipliers `> 1` (which lighten in the editor's linear HDR
//! compositing) clamp to `1.0`, an unavoidable 8-bit approximation; the flat
//! base tone carries the hue.

use wgpu_ui::{
	Bevel, DrawList, Emboss, FontId, Fonts, Metrics, Rect, Rgba, Role, TexRect, TextRole, TextureId, Theme, Vec2,
};
use wgpu_ui::{DrawCtx, LayoutCtx};
use wgpu_ui::{Insets, Size, Stack, Widget, WidgetState};

use crate::theme as ed;
use crate::ui::SteelMap;

/// How a [`SteelTheme`] samples the steel sheet for a frame.
#[derive(Clone, Copy)]
enum SteelSample {
	/// Viewport-density grain measured from `origin` (the window's top-left), so
	/// it stays fixed to the window as it moves. The dialogs, menu, tabs and
	/// status bar use this (origin `(0,0)` ≡ the whole-viewport plate).
	Origin(Vec2),
	/// The editor's own [`SteelMap`] - the exact mapping the native renderer uses
	/// for a panel's *content*, so migrated panel chrome shares one continuous
	/// grain with that content (docked: stretched plate; floating: anchored crop).
	Map(SteelMap),
}

/// The MAX font's design cell in font units (cap-top to descender), the span the
/// editor maps to the nominal text px - and, since U7.1, the **only** place that
/// conversion is written down.
/// The editor's chrome sizes text by that cell (`scale = px / DESIGN_CELL`), but
/// wgpu-ui sizes by the font's em (`px / units_per_em`). To render at the *same*
/// size as the chrome (so it isn't a different, "stretched"-looking scale), the
/// theme multiplies every text px by `units_per_em / DESIGN_CELL`.
///
/// `MAX_Redesign_Square.ttf` draws on a 64-unit em: cap-top 48, descender -12.
/// That is the same **0.9375 em** the 4096-unit `max_square.ttf` used (3840 of
/// 4096), and every advance matches to the unit, so the swap to the redesigned
/// face is metrically invisible - `font_scale` stays 1.0667 and no layout moves.
/// What changes is the outlines: the old face was a pixel-art trace, every point
/// on a 16-cell grid with not one off-curve point, and it could only be crisp at
/// a raster em divisible by 16 (see `crate::fontprobe`). This one has real
/// curves, so it rasterizes cleanly at any size.
const DESIGN_CELL_FU: f32 = 60.0;

/// sRGB-encode a linear channel to a 0..=255 byte.
fn enc(c: f32) -> u8 {
	let c = c.clamp(0.0, 1.0);
	let s = if c <= 0.003_130_8 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
	(s * 255.0).round() as u8
}

/// Tint a material's `base` toward `accent` (linear rgb) by `floor`, in LINEAR
/// light (docs/ui/theme.md §4.3): `base·(1 - f·(1 - a)) + f·a` per channel,
/// clamped. The steel `grain` is unchanged, so the machined look survives.
fn accent_tint(m: ed::Material, accent: [f32; 3], floor: f32) -> ed::Material {
	let b = m.base;
	let t = |i: usize| (b[i] * (1.0 - floor * (1.0 - accent[i])) + floor * accent[i]).clamp(0.0, 1.0);
	ed::Material { base: [t(0), t(1), t(2), b[3]], grain: m.grain }
}

/// An editor linear `[r,g,b,a]` as a `wgpu-ui` color (alpha is linear, no gamma).
/// Public so chrome that draws its own `DrawList` (the menu bar) can encode the
/// editor's `theme` colours the same way the `SteelTheme` does.
pub fn rgba(c: [f32; 4]) -> Rgba {
	Rgba::rgba(enc(c[0]), enc(c[1]), enc(c[2]), (c[3].clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Which edge of a shell strip carries its lit seam - see
/// [`SteelTheme::seam`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
	Top,
	Bottom,
}

pub struct SteelTheme {
	font: FontId,
	/// The monospace face for [`TextRole::Mono`] (the console), if the host
	/// registered one — see [`SteelTheme::with_mono`]. `None` falls back to the
	/// MAX chrome font, so a theme built without it still renders every role.
	mono: Option<FontId>,
	steel: TextureId,
	/// Logical viewport, set each frame; sets the steel grain's scale (density).
	viewport: (f32, f32),
	/// How this frame samples the steel sheet - an origin-anchored viewport plate
	/// (dialogs/menu) or the editor's [`SteelMap`] (panel chrome). Set each frame
	/// via [`set_origin`](Self::set_origin) / [`set_steel_map`](Self::set_steel_map).
	sample: SteelSample,
	/// `units_per_em / DESIGN_CELL_FU` - multiplies every text px so wgpu-ui
	/// renders the chrome face at the editor's design-cell scale (matching the
	/// prerendered chrome). See [`DESIGN_CELL_FU`].
	font_scale: f32,
	/// The UI logical→physical scale, set each frame; the text emboss offsets by
	/// [`wgpu_ui::emboss_offset`] so the engraved edge is a whole number of
	/// *physical* px, clear of the glyph's own anti-aliased fringe.
	scale: f32,
}

impl SteelTheme {
	/// `em` is the font's `units_per_em` (read from the parsed font), used to
	/// match the editor's design-cell text scale - see [`DESIGN_CELL_FU`].
	pub fn new(font: FontId, steel: TextureId, em: u16) -> Self {
		Self {
			font,
			mono: None,
			steel,
			viewport: (1.0, 1.0),
			sample: SteelSample::Origin(Vec2::new(0.0, 0.0)),
			font_scale: em as f32 / DESIGN_CELL_FU,
			scale: 1.0,
		}
	}

	/// Names the monospace face [`TextRole::Mono`] draws and measures with —
	/// register it in the same [`Fonts`] the chrome renders from. The console is
	/// its one consumer: fixed-pitch text is what makes a terminal's columns line
	/// up, and it is the reason the console can be an ordinary `TextInput` at all
	/// (U4.4).
	pub fn with_mono(mut self, mono: FontId) -> Self {
		self.mono = Some(mono);
		self
	}

	/// Sets the UI logical→physical scale (call each frame before `Ui::draw`), so
	/// the text emboss lands on whole physical pixels.
	pub fn set_scale(&mut self, scale: f32) {
		self.scale = scale.max(1e-4);
	}

	/// Updates the logical viewport used to scale the steel grain (call before
	/// `Ui::draw`).
	pub fn set_viewport(&mut self, w: f32, h: f32) {
		self.viewport = (w.max(1.0), h.max(1.0));
	}

	/// Anchors the steel grain to `origin` (the window's top-left), so the
	/// background stays fixed to the window as it moves. Set each frame after
	/// layout, before `Ui::draw`. Selects the origin-anchored viewport plate
	/// (the dialog / menu / tab / status path).
	pub fn set_origin(&mut self, origin: Vec2) {
		self.sample = SteelSample::Origin(origin);
	}

	/// Samples the sheet through the editor's [`SteelMap`] - the *same* mapping
	/// the native renderer uses for the panel's content - so migrated panel
	/// chrome and that content share one continuous grain. Set each frame before
	/// building a panel's chrome `DrawList`.
	pub fn set_steel_map(&mut self, map: SteelMap) {
		self.sample = SteelSample::Map(map);
	}

	/// Steel UVs for a screen rect under the frame's [`SteelSample`]: an
	/// origin-measured viewport plate, or the editor's [`SteelMap`] (shared with
	/// native content) - so neighbouring elements share one continuous grain.
	fn uv(&self, r: Rect) -> TexRect {
		let (vw, vh) = self.viewport;
		match self.sample {
			SteelSample::Origin(o) => {
				TexRect::new((r.x - o.x) / vw, (r.y - o.y) / vh, (r.right() - o.x) / vw, (r.bottom() - o.y) / vh)
			}
			SteelSample::Map(map) => {
				let [u0, v0, u1, v1] = map.uv(crate::ui::Rect::new(r.x, r.y, r.w, r.h), vw, vh);
				TexRect::new(u0, v0, u1, v1)
			}
		}
	}

	/// A material fill: flat base tone, then the tinted steel grain over it.
	/// Public for chrome that draws its own `DrawList` (the menu bar).
	pub fn material(&self, dl: &mut DrawList, r: Rect, m: ed::Material) {
		dl.fill_rect(r, rgba(m.base));
		dl.image(self.steel, r, self.uv(r), rgba(m.grain));
	}

	/// The raw steel sheet sampled across `r` at `tint` (no base tone) - the app
	/// background plate behind the map.
	pub fn steel_fill(&self, dl: &mut DrawList, r: Rect, tint: [f32; 4]) {
		dl.image(self.steel, r, self.uv(r), rgba(tint));
	}

	/// A `size`-px directional bevel ring (lit top-left when `raised`, swapped
	/// for inset wells), each edge alpha-blended over the fill.
	pub fn bevel(&self, dl: &mut DrawList, r: Rect, size: f32, raised: bool) {
		let b = ed::BEVEL;
		let (te, le, be, re) =
			if raised { (b.top, b.left, b.bottom, b.right) } else { (b.bottom, b.right, b.top, b.left) };
		dl.fill_rect(Rect::new(r.x, r.y, r.w, size), rgba(te));
		dl.fill_rect(Rect::new(r.x, r.bottom() - size, r.w, size), rgba(be));
		dl.fill_rect(Rect::new(r.x, r.y + size, size, r.h - 2.0 * size), rgba(le));
		dl.fill_rect(Rect::new(r.right() - size, r.y + size, size, r.h - 2.0 * size), rgba(re));
	}

	/// The 1px lit seam along one edge of a full-width **shell strip** - the tab
	/// strip's bottom, the status bar's top: the line that separates the chrome
	/// band from the map beside it. The two strips draw different materials by
	/// design (a panel plate under the tabs, the header band under the status
	/// line), but the seam is one look, and it is the skin's, not the shell's.
	pub fn seam(&self, dl: &mut DrawList, r: Rect, edge: Edge) {
		let (y, ink) = match edge {
			Edge::Top => (r.y, ed::BEVEL.top),
			Edge::Bottom => (r.bottom() - 1.0, ed::BEVEL.bottom),
		};
		dl.fill_rect(Rect::new(r.x, y, r.w, 1.0), rgba(ink));
	}

	/// Material fill + a raised (outset) bevel ring. Public for the menu bar.
	pub fn raised(&self, dl: &mut DrawList, r: Rect, m: ed::Material, size: f32) {
		self.material(dl, r, m);
		self.bevel(dl, r, size, true);
	}

	/// Material fill + an inset bevel ring (a well). Public for the menu bar.
	pub fn inset(&self, dl: &mut DrawList, r: Rect, m: ed::Material, size: f32) {
		self.material(dl, r, m);
		self.bevel(dl, r, size, false);
	}

	/// A state-highlight fill: `surface` tinted toward `accent` by `floor`
	/// (docs/ui/theme.md §4.3) - a hovered / active row or item owns a tinted crop
	/// of the steel, not a translucent wash. `material` samples the sheet by screen
	/// position, so the tinted crop's grain lines up with the untinted surface it
	/// sits in (the menu / list it's a row of).
	pub fn accent_fill(&self, dl: &mut DrawList, rect: Rect, surface: ed::Material, accent: [f32; 3], floor: f32) {
		self.material(dl, rect, accent_tint(surface, accent, floor));
	}

	/// Embossed text with an explicit `color` (the shared engraving used by the
	/// dialogs and the menu bar): a 1-physical-px shadow (down-right) always, a
	/// hilite (up-left) only for [`Emboss::Raised`], then the ink in `color`.
	/// Returns the advance width.
	pub fn emboss_text(
		&self,
		dl: &mut DrawList,
		fonts: &Fonts,
		baseline: Vec2,
		s: &str,
		px: f32,
		color: Rgba,
		emboss: Emboss,
	) -> f32 {
		Theme::text_run(self, dl, fonts, self.font, baseline, s, px, emboss, color)
	}
}

impl Theme for SteelTheme {
	fn as_any(&self) -> &dyn std::any::Any {
		self
	}

	fn metrics(&self) -> Metrics {
		Metrics {
			pad: 8.0,
			gap: 6.0,
			bevel: 1.0,
			scrollbar: 8.0,
			// The editor's bars have always floored the thumb at 16px, not the
			// toolkit's 24 - short panel bodies get a thumb, not a full track.
			scrollbar_min_thumb: 16.0,
			// Title text (16px) with 2px more breathing room above and below.
			titlebar: 26.0,
			modal_frame: 2.0,
			control_height: 24.0,
			button_min_width: 72.0,
			font_body: 16.0,
			font_small: 12.0,
			font_title: 16.0,
			// The console's face: ~9.6px advance and an ~18.6px line at 16px,
			// within a pixel of the 10x19 bitmap cell it replaced (U4.4).
			font_mono: 16.0,
		}
	}

	fn font(&self) -> FontId {
		self.font
	}

	fn font_for(&self, role: TextRole) -> FontId {
		match role {
			TextRole::Mono => self.mono.unwrap_or(self.font),
			_ => self.font,
		}
	}

	/// Text px scaled to the editor's design-cell, so the dialog font renders at
	/// the same size as the prerendered chrome (see [`DESIGN_CELL_FU`]). Drives
	/// both measurement and drawing, so layout stays consistent.
	fn font_px(&self, role: TextRole) -> f32 {
		let m = self.metrics();
		// The monospace face is an ordinary TTF sized by its own em, so it takes
		// the px straight - `font_scale` corrects the MAX face's design cell and
		// would shrink Hack to nothing.
		if role == TextRole::Mono {
			return m.font_mono;
		}
		let base = match role {
			TextRole::Body => m.font_body,
			TextRole::Small => m.font_small,
			TextRole::Title => m.font_title,
			TextRole::Mono => unreachable!("handled above"),
		};
		base * self.font_scale
	}

	fn accent(&self) -> Rgba {
		rgba(ed::ACCENT)
	}

	fn ink(&self) -> Rgba {
		rgba(ed::INK)
	}

	fn ink_dim(&self) -> Rgba {
		rgba(ed::INK_DIM)
	}

	fn panel(&self, dl: &mut DrawList, rect: Rect) {
		// The window's only frame is the 2px raised (outset) bevel ring - bright
		// top, brighter left, dark bottom, darker right. No extra border over it;
		// the steel fill is continuous under it and fixed to the window (`uv`).
		self.raised(dl, rect, ed::PANEL, 2.0);
	}

	fn popup(&self, dl: &mut DrawList, rect: Rect) {
		// A floating list/menu: the same steel panel material, but a lighter 1px
		// raised (outset) bevel frame - a window is 2px, a popup is 1px.
		self.raised(dl, rect, ed::PANEL, 1.0);
	}

	fn titlebar(&self, dl: &mut DrawList, rect: Rect) {
		// A rusted-steel band, inset by the 2px window bevel so the outset frame
		// still rings the titlebar (top/left/right). A 1px darkened-steel seam
		// at the bottom is the recessed border between titlebar and content.
		let b = 2.0;
		let band = Rect::new(rect.x + b, rect.y + b, (rect.w - 2.0 * b).max(0.0), (rect.h - b).max(0.0));
		self.material(dl, band, ed::RUST_TITLE);
		let seam = Rect::new(band.x, band.bottom() - 1.0, band.w, 1.0);
		self.material(dl, seam, ed::RUST_EDGE);
	}

	fn frame(&self, dl: &mut DrawList, rect: Rect, fill: Rgba, bevel: Bevel) {
		dl.fill_rect(rect, fill);
		match bevel {
			Bevel::Raised => self.bevel(dl, rect, 1.0, true),
			Bevel::Inset => self.bevel(dl, rect, 1.0, false),
			Bevel::Flat => {}
		}
	}

	/// The trait-level sized bevel: a custom widget requests the steel edge
	/// treatment at any weight through `&dyn Theme` (1px controls, 2px windows)
	/// instead of downcasting for the inherent [`SteelTheme::bevel`].
	fn bevel(&self, dl: &mut DrawList, rect: Rect, kind: Bevel, px: f32) {
		match kind {
			Bevel::Raised => SteelTheme::bevel(self, dl, rect, px, true),
			Bevel::Inset => SteelTheme::bevel(self, dl, rect, px, false),
			Bevel::Flat => {}
		}
	}

	/// The panel-header / status strip: the TITLE-tinted steel material — the
	/// band the toolbox/units/minimap headers and the status bar sit on.
	fn header_band(&self, dl: &mut DrawList, rect: Rect) {
		self.material(dl, rect, ed::TITLE);
	}

	/// Separators are engraved, not painted: the 1px inset ring of a 2px band
	/// reads as a groove cut into the steel (dark over light) — a flat bright
	/// line would sit *on* the material.
	fn separator(&self, dl: &mut DrawList, rect: Rect) {
		let y = (rect.center().y - 1.0).floor();
		SteelTheme::bevel(self, dl, Rect::new(rect.x, y, rect.w, 2.0), 1.0, false);
	}

	/// The same engraved groove cut top-to-bottom (menu column rules).
	fn vseparator(&self, dl: &mut DrawList, rect: Rect) {
		let x = (rect.center().x - 1.0).floor();
		SteelTheme::bevel(self, dl, Rect::new(x, rect.y, 2.0, rect.h), 1.0, false);
	}

	fn button(&self, dl: &mut DrawList, rect: Rect, role: Role, state: WidgetState) {
		// Rest surface + this face's accent (docs/ui/theme.md §4.2): a Neutral
		// (non-CTA) face hovers with the Primary green tint; each CTA face rests on
		// its own material and hovers with its own accent; a selected/active face
		// is green.
		let (rest, accent) = if state.disabled {
			(ed::BUTTON_DISABLED, ed::ACCENT_PRIMARY)
		} else if state.selected {
			(ed::BUTTON_ACTIVE, ed::ACCENT_PRIMARY)
		} else {
			match role {
				Role::Neutral => (ed::BUTTON, ed::ACCENT_PRIMARY),
				Role::Primary => (ed::BUTTON_PRIMARY, ed::ACCENT_PRIMARY),
				Role::Secondary => (ed::BUTTON_SECONDARY, ed::ACCENT_SECONDARY),
				Role::Danger => (ed::BUTTON_DANGER, ed::ACCENT_DANGER),
			}
		};
		let pressed = state.pressed && !state.disabled;
		let active = state.selected && !state.disabled;
		// Hover OR press (the pointer is over the face) tints toward the accent;
		// press additionally sinks the bevel (raised -> inset). No darken wash.
		// An active face hovers at the stronger active floor, so a lit toggle
		// still answers the pointer over its already-green body.
		let lit = !state.disabled && (state.hovered || pressed);
		let floor = if active { ed::FLOOR_ACTIVE_HOVER } else { ed::FLOOR_HOVER };
		let m = if lit { accent_tint(rest, accent, floor) } else { rest };
		self.material(dl, rect, m);
		// A toggled-on key sits *down*: its bevel sinks like a held press, so an
		// active tool reads as the key currently pushed in, not merely tinted.
		self.bevel(dl, rect, 1.0, !(pressed || active));
		// Focus ring: 50% Primary-green, 1px, 1px outside the control (§4.4).
		if state.focused && !state.disabled {
			let ring = rect.inset(Insets::all(-1.0));
			dl.stroke_rect(ring, 1.0, rgba([ed::ACCENT[0], ed::ACCENT[1], ed::ACCENT[2], 0.5]));
		}
	}

	fn well(&self, dl: &mut DrawList, rect: Rect, state: WidgetState) {
		self.inset(dl, rect, ed::TEXTAREA, 1.0);
		if state.focused {
			dl.stroke_rect(rect, 1.0, rgba(ed::ACCENT));
		}
	}

	/// The editor's bar, not the toolkit's default `well` + `button`: a flat
	/// translucent track under a solid 1px-inset thumb that lightens on hover and
	/// again while dragged. This is `kit::scrollbar` moved behind the trait, so
	/// every `Scroller` in the app - panel bodies, and the toolkit's own
	/// `ScrollArea`/`TextArea` - paints the one bar the editor has always drawn.
	fn scrollbar(&self, dl: &mut DrawList, track: Rect, thumb: Rect, state: WidgetState) {
		dl.fill_rect(track, rgba(ed::SCROLL_TRACK));
		let face = if state.pressed {
			ed::SCROLL_THUMB_DRAG
		} else if state.hovered {
			ed::SCROLL_THUMB_HOVER
		} else {
			ed::SCROLL_THUMB
		};
		dl.fill_rect(thumb.inset(Insets::all(1.0)), rgba(face));
	}

	fn accent_row(&self, dl: &mut DrawList, rect: Rect, floor: f32) {
		// A tinted crop of the popup/list surface (PANEL), not a translucent wash -
		// grain stays continuous with the list (docs/ui/theme.md §4).
		self.accent_fill(dl, rect, ed::PANEL, ed::ACCENT_PRIMARY, floor);
	}

	fn accent_well_row(&self, dl: &mut DrawList, rect: Rect, floor: f32) {
		// The same §4 rule inside a text well: the row is a tinted crop of the
		// darker TEXTAREA material (the palette panel's saved-list rows).
		self.accent_fill(dl, rect, ed::TEXTAREA, ed::ACCENT_PRIMARY, floor);
	}

	fn surface(&self, dl: &mut DrawList, rect: Rect) {
		// The bare panel material, no frame - the trait-level fill the panel
		// widgets use where they used to downcast for `material(PANEL)`.
		self.material(dl, rect, ed::PANEL);
	}

	fn text_em(
		&self,
		dl: &mut DrawList,
		fonts: &Fonts,
		baseline: Vec2,
		s: &str,
		role: TextRole,
		emboss: Emboss,
	) -> f32 {
		// Titles in amber (the shared titlebar ink), body in ink, small/hint text
		// dim - matching the editor's chrome-label colours. The engraving itself
		// lives in `emboss_text`.
		let ink = match role {
			TextRole::Title => ed::TITLE_INK,
			TextRole::Body | TextRole::Mono => ed::INK,
			TextRole::Small => ed::INK_DIM,
		};
		self.text_run(dl, fonts, self.font_for(role), baseline, s, self.font_px(role), emboss, rgba(ink))
	}

	/// Arbitrary-ink emboss text (a custom chrome widget's escape hatch): draw
	/// `s` at the role's px in exactly `ink`, with the role's font and the shared
	/// engraving. The tab strip uses it for per-tab inks (active accent, inactive
	/// dim, the close-`×` tint) that the role-based [`text_em`] doesn't cover.
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
		self.text_run(dl, fonts, self.font_for(role), baseline, s, self.font_px(role), emboss, ink)
	}

	/// The editor's engraving, at an explicit face and em size — the one place
	/// the steel shadow/hilite inks are drawn, for both the role path above and
	/// a content widget sizing text from its own domain. A 1-physical-px shadow
	/// (down-right) always, a hilite (up-left) only for [`Emboss::Raised`],
	/// nothing at all for [`Emboss::Flat`], then the ink.
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
			let o = wgpu_ui::emboss_offset(self.scale);
			wgpu_ui::text::draw_line(dl, fonts, font, s, baseline + Vec2::new(o, o), px, rgba(ed::TEXT_SHADOW));
			if emboss == Emboss::Raised {
				wgpu_ui::text::draw_line(dl, fonts, font, s, baseline - Vec2::new(o, o), px, rgba(ed::TEXT_HILITE));
			}
		}
		wgpu_ui::text::draw_line(dl, fonts, font, s, baseline, px, ink)
	}
}

/// A themed chrome box behind a single padded child (the credits box, the
/// New Map pick lists and items) - `wgpu-ui` has no well container, so the
/// dialog provides one. Inset (a well) by default; [`raised`](Self::raised)
/// flips it to an outset plate.
pub struct Well {
	inner: Stack,
	padding: Insets,
	/// Black wash alpha over the well fill (0 = the plain well material).
	shade: u8,
	/// An explicit colour fill over the well interior (e.g. a dark-red alert
	/// ground). Takes precedence over [`Self::shade`]. Well mode only.
	wash: Option<Rgba>,
	/// Outset plate (surface + raised bevel) instead of the inset well.
	raised: bool,
	rect: Rect,
}

impl Well {
	pub fn new(child: impl Widget + 'static) -> Self {
		Self {
			inner: Stack::new().push(child),
			padding: Insets::all(6.0),
			shade: 0,
			wash: None,
			raised: false,
			rect: Rect::ZERO,
		}
	}

	/// Overrides the content padding (default 6px each side).
	pub fn padding(mut self, px: f32) -> Self {
		self.padding = Insets::all(px);
		self
	}

	/// Washes the well fill toward black by `alpha`/255 (kept off the 1px
	/// bevel ring, so the engraving stays crisp). Well mode only.
	pub fn shaded(mut self, alpha: u8) -> Self {
		self.shade = alpha;
		self
	}

	/// Fills the well interior with `color` (over the 1px bevel ring's inside) —
	/// a coloured alert ground (dark red) rather than the plain steel/black wash.
	/// Well mode only; supersedes [`Self::shaded`].
	pub fn wash(mut self, color: Rgba) -> Self {
		self.wash = Some(color);
		self
	}

	/// An outset plate (the theme surface under a raised bevel) instead of
	/// the inset well - list items that should sit proud of a recessed list.
	pub fn raised(mut self) -> Self {
		self.raised = true;
		self
	}
}

impl Widget for Well {
	fn measure(&mut self, avail: Size, ctx: &mut LayoutCtx) -> Size {
		let inner =
			Size::new((avail.w - self.padding.horizontal()).max(0.0), (avail.h - self.padding.vertical()).max(0.0));
		let c = self.inner.measure(inner, ctx);
		Size::new(c.w + self.padding.horizontal(), c.h + self.padding.vertical())
	}

	fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
		self.rect = rect;
		self.inner.arrange(rect.inset(self.padding), ctx);
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		if ctx.is_base() {
			if self.raised {
				ctx.theme.surface(dl, self.rect);
				ctx.theme.bevel(dl, self.rect, Bevel::Raised, 1.0);
			} else {
				ctx.theme.well(dl, self.rect, WidgetState::default());
				if let Some(color) = self.wash {
					dl.fill_rect(self.rect.inset(Insets::all(1.0)), color);
				} else if self.shade > 0 {
					dl.fill_rect(self.rect.inset(Insets::all(1.0)), Rgba::rgba(0, 0, 0, self.shade));
				}
			}
		}
		self.inner.draw(dl, ctx);
	}

	// Chrome only: interaction belongs to the content (the trait defaults
	// neither forward events nor recurse hit-tests, which would dead-zone
	// everything inside).
	fn event(&mut self, ev: &wgpu_ui::Event, ctx: &mut wgpu_ui::EventCtx) -> bool {
		self.inner.event(ev, ctx)
	}

	fn hit_test(&self, pos: Vec2) -> Option<wgpu_ui::WidgetId> {
		self.inner.hit_test(pos)
	}

	fn rect(&self) -> Rect {
		self.rect
	}

	fn child_count(&self) -> usize {
		1
	}

	fn child(&self, i: usize) -> Option<&dyn Widget> {
		(i == 0).then_some(&self.inner as &dyn Widget)
	}

	fn child_mut(&mut self, i: usize) -> Option<&mut dyn Widget> {
		(i == 0).then_some(&mut self.inner as &mut dyn Widget)
	}
}

/// A 1px inset ring drawn *over* its child's edge - a preview tile recessed
/// into the steel. (A [`Well`] paints its chrome before the content, so an
/// opaque image would cover the ring; here the ring engraves the image edge.)
pub struct InsetFrame {
	inner: Stack,
	rect: Rect,
}

impl InsetFrame {
	pub fn new(child: impl Widget + 'static) -> Self {
		Self { inner: Stack::new().push(child), rect: Rect::ZERO }
	}
}

impl Widget for InsetFrame {
	fn measure(&mut self, avail: Size, ctx: &mut LayoutCtx) -> Size {
		self.inner.measure(avail, ctx)
	}

	fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
		self.rect = rect;
		self.inner.arrange(rect, ctx);
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		self.inner.draw(dl, ctx);
		if ctx.is_base() {
			ctx.theme.bevel(dl, self.rect, Bevel::Inset, 1.0);
		}
	}

	// Chrome only, like `Well`: forward interaction to the content.
	fn event(&mut self, ev: &wgpu_ui::Event, ctx: &mut wgpu_ui::EventCtx) -> bool {
		self.inner.event(ev, ctx)
	}

	fn hit_test(&self, pos: Vec2) -> Option<wgpu_ui::WidgetId> {
		self.inner.hit_test(pos)
	}

	fn rect(&self) -> Rect {
		self.rect
	}

	fn child_count(&self) -> usize {
		1
	}

	fn child(&self, i: usize) -> Option<&dyn Widget> {
		(i == 0).then_some(&self.inner as &dyn Widget)
	}

	fn child_mut(&mut self, i: usize) -> Option<&mut dyn Widget> {
		(i == 0).then_some(&mut self.inner as &mut dyn Widget)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use wgpu_ui::{DrawCmd, WidgetId};

	/// The parsed MAX font + a steel theme with no GPU behind it (the built-in
	/// atlas texture id) - enough for geometry/colour asserts on `DrawList`
	/// commands and for driving the layout contexts.
	fn setup() -> (Fonts, SteelTheme) {
		let mut fonts = Fonts::new();
		let font =
			fonts.add(include_bytes!("../assets/MAX_Redesign_Square.ttf").to_vec()).expect("parse MAX_Redesign_Square");
		let em = fonts.get(font).units_per_em();
		let theme = SteelTheme::new(font, TextureId::ATLAS, em);
		(fonts, theme)
	}

	/// [`setup`] plus the monospace face the console draws in (U4.4).
	fn setup_mono() -> (Fonts, SteelTheme) {
		let (mut fonts, theme) = setup();
		let mono = fonts.add(include_bytes!("../assets/Hack-Regular.ttf").to_vec()).expect("parse Hack-Regular");
		(fonts, theme.with_mono(mono))
	}

	/// A fixed-size, identifiable leaf - the content stand-in for the chrome
	/// containers' geometry / hit-test contracts.
	struct Probe {
		id: WidgetId,
		size: Size,
		rect: Rect,
	}

	impl Widget for Probe {
		fn measure(&mut self, _avail: Size, _ctx: &mut LayoutCtx) -> Size {
			self.size
		}

		fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
			self.rect = rect;
		}

		fn draw(&self, _dl: &mut DrawList, _ctx: &DrawCtx) {}

		fn rect(&self) -> Rect {
			self.rect
		}

		fn id(&self) -> WidgetId {
			self.id
		}
	}

	/// `as_any` exposes the concrete `SteelTheme` through `&dyn Theme` - the
	/// escape hatch chrome uses to reach the inherent steel helpers.
	#[test]
	fn as_any_downcasts_to_the_concrete_theme() {
		let (_fonts, theme) = setup();
		let t: &dyn Theme = &theme;
		assert!(t.as_any().downcast_ref::<SteelTheme>().is_some(), "downcast to SteelTheme");
		assert!(t.as_any().downcast_ref::<u32>().is_none(), "wrong type stays None");
	}

	/// `frame` paints the fill exactly, then a 1px bevel whose lit edge flips
	/// with the direction - and `Bevel::Flat` adds (or, for the trait-level
	/// sized bevel, draws) nothing.
	#[test]
	fn frame_and_bevel_follow_the_direction() {
		let (_fonts, theme) = setup();
		let t: &dyn Theme = &theme;
		let r = Rect::new(2.0, 2.0, 40.0, 20.0);
		let fill = Rgba::rgba(10, 20, 30, 255);

		let mut flat = DrawList::new();
		t.frame(&mut flat, r, fill, Bevel::Flat);
		assert_eq!(flat.cmds.len(), 1, "flat = the fill only");
		assert!(matches!(flat.cmds[0], DrawCmd::Solid { rect, color } if rect == r && color == fill));

		let mut raised = DrawList::new();
		t.frame(&mut raised, r, fill, Bevel::Raised);
		assert_eq!(raised.cmds.len(), 5, "fill + 4 bevel edges");
		let mut inset = DrawList::new();
		t.frame(&mut inset, r, fill, Bevel::Inset);
		assert_eq!(inset.cmds.len(), 5);
		let top = |dl: &DrawList| match dl.cmds[1] {
			DrawCmd::Solid { color, .. } => color,
			_ => panic!("bevel edges are solid fills"),
		};
		assert_eq!(top(&raised), rgba(ed::BEVEL.top), "raised lights the top edge");
		assert_eq!(top(&inset), rgba(ed::BEVEL.bottom), "inset flips it dark");

		let mut none = DrawList::new();
		t.bevel(&mut none, r, Bevel::Flat, 2.0);
		assert!(none.is_empty(), "the sized trait bevel treats Flat as a no-op");
	}

	/// `vseparator` engraves a 2px groove at the row's horizontal centre
	/// (floored to the pixel grid), spanning its full height.
	#[test]
	fn vseparator_engraves_a_centered_groove() {
		let (_fonts, theme) = setup();
		let t: &dyn Theme = &theme;
		let r = Rect::new(10.0, 5.0, 9.0, 30.0);
		let x = (r.center().x - 1.0).floor();
		let mut dl = DrawList::new();
		t.vseparator(&mut dl, r);
		assert_eq!(dl.cmds.len(), 4, "the groove is a 1px inset ring of a 2px band");
		for c in &dl.cmds {
			let DrawCmd::Solid { rect, .. } = c else { panic!("groove edges are solid fills") };
			assert!(rect.x >= x && rect.right() <= x + 2.0, "stays inside the 2px column at {x}: {rect:?}");
			assert!(rect.y >= r.y && rect.bottom() <= r.bottom(), "stays inside the row: {rect:?}");
		}
	}

	/// `accent_well_row` is a tinted crop of the darker TEXTAREA material: the
	/// base tone moves toward the primary accent by the floor while the steel
	/// grain stays untouched (the machined look survives the highlight).
	#[test]
	fn accent_well_row_tints_base_not_grain() {
		let (_fonts, theme) = setup();
		let t: &dyn Theme = &theme;
		let r = Rect::new(0.0, 0.0, 50.0, 18.0);
		let mut dl = DrawList::new();
		t.accent_well_row(&mut dl, r, ed::FLOOR_HOVER);
		let want = accent_tint(ed::TEXTAREA, ed::ACCENT_PRIMARY, ed::FLOOR_HOVER);
		assert!(
			matches!(dl.cmds[0], DrawCmd::Solid { rect, color } if rect == r && color == rgba(want.base)),
			"the base is the §4.3 accent tint of TEXTAREA"
		);
		assert_ne!(rgba(want.base), rgba(ed::TEXTAREA.base), "the tint actually moved the base");
		assert!(
			matches!(dl.cmds[1], DrawCmd::Image { rect, color, .. } if rect == r && color == rgba(ed::TEXTAREA.grain)),
			"the grain is the untinted TEXTAREA grain"
		);
	}

	/// The chrome containers report their arranged rect, and `InsetFrame`
	/// forwards hit-tests to its content (chrome must never dead-zone it).
	#[test]
	fn well_and_inset_frame_geometry_and_hits() {
		let (fonts, theme) = setup();
		let mut ctx = LayoutCtx { fonts: &fonts, theme: &theme, scale: 1.0, viewport: wgpu_ui::Rect::ZERO };

		let mut well = Well::new(Probe { id: WidgetId(7), size: Size::new(30.0, 10.0), rect: Rect::ZERO });
		let m = well.measure(Size::new(200.0, 100.0), &mut ctx);
		assert_eq!((m.w, m.h), (42.0, 22.0), "content + the default 6px padding per side");
		let r = Rect::new(5.0, 8.0, 60.0, 30.0);
		well.arrange(r, &mut ctx);
		assert_eq!(well.rect(), r, "Well reports its arranged rect");

		let mut frame = InsetFrame::new(Probe { id: WidgetId(9), size: Size::new(20.0, 20.0), rect: Rect::ZERO });
		let _ = frame.measure(Size::new(100.0, 100.0), &mut ctx);
		let fr = Rect::new(40.0, 40.0, 20.0, 20.0);
		frame.arrange(fr, &mut ctx);
		assert_eq!(frame.rect(), fr, "InsetFrame reports its arranged rect");
		assert_eq!(frame.hit_test(Vec2::new(50.0, 50.0)), Some(WidgetId(9)), "hits forward to the content");
		assert_eq!(frame.hit_test(Vec2::new(5.0, 5.0)), None, "outside the content: no hit");
	}

	/// U4.4: `TextRole::Mono` resolves the *monospace* face, and takes its px
	/// straight — `font_scale` is the MAX face's design-cell correction and would
	/// shrink an ordinary TTF to nothing. Fixed pitch is the whole point: every
	/// glyph in a console line has to advance by the same width.
	#[test]
	fn the_mono_role_uses_the_monospace_face_at_a_plain_px() {
		let (fonts, theme) = setup_mono();
		assert_ne!(theme.font_for(TextRole::Mono), theme.font(), "the mono role names the second face");
		assert_eq!(theme.font_for(TextRole::Body), theme.font(), "every other role keeps the chrome font");
		assert_eq!(theme.font_px(TextRole::Mono), theme.metrics().font_mono, "no design-cell scaling");
		assert_ne!(theme.font_px(TextRole::Body), theme.metrics().font_body, "...which body text still gets");

		// Fixed pitch: `i` and `W` advance identically in the mono face, and do
		// not in the chrome one.
		let mono = fonts.get(theme.font_for(TextRole::Mono));
		let px = theme.font_px(TextRole::Mono);
		assert_eq!(mono.measure("i", px), mono.measure("W", px), "the mono face is fixed pitch");
		assert_eq!(mono.measure("iiii", px), 4.0 * mono.measure("i", px), "and advances linearly");

		// A theme built without a mono face still renders the role (chrome font).
		let (_f, plain) = setup();
		assert_eq!(plain.font_for(TextRole::Mono), plain.font(), "no second face: fall back, never panic");
	}

	/// The role reaches *drawing*, not just measurement: a mono run is emitted in
	/// the mono face at the mono px.
	#[test]
	fn a_mono_run_draws_in_the_monospace_face() {
		let (fonts, theme) = setup_mono();
		let mut dl = DrawList::new();
		theme.text(&mut dl, &fonts, Vec2::new(0.0, 20.0), "] fit", TextRole::Mono);
		let faces: Vec<_> = dl
			.cmds
			.iter()
			.filter_map(|c| match c {
				DrawCmd::Glyph { font, px, .. } => Some((*font, *px)),
				_ => None,
			})
			.collect();
		assert!(!faces.is_empty(), "the run drew glyphs");
		assert!(
			faces.iter().all(|&(f, px)| f == theme.font_for(TextRole::Mono) && px == theme.font_px(TextRole::Mono)),
			"every glyph of a mono run is in the mono face, got {faces:?}",
		);
	}
	/// The scrollbar the panels draw is `SteelTheme::scrollbar`, over a
	/// `wgpu_ui::Scroller`'s geometry: no bar when the content fits, a flat track
	/// under a 1px-inset thumb whose colour tracks rest / hover / drag. (It moved
	/// here from `uikit_draw`'s tests with U6.3 - it always tested the theme.)
	#[test]
	fn scrollbar_states_pick_the_documented_colors() {
		use crate::theme;
		use crate::ui::Rect;
		use wgpu_ui::widget::{DrawCtx, DrawPass, LayoutCtx};
		use wgpu_ui::{DrawList, Scroller, WidgetState};

		let (fonts, skin) = setup();
		let region = Rect::new(0.0, 0.0, 100.0, 100.0);
		let lctx = LayoutCtx { fonts: &fonts, theme: &skin, scale: 1.0, viewport: wgpu_ui::Rect::ZERO };
		let ctx = DrawCtx {
			fonts: &fonts,
			theme: &skin,
			scale: 1.0,
			hovered: WidgetId::NONE,
			focused: WidgetId::NONE,
			pass: DrawPass::Base,
		};

		let mut fits = Scroller::new();
		fits.layout(&lctx, region, 80.0);
		let mut dl = DrawList::new();
		fits.draw(&mut dl, &ctx);
		assert!(dl.is_empty(), "content fits -> no bar at all");

		// Content 200 in a 100 view: track = the right 8px (x 92..100), thumb
		// = half the track height, inset 1px -> (93, 1, 6, 48).
		let mut bar = Scroller::new();
		bar.layout(&lctx, region, 200.0);
		let thumb = bar.thumb_rect();
		assert_eq!((thumb.x, thumb.w), (92.0, 8.0), "the thumb spans the 8px gutter");
		let thumb_color = |state: WidgetState| {
			let mut dl = DrawList::new();
			Theme::scrollbar(&skin, &mut dl, bar.track_rect(), thumb, state);
			assert!(
				matches!(dl.cmds[0], DrawCmd::Solid { color, .. } if color == rgba(theme::SCROLL_TRACK)),
				"the flat track is under the thumb"
			);
			match dl.cmds[1] {
				DrawCmd::Solid { rect, color } => {
					assert_eq!((rect.x, rect.w), (93.0, 6.0), "the thumb face is inset 1px");
					color
				}
				_ => panic!("the thumb is a solid fill"),
			}
		};
		assert_eq!(thumb_color(WidgetState::default()), rgba(theme::SCROLL_THUMB), "rest");
		let hovered = WidgetState { hovered: true, ..Default::default() };
		assert_eq!(thumb_color(hovered), rgba(theme::SCROLL_THUMB_HOVER), "hover brightens");
		let dragging = WidgetState { pressed: true, ..Default::default() };
		assert_eq!(thumb_color(dragging), rgba(theme::SCROLL_THUMB_DRAG), "drag brightens most");
	}
}
