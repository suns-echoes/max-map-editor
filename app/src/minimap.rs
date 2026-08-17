//! Interactive minimap dockable: the whole map fitted into
//! the panel body with the current view as a draggable rectangle - click or
//! drag pans, wheel over it zooms. Three sources (header radios):
//! **overworld** (the composed map, sampled per panel pixel), **pass**
//! (passability colors), **minimap** (the in-game minimap bytes).
//!
//! Geometry/hit logic is pure (tested); pixels are CPU-built into a small
//! RGBA texture (rebuilt on revision/mode/size change, palette snapshot at
//! build time) and blitted by `blit.wgsl`.
//!
//! **The panel is a real `wgpu-ui` widget tree** (U5.3, the stage U5 content-
//! widget pilot): a `Linear` column of a **header row of three mode keys** —
//! stock `Button`s, selected-face — over a [`MinimapView`], the **content
//! widget** that reserves the fitted texture rect the native blit fills, draws
//! the camera outline over it, and owns the pan drag as a real pointer capture.
//! `radio_rect`, the `ArmFire<Mode>`, the radio half of `click` and the `hot`
//! field are gone. The header steel band is still drawn shell-side (the material
//! fill is not on the `Theme` trait) — that stays until U6.

use wgpu_ui::widget::{DrawCtx, EventCtx, LayoutCtx, Widget};
use wgpu_ui::{
	CrossAlign, DrawList, Event, Insets, Length, Linear, PointerButton, Size, Vec2, WidgetId, descendant_mut,
};

use crate::blit::BlitPass;
use crate::state::EditorState;
use crate::theme;
use crate::ui::Rect;
use crate::uikit_theme::rgba;

pub const HEADER_H: f32 = 22.0;
const PAD: f32 = 4.0;

/// Pass colors (sRGB): land / water / shore / blocked.
const PASS_RGBA: [[u8; 4]; 4] = [[58, 140, 58, 255], [42, 90, 223, 255], [200, 180, 0, 255], [140, 42, 42, 255]];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
	Overworld,
	Pass,
	Minimap,
}

impl Mode {
	pub const ALL: [Mode; 3] = [Mode::Overworld, Mode::Pass, Mode::Minimap];

	pub fn name(self) -> &'static str {
		match self {
			Mode::Overworld => "overworld",
			Mode::Pass => "pass",
			Mode::Minimap => "minimap",
		}
	}

	/// The header key's caption — the command name abbreviated to fit a third of
	/// a docked minimap's width.
	fn key(self) -> &'static str {
		match self {
			Mode::Overworld => "over",
			Mode::Pass => "pass",
			Mode::Minimap => "mini",
		}
	}

	pub fn parse(s: &str) -> Option<Mode> {
		Self::ALL.iter().copied().find(|m| m.name() == s)
	}
}

/// The panel body below the header band — the rect the [`MinimapView`] content
/// widget is arranged into, and what [`fit`] letterboxes the map inside.
fn content_of(body: Rect) -> Rect {
	Rect::new(body.x, body.y + HEADER_H, body.w, (body.h - HEADER_H).max(0.0))
}

/// The fitted map rect inside a *content* rect (the body below the header) +
/// panel px per map cell. Pure, and the one definition of the minimap's
/// geometry: [`MinimapView`] reserves this rect from its own arranged rect, and
/// [`map_area`] gives the same answer to the native blit from the whole body.
pub fn fit(map: (u16, u16), content: Rect) -> (Rect, f32) {
	let avail =
		Rect::new(content.x + PAD, content.y + PAD, (content.w - 2.0 * PAD).max(1.0), (content.h - 2.0 * PAD).max(1.0));
	let scale = (avail.w / map.0 as f32).min(avail.h / map.1 as f32).max(0.001);
	let (mw, mh) = (map.0 as f32 * scale, map.1 as f32 * scale);
	(Rect::new(avail.x + (avail.w - mw) / 2.0, avail.y + (avail.h - mh) / 2.0, mw, mh), scale)
}

/// The fitted map rect inside a whole panel body (header included) + panel px
/// per map cell — [`fit`] applied below the header band.
pub fn map_area(map: (u16, u16), body: Rect) -> (Rect, f32) {
	fit(map, content_of(body))
}

