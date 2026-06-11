//! UI theme tokens — one place for every on-screen color literal, so re-skinning is a single file. Linear RGBA. The look aims at the
//! original M.A.X. shell: brushed-gunmetal chrome with amber labels.
//!
//! Chrome is **textured**: every panel/button/field is cut from one
//! brushed-steel sheet ([`crate::skin`]), composited as
//!
//! ```text
//!   base fill           // a flat tone that sets the element's lightness
//!   + steel grain       // the sheet, tinted, alpha = how strongly it shows
//!   + directional bevel // lit top-left, shaded bottom-right (raised) or the
//!                        // reverse (inset wells)
//! ```
//!
//! The reusable widget helpers live on [`crate::ui::UiQuads`]
//! (`button`/`button_primary`/`button_active`/`field`/`raised`/`inset`); panels
//! and modals share them so the whole shell reads as one machined surface.

// ----- ink + thin-line accents ------------------------------------------------

/// Primary / body text — silver.
pub const INK: [f32; 4] = [0.80, 0.82, 0.85, 1.0];
/// Accent ink — bright neon green (#44FF00, sRGB→linear), for titles +
/// active/selected items.
pub const ACCENT: [f32; 4] = [0.058, 1.0, 0.0, 1.0];
/// Secondary text (placeholders, hints) — dim gray.
pub const INK_DIM: [f32; 4] = [0.52, 0.54, 0.58, 1.0];
/// Bright cool silver — dockable-window captions.
pub const SILVER: [f32; 4] = [0.84, 0.86, 0.91, 1.0];
/// Close button glyph.
pub const CLOSE_INK: [f32; 4] = [0.85, 0.45, 0.32, 1.0];
/// Hairline borders / focus-ring fallback (drawn over a bevel where one exists).
pub const PANEL_BORDER: [f32; 4] = [0.30, 0.33, 0.36, 1.0];
/// Splitters between windows in a dock + dock-edge resizers.
pub const SPLITTER: [f32; 4] = [0.05, 0.055, 0.065, 1.0];
/// A drop-target dock previewed while dragging a window near it — a black 50 %
/// wash drawn on the map *below* the windows (so docked panels stay readable).
pub const DOCK_PEEK: [f32; 4] = [0.0, 0.0, 0.0, 0.5];
/// Hovered row/cell — a 20 % darkening of the chrome beneath, not a colour
///. Opaque result (the chrome it covers is opaque).
pub const HOVER: [f32; 4] = [0.0, 0.0, 0.0, 0.20];
/// Selected row / open menu — a stronger darkening.
pub const SELECTION: [f32; 4] = [0.0, 0.0, 0.0, 0.34];
/// A button held down under the cursor — between [`HOVER`] and [`SELECTION`];
/// paired with an inset bevel so the key visibly sinks while pressed.
pub const PRESS: [f32; 4] = [0.0, 0.0, 0.0, 0.28];
/// Floating-window resize-handle grip — a dark corner triangle.
pub const RESIZE_HANDLE: [f32; 4] = [0.0125, 0.015, 0.0175, 1.0];
/// Scrollbar track (inset well) and the draggable thumb over it.
pub const SCROLL_TRACK: [f32; 4] = [0.0, 0.0, 0.0, 0.34];
pub const SCROLL_THUMB: [f32; 4] = [0.55, 0.57, 0.62, 0.95];
/// The thumb under the cursor, and while it's being dragged.
pub const SCROLL_THUMB_HOVER: [f32; 4] = [0.68, 0.70, 0.75, 0.95];
pub const SCROLL_THUMB_DRAG: [f32; 4] = [0.80, 0.82, 0.87, 1.0];
/// Chrome-label drop shadow (bottom-right) — the emboss under button/menu/
/// caption text, light coming from the top-left.
pub const TEXT_SHADOW: [f32; 4] = [0.0, 0.0, 0.0, 0.55];
/// Chrome-label highlight (top-left) — the lit edge of the emboss.
pub const TEXT_HILITE: [f32; 4] = [1.0, 1.0, 1.0, 0.16];

