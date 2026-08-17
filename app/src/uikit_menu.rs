//! Renders the editor's main menu bar and right-click context menu through the
//! first-party `wgpu-ui` renderer + the brushed-steel [`SteelTheme`], so the
//! menu matches the dialogs (crisp pixel-locked text, the same engraved emboss
//! and steel skin). It owns a `wgpu-ui` `UiRenderer`, a `SteelTheme` and a
//! `Fonts`, and lives in [`Passes`](crate::Passes) so `render_frame` can reach
//! it in every path - including the headless `--screenshot` path, where there
//! is no modal overlay but the menu must still render faithfully.
//!
//! The menu keeps its own geometry/state/hit-testing (in [`crate::menu`]); this
//! only swaps the *drawing* backend, compositing the menu's `DrawList` at the
//! menu's z-position (over the chrome, under modals).

use wgpu_ui::{DrawList, Fonts, UiRenderer, Vec2};

use crate::uikit_theme::SteelTheme;

/// The editor's MAX UI font (glyf TrueType), parsed by `wgpu-ui` directly.
const FONT: &[u8] = include_bytes!("../assets/MAX_Redesign_Square.ttf");

/// The monospace face for `TextRole::Mono` — the console. Hack (MIT over the
/// Bitstream Vera license, `assets/Hack-LICENSE.txt`), as a real outline font:
/// the console once drew its own glyphs from a baked Hack *bitmap atlas*, which
/// could not follow UI Scale. Parsing the typeface instead keeps the look and
/// gives one font stack, and a console that scales with the rest of the chrome.
const MONO_FONT: &[u8] = include_bytes!("../assets/Hack-Regular.ttf");

pub struct MenuChrome {
	rend: UiRenderer,
	theme: SteelTheme,
	fonts: Fonts,
}

impl MenuChrome {
	/// Builds the menu renderer sharing the editor's GPU. Returns `None` if the
	/// font fails to parse (the editor then runs without the wgpu-ui menu).
	pub fn new(
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		format: wgpu::TextureFormat,
		steel: &crate::skin::Image,
	) -> Option<Self> {
		let mut fonts = Fonts::new();
		let font = fonts.add(FONT.to_vec()).ok()?;
		let mono = fonts.add(MONO_FONT.to_vec()).ok()?;
		let mut rend = UiRenderer::new(device, queue, format);
		let steel_tex = rend.register_texture(&steel.rgba, steel.size.0, steel.size.1);
		let em = fonts.get(font).units_per_em();
		let theme = SteelTheme::new(font, steel_tex, em).with_mono(mono);
		Some(Self { rend, theme, fonts })
	}

	/// Registers an RGBA texture in this renderer (e.g. the console glyph atlas),
	/// returning its id for `DrawList::image` ops composited via `render_list`.
	pub fn register_texture(&mut self, rgba: &[u8], w: u32, h: u32) -> wgpu_ui::TextureId {
		self.rend.register_texture(rgba, w, h)
	}

	/// Re-creates a registered texture at a (possibly new) size, reusing its id
	/// — for a modal preview whose contents change per open.
	pub fn replace_texture(&mut self, id: wgpu_ui::TextureId, rgba: &[u8], w: u32, h: u32) {
		self.rend.replace_texture(id, rgba, w, h);
	}

	/// Rewrites a registered texture's pixels at its existing size — the cheap
	/// per-frame path (the Tile Painter's live canvas/swatches).
	pub fn update_texture(&self, id: wgpu_ui::TextureId, rgba: &[u8]) {
		self.rend.update_texture(id, rgba);
	}

	/// Anchors the steel grain / emboss to the viewport (origin 0, full-viewport
	/// stretch) at `scale`, for the always-on chrome. Returns the logical size.
	/// Call before building a chrome `DrawList` through [`theme`](Self::theme).
	pub fn prepare(&mut self, size: (u32, u32), scale: f32) -> (f32, f32) {
		let (vw, vh) = (size.0 as f32 / scale, size.1 as f32 / scale);
		self.rend.set_scale(scale);
		self.theme.set_scale(scale);
		self.theme.set_origin(Vec2::new(0.0, 0.0));
		self.theme.set_viewport(vw, vh);
		(vw, vh)
	}

	/// Like [`prepare`](Self::prepare), but samples the steel through the editor's
	/// [`SteelMap`](crate::ui::SteelMap) (a panel's own mapping) instead of the
	/// viewport origin - so a migrated panel's chrome grain matches its still-
	/// native content. Call before building a panel's chrome `DrawList`.
	pub fn prepare_panel(&mut self, size: (u32, u32), scale: f32, map: crate::ui::SteelMap) {
		let (vw, vh) = (size.0 as f32 / scale, size.1 as f32 / scale);
		self.rend.set_scale(scale);
		self.theme.set_scale(scale);
		self.theme.set_steel_map(map);
		self.theme.set_viewport(vw, vh);
	}

	/// The steel theme + fonts a caller draws a chrome `DrawList` through (after
	/// [`prepare`](Self::prepare)).
	pub fn theme(&self) -> &SteelTheme {
		&self.theme
	}

	pub fn fonts(&self) -> &Fonts {
		&self.fonts
	}

	/// Sets the renderer + theme scale (logical→physical) without touching the
	/// steel mapping — for a caller that drives its own viewport/origin (the
	/// dialog [`Overlay`](crate::uikit_overlay::Overlay), which shares this one
	/// renderer/theme/fonts so the font is parsed and the steel registered once).
	pub fn set_scale(&mut self, scale: f32) {
		self.rend.set_scale(scale);
		self.theme.set_scale(scale);
	}

	/// The steel theme, mutably — so the overlay can anchor the grain to its
	/// (possibly dragged) dialog window before drawing through the shared renderer.
	pub fn theme_mut(&mut self) -> &mut SteelTheme {
		&mut self.theme
	}

	/// Composites a prepared chrome `DrawList` over `view`.
	pub fn render_list(
		&mut self,
		encoder: &mut wgpu::CommandEncoder,
		view: &wgpu::TextureView,
		size: (u32, u32),
		dl: &DrawList,
	) {
		self.rend.render_into(encoder, view, size, &self.fonts, dl);
	}
}