/// Cursor → map cell coords (fractional) against an already-fitted `area`,
/// clamped to the map — so a drag-pan keeps tracking the edge once the cursor
/// leaves the rect (and, since U5.3, the panel).
pub fn pan_target_in(map: (u16, u16), area: Rect, x: f32, y: f32) -> (f32, f32) {
	let scale = (area.w / map.0 as f32).max(0.001);
	(
		((x.clamp(area.x, area.x + area.w) - area.x) / scale).min(map.0 as f32),
		((y.clamp(area.y, area.y + area.h) - area.y) / scale).min(map.1 as f32),
	)
}

/// The camera-viewport border rect inside the minimap `body` — the visible
/// world window mapped into the fitted area — or `None` if it's too small to
/// draw. Pure (the geometry the overlay outlines over the minimap texture).
pub fn view_rect(editor: &EditorState, body: Rect) -> Option<Rect> {
	let map = editor.map_size();
	let (area, scale) = map_area(map, body);
	let zoom = editor.view.zoom;
	let cell_px = crate::render::TILE_PX as f32;
	let (sw, sh) = (editor.screen.0 as f32, editor.screen.1 as f32);
	let x0 = area.x + editor.view.pan[0] / cell_px * scale;
	let y0 = area.y + editor.view.pan[1] / cell_px * scale;
	let vw = sw / zoom / cell_px * scale;
	let vh = sh / zoom / cell_px * scale;
	// Clamp to the fitted rect so an off-map view doesn't bleed out.
	let cx0 = x0.clamp(area.x, area.x + area.w);
	let cy0 = y0.clamp(area.y, area.y + area.h);
	let cx1 = (x0 + vw).clamp(area.x, area.x + area.w);
	let cy1 = (y0 + vh).clamp(area.y, area.y + area.h);
	(cx1 - cx0 >= 2.0 && cy1 - cy0 >= 2.0).then(|| Rect::new(cx0, cy0, cx1 - cx0, cy1 - cy0))
}

/// The minimap's **content widget**: it reserves the fitted map rect the native
/// blit fills, draws the camera-view outline over it, and owns the pan drag.
///
/// The pan is a real pointer **capture** (U5.3). Before, the overlay declined to
/// consume a press on the texture so it fell through to a shell-side `minipan`
/// rect, and the shell re-derived the target on every `CursorMoved`; the drag
/// only kept working off-panel because the shell was driving it. Now the widget
/// captures on the press, keeps converting moves while it holds the pointer
/// (the router feeds it every one, wherever the cursor goes) and lets go on the
/// release. What it must **not** do — the §5.2 line for a content widget — is
/// contain any chrome: the mode keys are its siblings in the tree, not its
/// children.
pub struct MinimapView {
	id: WidgetId,
	/// Map size, synced per frame — the pan/fit geometry needs it.
	map: (u16, u16),
	/// The camera-viewport outline, precomputed shell-side ([`view_rect`]).
	view: Option<Rect>,
	/// The fitted map rect, settled in `arrange` — what the blit fills.
	area: Rect,
	rect: Rect,
	/// Live while this widget holds the pointer for a pan drag.
	dragging: bool,
	/// The latest pan target (map cells, fractional), polled by the shell.
	pan: Option<(f32, f32)>,
}

impl MinimapView {
	fn new() -> Self {
		Self {
			id: wgpu_ui::next_id(),
			map: (1, 1),
			view: None,
			area: Rect::ZERO,
			rect: Rect::ZERO,
			dragging: false,
			pan: None,
		}
	}

	/// Take the pending pan target (map cells, fractional). A slot, not a queue:
	/// only the newest position of a drag matters, and the shell polls after
	/// every dispatch that could have moved it.
	pub fn take_pan(&mut self) -> Option<(f32, f32)> {
		self.pan.take()
	}

	fn aim(&self, at: Vec2) -> (f32, f32) {
		pan_target_in(self.map, self.area, at.x, at.y)
	}
}

impl Widget for MinimapView {
	fn measure(&mut self, avail: Size, _ctx: &mut LayoutCtx) -> Size {
		avail
	}

	fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
		self.rect = rect;
		self.area = fit(self.map, rect).0;
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		if !ctx.is_base() {
			return;
		}
		// The map pixels are a native blit into `area`; this widget draws only
		// the camera outline over them.
		if let Some(v) = self.view {
			dl.stroke_rect(v, 1.0, rgba(theme::INK));
		}
	}

	fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
		match ev {
			Event::PointerButton { button: PointerButton::Primary, pressed: true, .. } if ctx.is_target(self.id) => {
				self.dragging = true;
				self.pan = Some(self.aim(ctx.pointer));
				ctx.capture(self.id);
				ctx.consume_pointer();
				true
			}
			// Mid-drag the pointer is ours wherever it is; `pan_target_in` clamps
			// to the map, so dragging off the panel keeps tracking the edge.
			Event::PointerMoved { .. } if self.dragging => {
				self.pan = Some(self.aim(ctx.pointer));
				ctx.consume_pointer();
				true
			}
			Event::PointerButton { button: PointerButton::Primary, pressed: false, .. } if self.dragging => {
				self.dragging = false;
				ctx.consume_pointer();
				true
			}
			// The release will never arrive (the window lost focus mid-drag), so
			// end it here or the pan follows the cursor forever - the hole G9
			// found in `Slider`, which `Scroller` had already closed.
			Event::Focus(false) => {
				self.dragging = false;
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

	/// Only the fitted map claims the pointer: the letterbox margin around it is
	/// inert, exactly as the old `click` oracle had it.
	fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
		self.area.contains(pos).then_some(self.id)
	}
}

/// The minimap panel as a retained `wgpu_ui` [`Widget`]: a thin root over a
/// `Linear` column of the three mode keys and the [`MinimapView`]. It exists to
/// hold the key ids and to forward the per-frame sync / the view's outputs; the
/// tree owns layout, paint, hover, arming, firing and the pan capture.
pub struct MinimapOverlay {
	id: WidgetId,
	root: Linear,
	/// The three header keys, in [`Mode::ALL`] order.
	keys: [WidgetId; 3],
	view_id: WidgetId,
	rect: Rect,
}

impl Default for MinimapOverlay {
	fn default() -> Self {
		Self::new()
	}
}

impl MinimapOverlay {
	pub fn new() -> Self {
		// The three keys share the header width evenly (2px outer margins + 2px
		// gaps), stretched to the band's height — the segmented control the
		// hand-drawn `radio_rect` always described.
		let mut header = Linear::row().padding(Insets::all(2.0)).spacing(2.0).cross_align(CrossAlign::Stretch);
		let mut keys = [WidgetId::NONE; 3];
		for (i, m) in Mode::ALL.iter().enumerate() {
			let key = wgpu_ui::Button::new(m.key()).small().action(i as u64);
			keys[i] = key.id();
			header = header.child(key, Length::Flex(1.0));
		}
		let view = MinimapView::new();
		let view_id = view.id();
		// `Stretch` is what makes the header band follow the dock's width: a
		// `Linear` measures to its *content*, so without it the key row keeps its
		// measured width (three `button_min_width`s) and overflows a narrow dock.
		let root = Linear::column()
			.cross_align(CrossAlign::Stretch)
			.child(header, Length::Fixed(HEADER_H))
			.child(view, Length::Flex(1.0));
		Self { id: wgpu_ui::next_id(), root, keys, view_id, rect: Rect::ZERO }
	}

	/// Refresh the per-frame inputs: the active mode (which key lights), the map
	/// size (the view's fit/pan geometry) and the precomputed camera outline
	/// ([`view_rect`]).
	pub fn sync(&mut self, mode: Mode, map: (u16, u16), view: Option<Rect>) {
		for (i, m) in Mode::ALL.iter().enumerate() {
			if let Some(key) = descendant_mut::<wgpu_ui::Button>(&mut self.root, self.keys[i]) {
				key.set_selected(*m == mode);
			}
		}
		if let Some(v) = descendant_mut::<MinimapView>(&mut self.root, self.view_id) {
			v.map = map;
			v.view = view;
		}
	}

	/// The mode a fired action tag stands for — the shell maps what its `Ui`
	/// collected back through [`Mode::ALL`].
	pub fn mode_of(tag: u64) -> Option<Mode> {
		Mode::ALL.get(tag as usize).copied()
	}

