//! UI theme tokens - one place for every on-screen color literal, so re-skinning is a single file. Linear RGBA. The look aims at the
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
//! Every chrome part is drawn by the wgpu-ui
//! [`crate::uikit_theme::SteelTheme`] through the `Theme` trait - a widget asks
//! for a button face or a well, never for a colour - so the whole shell reads
//! as one machined surface. (The `kit::` shim that used to wrap those calls
//! app-side retired with U6.3; these constants are what the skin resolves to.)

// ----- ink + thin-line accents ------------------------------------------------

/// Primary / body text - silver.
pub const INK: [f32; 4] = [0.80, 0.82, 0.85, 1.0];
/// Accent ink - bright neon green (#44FF00, sRGB→linear), for titles +
/// active/selected items.
pub const ACCENT: [f32; 4] = [0.058, 1.0, 0.0, 1.0];
/// Secondary text (placeholders, hints) - dim gray.
pub const INK_DIM: [f32; 4] = [0.52, 0.54, 0.58, 1.0];
/// Close button glyph.
pub const CLOSE_INK: [f32; 4] = [0.85, 0.45, 0.32, 1.0];
/// A defect / error marker - the red box the Fix Shore tool draws around each
/// broken coast cell.
pub const DEFECT: [f32; 4] = [0.95, 0.16, 0.13, 1.0];
/// Hairline borders / focus-ring fallback (drawn over a bevel where one exists).
pub const PANEL_BORDER: [f32; 4] = [0.30, 0.33, 0.36, 1.0];
/// Splitters between windows in a dock + dock-edge resizers.
pub const SPLITTER: [f32; 4] = [0.05, 0.055, 0.065, 1.0];
/// A drop-target dock previewed while dragging a window near it - a black 50 %
/// wash drawn on the map *below* the windows (so docked panels stay readable).
pub const DOCK_PEEK: [f32; 4] = [0.0, 0.0, 0.0, 0.5];
/// Hovered row/cell - a 20 % darkening of the chrome beneath, not a colour
///. Opaque result (the chrome it covers is opaque).
pub const HOVER: [f32; 4] = [0.0, 0.0, 0.0, 0.20];
/// A button held down under the cursor - between [`HOVER`] and [`SELECTION`];
/// paired with an inset bevel so the key visibly sinks while pressed.
pub const PRESS: [f32; 4] = [0.0, 0.0, 0.0, 0.28];
/// A 50 % black veil dimming a cell a control shows but will not accept right
/// now (the toolbox's disallowed stamp orientations).
pub const VEIL: [f32; 4] = [0.0, 0.0, 0.0, 0.5];
/// The neutral black ground behind a sprite cell (the units roster's wells) -
/// a deliberate non-chrome backing, so team tints and sprite palettes read
/// true against it.
pub const SPRITE_WELL: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
/// The transparency checkerboard behind an authoring preview (New Scenery's
/// well). Two mid greys, far enough apart to read through a half-alpha shadow
/// and close enough not to compete with the art: what is see-through, and how
/// see-through, is the one thing a flat backdrop cannot show.
pub const CHECKER_LIGHT: [f32; 4] = [0.62, 0.62, 0.62, 1.0];
pub const CHECKER_DARK: [f32; 4] = [0.42, 0.42, 0.42, 1.0];
/// Floating-window resize-handle grip - a dark corner triangle.
pub const RESIZE_HANDLE: [f32; 4] = [0.0125, 0.015, 0.0175, 1.0];
/// Scrollbar track (inset well) and the draggable thumb over it.
pub const SCROLL_TRACK: [f32; 4] = [0.0, 0.0, 0.0, 0.34];
pub const SCROLL_THUMB: [f32; 4] = [0.55, 0.57, 0.62, 0.95];
/// The thumb under the cursor, and while it's being dragged.
pub const SCROLL_THUMB_HOVER: [f32; 4] = [0.68, 0.70, 0.75, 0.95];
pub const SCROLL_THUMB_DRAG: [f32; 4] = [0.80, 0.82, 0.87, 1.0];
/// Chrome-label drop shadow (bottom-right) - the emboss under button/menu/
/// caption text, light coming from the top-left.
pub const TEXT_SHADOW: [f32; 4] = [0.0, 0.0, 0.0, 0.55];
/// Chrome-label highlight (top-left) - the lit edge of the emboss.
pub const TEXT_HILITE: [f32; 4] = [1.0, 1.0, 1.0, 0.16];

// ----- the steel-skin material system -----------------------------------------