// ----- the steel-skin material system -----------------------------------------

/// A chrome fill rendered as a **brightness/darkness exposure** of the raw
/// steel sheet — the same lighten/darken idea as the bevel, applied to the
/// whole surface:
///
/// - `grain.rgb` multiplies the steel texel — the exposure level (`<1` darkens,
///   `>1` lightens, a colour vector tints warm/green).
/// - `grain.a` is how much of that exposed steel shows; kept high (~0.85) so the
///   grain survives.
/// - the small remainder fades to the flat `base` tone, lowering the texture's
///   contrast only slightly (~30% of the old flat-base wash) to keep detail.
#[derive(Clone, Copy)]
pub struct Material {
	pub base: [f32; 4],
	pub grain: [f32; 4],
}

/// Directional-light edges (emulated bevel), lit from the top-left. The four
/// edges carry their own tone and the corners are mitered at 45° (CSS-border
/// style, no overlap): `top` lit, `bottom` shaded, with `left` a touch brighter
/// and `right` a touch darker than those (the side walls catch / lose the most
/// light). Inset wells swap the lit/shaded sets. Each blends over whatever is
/// beneath, so one bevel suits every tint.
#[derive(Clone, Copy)]
pub struct Bevel {
	pub top: [f32; 4],
	pub bottom: [f32; 4],
	pub left: [f32; 4],
	pub right: [f32; 4],
}

/// The shared bevel: a soft white top + deep-shadow bottom, with a brighter
/// left wall and a darker right wall. Drives raised chrome and inset wells.
pub const BEVEL: Bevel = Bevel {
	top: [1.0, 1.0, 1.0, 0.16],
	bottom: [0.0, 0.0, 0.0, 0.42],
	left: [1.0, 1.0, 1.0, 0.24],
	right: [0.0, 0.0, 0.0, 0.55],
};

/// Panel / dialog body — steel darkened to a dark gunmetal exposure. Opaque.
pub const PANEL: Material = Material { base: [0.14, 0.15, 0.17, 1.0], grain: [0.72, 0.76, 0.82, 0.85] };
/// Titlebars + the menu bar — a lifted, faintly warm exposure.
pub const TITLE: Material = Material { base: [0.19, 0.19, 0.18, 1.0], grain: [0.92, 0.90, 0.84, 0.86] };
/// A titlebar while its panel is dragged (brighter exposure).
pub const TITLE_DRAG: Material = Material { base: [0.25, 0.26, 0.28, 1.0], grain: [1.05, 1.06, 1.12, 0.88] };

/// A plain clickable button — a brighter, faintly warm exposure so it stands
/// off panels.
pub const BUTTON: Material = Material { base: [0.22, 0.22, 0.21, 1.0], grain: [1.08, 1.05, 0.99, 0.87] };
/// The primary action (Create / Save / Open / Resize / Start) — warm amber steel.
pub const BUTTON_PRIMARY: Material = Material { base: [0.34, 0.25, 0.12, 1.0], grain: [1.35, 1.02, 0.55, 0.87] };
/// A toggled-on control (selected tool / mode / anchor / pass) — a green
/// exposure.
pub const BUTTON_ACTIVE: Material = Material { base: [0.10, 0.30, 0.10, 1.0], grain: [0.55, 1.35, 0.42, 0.87] };
/// A control that can't be used right now (settings locked mid-run): a muted
/// face between PANEL and BUTTON — visibly a key, visibly inert.
pub const BUTTON_DISABLED: Material = Material { base: [0.165, 0.17, 0.18, 1.0], grain: [0.82, 0.84, 0.88, 0.78] };

/// A text field / list / well — a darker, recessed exposure; still textured so
/// it reads as machined metal rather than a flat hole.
pub const TEXTAREA: Material = Material { base: [0.09, 0.10, 0.12, 1.0], grain: [0.62, 0.65, 0.72, 0.80] };