	/// Take the content widget's pending pan target — see
	/// [`MinimapView::take_pan`].
	pub fn take_pan(&mut self) -> Option<(f32, f32)> {
		descendant_mut::<MinimapView>(&mut self.root, self.view_id).and_then(|v| v.take_pan())
	}
}

impl Widget for MinimapOverlay {
	crate::panel_ui::thin_root_plumbing!(arrange, draw, event);
}

/// Build the source texture's RGBA for `mode` at `tex` resolution.
fn build_rgba(editor: &EditorState, mode: Mode, tex: (u32, u32)) -> Vec<u8> {
	let (tw, th) = tex;
	let map = editor.map_size();
	let mut out = Vec::with_capacity((tw * th * 4) as usize);

	let palette_rgba = |palette: &[u8], index: u8, out: &mut Vec<u8>| {
		let i = index as usize * 3;
		out.extend_from_slice(&[palette[i], palette[i + 1], palette[i + 2], 255]);
	};

	let project = &editor.project;
	match mode {
		Mode::Overworld => {
			// One composed-world sample per texel (nearest "downscale").
			for j in 0..th {
				for i in 0..tw {
					let wx = (i as f32 + 0.5) / tw as f32 * map.0 as f32 * 64.0;
					let wy = (j as f32 + 0.5) / th as f32 * map.1 as f32 * 64.0;
					let (cx, cy) = ((wx / 64.0) as u16, (wy / 64.0) as u16);
					let sub = ((wx % 64.0) as usize, (wy % 64.0) as usize);
					palette_rgba(&project.palette, project.pixel_at(cx, cy, sub), &mut out);
				}
			}
		}
		Mode::Pass => {
			for y in 0..map.1 {
				for x in 0..map.0 {
					let pass = project.pass_at(x, y).unwrap_or(0).min(3);
					out.extend_from_slice(&PASS_RGBA[pass as usize]);
				}
			}
		}
		Mode::Minimap => {
			for y in 0..map.1 {
				for x in 0..map.0 {
					palette_rgba(&project.palette, project.minimap_pixel(x, y), &mut out);
				}
			}
		}
	}
	out
}

/// Texture resolution for a mode: overworld samples at panel resolution,
/// pass/minimap are one texel per cell (blit upscales nearest = chunky).
fn tex_size(editor: &EditorState, mode: Mode, area: Rect) -> (u32, u32) {
	let map = editor.map_size();
	match mode {
		Mode::Overworld => ((area.w.max(1.0)) as u32, (area.h.max(1.0)) as u32),
		_ => (map.0 as u32, map.1 as u32),
	}
}

// ----- GPU side (texture cache over the shared BlitPass) ---------------------

/// The minimum wall-time between content rebuilds while the document keeps
/// changing. The minimap texture is a whole-panel CPU sweep (`build_rgba` -
/// one composed sample per texel in Overworld mode), and every painted cell
/// bumps `revision()`, so an un-throttled cache would rebuild + re-upload it
/// on *every* frame of a paint/stamp/pass stroke. Rebuilding at most ~10 Hz
/// keeps the overview live enough while cutting that per-frame cost; the
/// camera view-rect is drawn shell-side, so it still tracks every frame.
const REBUILD_THROTTLE: std::time::Duration = std::time::Duration::from_millis(100);

struct Cache {
	mode: Mode,
	revision: u64,
	size: (u32, u32),
	/// When this texture's content was last built - the throttle clock.
	built_at: std::time::Instant,
	bind_group: wgpu::BindGroup,
}

/// The identity + age of a cached minimap texture (the throttle inputs).
#[derive(Clone, Copy)]
struct CacheKey {
	mode: Mode,
	size: (u32, u32),
	revision: u64,
	built_at: std::time::Instant,
}

/// Decide, for a draw at `now`, whether to rebuild the source texture and
/// whether the content shown will be throttle-stale (so the shell schedules a
/// follow-up redraw). A mode/size change forces an immediate rebuild (a
/// wrong-shape texture can't be shown); a pure content (revision) change is
/// throttled - the existing texture is reused until [`REBUILD_THROTTLE`] has
/// elapsed since it was built. Pure, so the policy is tested without a GPU.
/// Returns `(rebuild, behind)`.
fn plan_rebuild(
	cached: Option<CacheKey>,
	mode: Mode,
	size: (u32, u32),
	revision: u64,
	now: std::time::Instant,
) -> (bool, bool) {
	let reusable = matches!(cached, Some(c) if c.mode == mode && c.size == size);
	let current = matches!(cached, Some(c) if c.mode == mode && c.size == size && c.revision == revision);
	let throttled = reusable && !current && cached.is_some_and(|c| now.duration_since(c.built_at) < REBUILD_THROTTLE);
	(!current && !throttled, throttled)
}