/// A chrome fill rendered as a **brightness/darkness exposure** of the raw
/// steel sheet - the same lighten/darken idea as the bevel, applied to the
/// whole surface:
///
/// - `grain.rgb` multiplies the steel texel - the exposure level (`<1` darkens,
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

/// Panel / dialog body - steel darkened to a dark gunmetal exposure. Opaque.
pub const PANEL: Material = Material { base: [0.14, 0.15, 0.17, 1.0], grain: [0.72, 0.76, 0.82, 0.85] };
/// The menu bar band - a lifted, faintly warm exposure.
pub const TITLE: Material = Material { base: [0.19, 0.19, 0.18, 1.0], grain: [0.92, 0.90, 0.84, 0.86] };

/// The shared window / dialog **titlebar** - rusted (warm orange-brown) steel;
/// every window and modal titlebar uses this one look (docs/ui/theme.md §6.1).
pub const RUST_TITLE: Material = Material { base: [0.22, 0.12, 0.07, 1.0], grain: [1.0, 0.56, 0.32, 0.85] };
/// The titlebar of a window being dragged - a brighter rust.
pub const RUST_TITLE_DRAG: Material = Material { base: [0.30, 0.17, 0.10, 1.0], grain: [1.15, 0.68, 0.40, 0.86] };
/// The 1px recessed seam under a titlebar - the same rust, darkened.
pub const RUST_EDGE: Material = Material { base: [0.10, 0.055, 0.03, 1.0], grain: [0.5, 0.28, 0.16, 0.9] };
/// Titlebar **title** text - amber (#FFCC00), the shared title ink.
pub const TITLE_INK: [f32; 4] = [1.0, 0.604, 0.0, 1.0];

/// A plain clickable button - a brighter, faintly warm exposure so it stands
/// off panels.
pub const BUTTON: Material = Material { base: [0.22, 0.22, 0.21, 1.0], grain: [1.08, 1.05, 0.99, 0.87] };
/// The primary action (Create / Save / Open / Resize / Start) - a green "go"
/// exposure, the app's one lit-green identity (shared with [`BUTTON_ACTIVE`]);
/// on hover it brightens its own green (see `SteelTheme::button`).
pub const BUTTON_PRIMARY: Material = Material { base: [0.10, 0.30, 0.10, 1.0], grain: [0.55, 1.35, 0.42, 0.87] };
/// A toggled-on control (selected tool / mode / anchor / pass) - the lit-green
/// identity of [`BUTTON_PRIMARY`] pushed a clear step brighter, paired with an
/// inset bevel in `SteelTheme::button` so the key reads pressed-down: an active
/// toggle must be tellable from its neighbours at a glance, where the CTA green
/// only has to stand off a dialog's neutral faces.
pub const BUTTON_ACTIVE: Material = Material { base: [0.14, 0.48, 0.12, 1.0], grain: [0.80, 1.90, 0.60, 0.62] };
/// The secondary (non-destructive alternate) CTA - warm amber steel (Cancel).
pub const BUTTON_SECONDARY: Material = Material { base: [0.34, 0.25, 0.12, 1.0], grain: [1.35, 1.02, 0.55, 0.87] };
/// The danger (destructive) CTA - deep red steel.
pub const BUTTON_DANGER: Material = Material { base: [0.34, 0.10, 0.09, 1.0], grain: [1.40, 0.50, 0.45, 0.87] };
/// A control that can't be used right now (settings locked mid-run): a muted
/// face between PANEL and BUTTON - visibly a key, visibly inert.
pub const BUTTON_DISABLED: Material = Material { base: [0.165, 0.17, 0.18, 1.0], grain: [0.82, 0.84, 0.88, 0.78] };

// ----- accent-tint state model (docs/ui/theme.md §4) --------------------------
// A control's interaction state (hover / active) tints its surface toward an
// accent, in LINEAR light: `out = base·(1 - f·(1 - accent)) + f·accent`, clamped,
// with the steel grain re-exposed over the tinted base. Text never blends (§0).

/// Primary accent (green "go") - the default hover/active tint + Primary CTA.
pub const ACCENT_PRIMARY: [f32; 3] = [0.058, 1.0, 0.0];
/// Secondary accent (amber) - the non-destructive alternate CTA (Cancel).
pub const ACCENT_SECONDARY: [f32; 3] = [1.0, 0.604, 0.0];
/// Danger accent (red) - the destructive CTA.
pub const ACCENT_DANGER: [f32; 3] = [1.0, 0.0, 0.0];