/// The minimap's source-texture cache; drawing goes through [`BlitPass`].
pub struct MinimapPass {
	cache: Option<Cache>,
	/// The last draw reused a texture older than the live revision because the
	/// rebuild was throttled. The shell reads this to schedule a follow-up
	/// redraw, so the minimap catches up shortly after the edits settle. Reset
	/// each frame (via [`Self::clear_followup`]) so a hidden panel can't pin it.
	behind: bool,
}

impl MinimapPass {
	pub fn new() -> Self {
		Self { cache: None, behind: false }
	}

	/// Drop the cached texture (document replaced).
	pub fn invalidate(&mut self) {
		self.cache = None;
	}

	/// Whether the last draw showed throttle-stale content and wants a
	/// follow-up redraw to catch up. Cleared by [`Self::clear_followup`].
	pub fn needs_followup(&self) -> bool {
		self.behind
	}

	/// Reset the follow-up flag before a frame; [`Self::draw`] re-sets it only
	/// if it actually throttles this frame, so a frame that doesn't draw the
	/// minimap (panel hidden) leaves it clear.
	pub fn clear_followup(&mut self) {
		self.behind = false;
	}

	/// Draw the minimap content into the panel body.
	pub fn draw(
		&mut self,
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		encoder: &mut wgpu::CommandEncoder,
		target: &wgpu::TextureView,
		blit: &BlitPass,
		editor: &EditorState,
		body: Rect,
		screen: (u32, u32),
		scale: f32,
	) {
		let mode = editor.minimap_mode;
		let (area, _) = map_area(editor.map_size(), body);
		let size = tex_size(editor, mode, area);
		if size.0 == 0 || size.1 == 0 {
			return;
		}

		let now = std::time::Instant::now();
		let revision = editor.revision();
		let key = self.cache.as_ref().map(|c| CacheKey {
			mode: c.mode,
			size: c.size,
			revision: c.revision,
			built_at: c.built_at,
		});
		let (rebuild, behind) = plan_rebuild(key, mode, size, revision, now);
		self.behind = behind;
		if rebuild {
			let rgba = build_rgba(editor, mode, size);
			let bind_group = blit.upload(device, queue, &rgba, size);
			self.cache = Some(Cache { mode, revision, size, built_at: now, bind_group });
		}
		blit.draw(
			device,
			encoder,
			target,
			&self.cache.as_ref().expect("cache built").bind_group,
			area,
			[0.0, 0.0, 1.0, 1.0],
			body,
			screen,
			scale,
		);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use map_core::Project;
	use std::path::{Path, PathBuf};

	fn resources() -> PathBuf {
		Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources")
	}

	fn editor() -> EditorState {
		let resources = resources();
		let project = Project::new(8, 6, &["GREEN".to_string()], &resources.join("assets/tilepacks"), 42).unwrap();
		EditorState::new(project, (800, 600), None, resources)
	}

	#[test]
	fn rebuild_is_throttled_between_revisions_but_not_on_shape_change() {
		use std::time::{Duration, Instant};
		let t0 = Instant::now();
		let within = t0 + REBUILD_THROTTLE / 2;
		let past = t0 + REBUILD_THROTTLE + Duration::from_millis(1);
		let key = |mode, size, rev| Some(CacheKey { mode, size, revision: rev, built_at: t0 });

		// No cache yet → build now, nothing stale.
		assert_eq!(plan_rebuild(None, Mode::Overworld, (64, 48), 1, t0), (true, false));
		// Same mode/size/revision → reuse, no rebuild, not behind.
		assert_eq!(
			plan_rebuild(key(Mode::Overworld, (64, 48), 1), Mode::Overworld, (64, 48), 1, within),
			(false, false)
		);
		// New revision within the throttle window → reuse the stale texture (behind).
		assert_eq!(
			plan_rebuild(key(Mode::Overworld, (64, 48), 1), Mode::Overworld, (64, 48), 2, within),
			(false, true)
		);
		// New revision after the window → rebuild once, no longer behind.
		assert_eq!(plan_rebuild(key(Mode::Overworld, (64, 48), 1), Mode::Overworld, (64, 48), 2, past), (true, false));
		// A size change rebuilds immediately even inside the window (wrong-shape
		// texture can't be shown), and is never reported as merely behind.
		assert_eq!(
			plan_rebuild(key(Mode::Overworld, (64, 48), 1), Mode::Overworld, (80, 60), 2, within),
			(true, false)
		);
		// A mode change likewise forces an immediate rebuild.
		assert_eq!(plan_rebuild(key(Mode::Overworld, (64, 48), 1), Mode::Pass, (64, 48), 1, within), (true, false));
	}

	#[test]
	fn map_area_letterboxes_and_centers() {
		let body = Rect::new(10.0, 30.0, 200.0, 300.0);
		// 8×6 map in a tall body: width-bound, vertically centered.
		let (area, scale) = map_area((8, 6), body);
		assert_eq!(scale, (200.0 - 2.0 * PAD) / 8.0);
		assert_eq!(area.w, 8.0 * scale);
		assert_eq!(area.h, 6.0 * scale);
		assert_eq!(area.x, body.x + PAD);
		let avail_top = body.y + HEADER_H + PAD;
		let avail_h = body.h - HEADER_H - 2.0 * PAD;
		assert!((area.y - (avail_top + (avail_h - area.h) / 2.0)).abs() < 0.01);
	}

	#[test]
	fn pan_target_clamps_and_round_trips() {
		let body = Rect::new(0.0, 0.0, 200.0, 300.0);
		let map = (8u16, 6u16);
		let (area, scale) = map_area(map, body);
		// A point at cell (2.5, 3.0) maps back to itself.
		let (px, py) = (area.x + 2.5 * scale, area.y + 3.0 * scale);
		let (tx, ty) = pan_target_in(map, area, px, py);
		assert!((tx - 2.5).abs() < 0.01 && (ty - 3.0).abs() < 0.01);
		// Way off the rect clamps to the map edge - which is what keeps a drag
		// that has left the panel tracking, now that the widget owns it.
		let (tx, ty) = pan_target_in(map, area, -999.0, 9999.0);
		assert_eq!((tx, ty), (0.0, 6.0));
	}

	#[test]
	fn mode_names_and_tags_round_trip() {
		assert_eq!(Mode::parse("pass"), Some(Mode::Pass));
		assert_eq!(Mode::parse("nope"), None);
		// The header key's action tag is its index in `Mode::ALL` — what the
		// shell maps a fired action back through.
		for (i, m) in Mode::ALL.iter().enumerate() {
			assert_eq!(MinimapOverlay::mode_of(i as u64), Some(*m));
		}
		assert_eq!(MinimapOverlay::mode_of(Mode::ALL.len() as u64), None);
	}

	/// The panel hosted in a `Ui` and laid out into `body` through the editor's
	/// real steel theme + fonts — the key row measures text now, so a bare
	/// `Fonts` is no longer enough (it was, while `radio_rect` hand-placed them).
	/// The chrome is returned so it outlives the borrow the layout took.
	fn laid_out(body: Rect, map: (u16, u16)) -> (crate::uikit_menu::MenuChrome, wgpu_ui::Ui, WidgetId) {
		let (_device, _queue, chrome) = crate::visual_test::chrome_fixture();
		let overlay = MinimapOverlay::default();
		let id = overlay.id();
		let mut ui = wgpu_ui::Ui::new(overlay);
		ui.get_mut::<MinimapOverlay>(id).unwrap().sync(Mode::Overworld, map, None);
		ui.layout_in(body, chrome.theme(), chrome.fonts());
		(chrome, ui, id)
	}

	fn button(pressed: bool, x: f32, y: f32) -> Event {
		Event::PointerButton {
			button: PointerButton::Primary,
			pressed,
			pos: Vec2::new(x, y),
			mods: wgpu_ui::Modifiers::NONE,
		}
	}

	/// **The U5.3 acceptance invariant.** The content widget reserves its rect
	/// from its own arranged rect, and `MinimapPass` computes one from the whole
	/// body — the native texture pass and the camera outline drawn over it would
	/// disagree by a pixel if the two ever drifted apart.
	#[test]
	fn the_content_widget_reserves_exactly_the_blitted_rect() {
		for body in [Rect::new(0.0, 24.0, 220.0, 300.0), Rect::new(7.0, 0.0, 260.0, 140.0)] {
			for map in [(8u16, 6u16), (112, 112), (1, 200)] {
				let (_chrome, ui, id) = laid_out(body, map);
				let overlay = ui.get::<MinimapOverlay>(id).expect("typed root");
				let view = wgpu_ui::descendant::<MinimapView>(&overlay.root, overlay.view_id).expect("the content");
				assert_eq!(view.area, map_area(map, body).0, "body {body:?} map {map:?}");
			}
		}
	}

	/// The three mode keys fire on release-inside, and their action tag is the
	/// mode. The header is chrome in the same tree as the content widget, never
	/// inside it (the §5.2 line).
	#[test]
	fn a_mode_key_fires_its_mode() {
		let body = Rect::new(0.0, 24.0, 220.0, 300.0);
		let (_chrome, mut ui, id) = laid_out(body, (8, 6));
		let key = ui.rect_of(ui.get::<MinimapOverlay>(id).unwrap().keys[1]).expect("key 1 is arranged");
		let (kx, ky) = (key.center().x, key.center().y);

		assert!(ui.dispatch(&[button(true, kx, ky)]).wants_pointer(), "the press is consumed");
		assert!(ui.actions().is_empty(), "and only arms");
		ui.dispatch(&[button(false, kx, ky)]);
		assert_eq!(ui.actions().len(), 1, "one key, one action");
		assert_eq!(MinimapOverlay::mode_of(ui.actions()[0]), Some(Mode::Pass));

		// The keys share the header width evenly and stay inside the band.
		let band = Rect::new(body.x, body.y, body.w, HEADER_H);
		for i in 0..3 {
			let r = ui.rect_of(ui.get::<MinimapOverlay>(id).unwrap().keys[i]).expect("arranged");
			assert!((r.w - key.w).abs() < 0.01, "key {i} is the same width");
			assert!(r.y >= band.y && r.y + r.h <= band.y + band.h, "key {i} stays in the header band");
		}
	}

	/// The header band follows the dock's width (the root column stretches its
	/// cross axis): in a dock narrower than three `button_min_width`s the keys
	/// shrink to share it instead of overflowing the panel's right edge.
	#[test]
	fn the_keys_follow_a_narrow_docks_width() {
		for w in [140.0, 220.0, 320.0] {
			let body = Rect::new(0.0, 24.0, w, 300.0);
			let (_chrome, ui, id) = laid_out(body, (8, 6));
			let keys = ui.get::<MinimapOverlay>(id).expect("typed root").keys;
			for (i, key) in keys.iter().enumerate() {
				let r = ui.rect_of(*key).expect("arranged");
				assert!(r.right() <= body.right() + 0.01, "key {i} stays inside a {w}px body, got {r:?}");
			}
			let last = ui.rect_of(keys[2]).expect("arranged");
			assert!((last.right() - (body.right() - 2.0)).abs() < 1.0, "the row spans the {w}px band, got {last:?}");
		}
	}

	/// The pan is the content widget's own drag now: the press captures the
	/// pointer and reports a target, every move while it holds reports a new
	/// one — **including moves far outside the panel**, which is what the old
	/// shell-side `minipan` rect existed to do — and the release lets go.
	#[test]
	fn a_pan_drag_captures_and_keeps_tracking_off_panel() {
		let body = Rect::new(0.0, 24.0, 220.0, 300.0);
		let map = (8u16, 6u16);
		let (_chrome, mut ui, id) = laid_out(body, map);
		let (area, scale) = map_area(map, body);
		let take = |ui: &mut wgpu_ui::Ui| ui.get_mut::<MinimapOverlay>(id).unwrap().take_pan();

		// A press inside the fitted map captures and aims.
		let resp = ui.dispatch(&[button(true, area.x + 2.0 * scale, area.y + 3.0 * scale)]);
		assert!(resp.wants_pointer(), "the press is consumed (it used to fall through)");
		assert!(resp.capturing, "and takes the pointer, so the shell keeps feeding this layer");
		let (tx, ty) = take(&mut ui).expect("the press aims");
		assert!((tx - 2.0).abs() < 0.01 && (ty - 3.0).abs() < 0.01);
		assert_eq!(take(&mut ui), None, "the target is taken once");

		// Dragged way off the panel: still tracking, clamped to the map edge.
		ui.dispatch(&[Event::PointerMoved { pos: Vec2::new(-500.0, 5000.0) }]);
		assert_eq!(take(&mut ui), Some((0.0, 6.0)), "a drag off-panel clamps and keeps panning");

		// The release ends it; later moves are nobody's.
		let resp = ui.dispatch(&[button(false, -500.0, 5000.0)]);
		assert!(!resp.capturing, "the drag hands the pointer back");
		ui.dispatch(&[Event::PointerMoved { pos: Vec2::new(area.x + scale, area.y + scale) }]);
		assert_eq!(take(&mut ui), None, "a move after the release pans nothing");
	}

	/// A press in the letterbox margin — inside the panel body, outside the
	/// fitted map — belongs to nobody, exactly as the old `click` oracle had it.
	#[test]
	fn the_letterbox_margin_is_inert() {
		let body = Rect::new(0.0, 24.0, 220.0, 300.0);
		let map = (8u16, 6u16);
		let (_chrome, mut ui, id) = laid_out(body, map);
		let (area, _) = map_area(map, body);
		assert!(area.y > body.y + HEADER_H + 1.0, "this body letterboxes vertically");

		let resp = ui.dispatch(&[button(true, area.x + 5.0, body.y + HEADER_H + 1.0)]);
		assert!(!resp.wants_pointer(), "the margin consumes nothing");
		assert!(!resp.capturing, "and starts no pan");
		assert_eq!(ui.get_mut::<MinimapOverlay>(id).unwrap().take_pan(), None);
	}

	/// Losing the window mid-drag ends it. Without this the release never
	/// arrives and the view would pan with the cursor forever — the hole G9
	/// found in `Slider`, which `Scroller` had already closed.
	#[test]
	fn window_focus_loss_ends_a_live_pan() {
		let body = Rect::new(0.0, 24.0, 220.0, 300.0);
		let map = (8u16, 6u16);
		let (_chrome, mut ui, id) = laid_out(body, map);
		let (area, scale) = map_area(map, body);

		ui.dispatch(&[button(true, area.x + scale, area.y + scale)]);
		ui.get_mut::<MinimapOverlay>(id).unwrap().take_pan();
		ui.dispatch(&[Event::Focus(false)]);
		ui.dispatch(&[Event::PointerMoved { pos: Vec2::new(area.x + 3.0 * scale, area.y + scale) }]);
		assert_eq!(ui.get_mut::<MinimapOverlay>(id).unwrap().take_pan(), None, "the stranded drag is over");
	}

	#[test]
	fn pass_texture_is_all_water_on_a_fresh_map() {
		let e = editor();
		let rgba = build_rgba(&e, Mode::Pass, (8, 6));
		assert_eq!(rgba.len(), 8 * 6 * 4);
		for px in rgba.chunks_exact(4) {
			assert_eq!(px, PASS_RGBA[1], "fresh map is water everywhere");
		}
	}

	#[test]
	fn minimap_texture_uses_palette_colors() {
		let e = editor();
		let p = &e.project;
		let rgba = build_rgba(&e, Mode::Minimap, (8, 6));
		let index = p.minimap_pixel(0, 0) as usize;
		assert_eq!(&rgba[0..3], &p.palette[index * 3..index * 3 + 3]);
	}

	#[test]
	fn overworld_texture_samples_the_composed_world() {
		let e = editor();
		let p = &e.project;
		let rgba = build_rgba(&e, Mode::Overworld, (16, 12));
		// Texel (0,0) samples world center of its footprint: cell (0,0),
		// sub (16,16) for a 16×12 texture over an 8×6 map.
		let index = p.pixel_at(0, 0, (16, 16)) as usize;
		assert_eq!(&rgba[0..3], &p.palette[index * 3..index * 3 + 3]);
	}
}