/// State floor `f` (added accent intensity) for a hovered control.
pub const FLOOR_HOVER: f32 = 0.30;
/// State floor for a control that is both active and hovered (active + hover).
pub const FLOOR_ACTIVE_HOVER: f32 = 0.70;

/// A text field / list / well - a darker, recessed exposure; still textured so
/// it reads as machined metal rather than a flat hole.
pub const TEXTAREA: Material = Material { base: [0.09, 0.10, 0.12, 1.0], grain: [0.62, 0.65, 0.72, 0.80] };

// ----- console ------------------------------------------------------------------
// The drop-down command console (a G5 surface: its scrollback is still a
// faithful draw until the toolkit grows a LogView). Deliberately not steel -
// a translucent terminal plate over whatever is beneath.

/// The console plate - a translucent near-black wash.
pub const CONSOLE_PANEL: [f32; 4] = [0.04, 0.05, 0.07, 0.88];
/// The plate's bottom edge - the console's cyan identity line.
pub const CONSOLE_BORDER: [f32; 4] = [0.25, 0.70, 0.92, 0.85];
/// Scrollback output ink.
pub const CONSOLE_LOG: [f32; 4] = [0.80, 0.86, 0.82, 1.0];
/// The input line's ink - the brightest; it is where the user is typing.
pub const CONSOLE_INPUT: [f32; 4] = [0.96, 0.98, 0.92, 1.0];
/// Echoed commands (`] ...`) - dimmer than output, so replies stand out.
pub const CONSOLE_ECHO: [f32; 4] = [0.55, 0.65, 0.75, 1.0];
/// Errors / failures - soft red.
pub const CONSOLE_ERROR: [f32; 4] = [0.95, 0.45, 0.40, 1.0];

// ----- match editor (DEV) -------------------------------------------------------

/// A water-class cell in the match editor's orientation previews.
pub const MATCH_WATER: [f32; 4] = [0.12, 0.45, 0.95, 1.0];
/// A land-class cell in the previews.
pub const MATCH_LAND: [f32; 4] = [0.10, 0.62, 0.16, 1.0];
/// A wildcard-rule row's tone in the match lists - amber-yellow.
pub const MATCH_RULE: [f32; 4] = [0.93, 0.83, 0.22, 1.0];
/// A warning row's tone (candidate / unsaved markers) - orange.
pub const MATCH_WARN: [f32; 4] = [0.96, 0.56, 0.16, 1.0];

// ----- UI Tests font probe (DEV) ------------------------------------------------
//
// The probe is a *measuring instrument*, so its tones are deliberately not the
// steel chrome's: pure white on pure black is the highest-contrast ground there
// is, which is what makes a half-lit anti-aliased edge visible at all.

/// The probe's ground - true black, so nothing the raster puts down is hidden
/// by the surface under it.
pub const PROBE_GROUND: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
/// The sample text's ink - true white, full contrast against [`PROBE_GROUND`].
pub const PROBE_INK: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
/// The lit half-ground under the right end of each role specimen. On black the
/// engraving's *shadow* is black on black - invisible - so an engraved line and
/// a flat one look identical, which is the one comparison the probe must not
/// lose. Each role row therefore crosses from [`PROBE_GROUND`] onto this mid
/// grey (near the steel's own value) half way along, and the shadow appears.
pub const PROBE_LIT: [f32; 4] = [0.42, 0.42, 0.44, 1.0];
/// The readout captions beside each sample - grey, so they read as annotation
/// rather than as another specimen.
pub const PROBE_NOTE: [f32; 4] = [0.62, 0.64, 0.68, 1.0];
/// The caption of the one ladder row the chrome is *actually* rasterizing body
/// text at right now - green, so the sweep stays tied to what is on screen.
pub const PROBE_LIVE: [f32; 4] = [0.36, 0.86, 0.40, 1.0];

// ----- Edit Save Data tables ----------------------------------------------------

/// Per-team column washes behind the Edit Save Data all-players tables - the
/// game's five player colours (`units::TEAM_SWATCH`) at a whisper of alpha, so
/// each column reads as its team without drowning the fields on the steel.
pub const TEAM_WASH: [[f32; 4]; 5] = [
	[0.78, 0.16, 0.16, 0.14],
	[0.20, 0.62, 0.22, 0.14],
	[0.22, 0.38, 0.78, 0.14],
	[0.55, 0.55, 0.55, 0.14],
	[0.80, 0.72, 0.25, 0.14],
];
