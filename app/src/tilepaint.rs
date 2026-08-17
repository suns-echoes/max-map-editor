//! Tile Painter pieces: the editor-owned run state, the pure paint-tool
//! semantics, and the two custom wgpu-ui widgets the dialog composes
//! ([`PixelCanvas`] + [`SwatchGrid`]).
//!
//! The dialog itself is built in [`crate::uikit_overlay`] out of stock wgpu-ui
//! bricks plus these widgets; all interactive paint state (working canvas,
//! picked color, armed tool) lives dialog-side like any other dialog's fields.
//! [`TilePaintRun`] on [`crate::state::EditorState`] carries only what command
//! paths need outside a frame: the commit context (mode/ids/pack/mask) and a
//! canvas mirror the shell re-syncs after every edit, so `tile-commit`,
//! PNG export, and PNG import read/write current pixels without reaching into
//! the widget tree.
//!
//! The art is texture-based: the 64×64 canvas and the 16×16 palette grid are
//! composed to RGBA through the live (cycled) palette table and re-uploaded on
//! change — with "animate colors" on, the shell keeps ticking the shared
//! [`crate::palette::PaletteCycler`], so the preview shimmers exactly as the
//! game would show it.

use wgpu_ui::widget::{DrawCtx, EventCtx, LayoutCtx};
use wgpu_ui::{
	DrawList, Event, PointerButton, Rect, Rgba, ScrollDelta, Size, TexRect, TextureId, Vec2, Widget, WidgetId,
	WidgetState,
};

/// One tile is 64×64 palette indices.
pub const TILE: usize = 64;

/// The square canvas viewport (logical px). 384 = 64 × 6, so 600 % fills it.
pub const WELL: f32 = 384.0;

/// Palette swatch size; 16 × 18 = 288-px grid.
pub const SW: f32 = 18.0;

/// Zoom levels: screen px per tile pixel, and the label. At 600 % a 64-px tile
/// fills the [`WELL`]-px canvas viewport exactly (64 × 6 = 384).
pub const ZOOMS: [(f32, &str); 4] = [(1.0, "100%"), (2.0, "200%"), (4.0, "400%"), (6.0, "600%")];

/// Passability values + labels (0 land / 1 water / 2 shore / 3 blocked).
pub const PASSES: [&str; 4] = ["land", "water", "shore", "blocked"];

/// Why the painter was opened - shapes the title and what Save commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
	/// A blank tile in a chosen pack.
	New,
	/// A copy of the selected tile (a new tile, same pack family).
	Clone,
	/// The selected tile, edited in place (stock tiles need `--dev`).
	Edit,
}

impl Mode {
	pub fn title(self) -> &'static str {
		match self {
			Mode::New => "New Tile",
			Mode::Clone => "Clone Tile",
			Mode::Edit => "Edit Tile",
		}
	}
}

/// A valid character for a tile id (ascii letters, digits, `_`).
pub fn is_id_char(c: char) -> bool {
	c.is_ascii_alphanumeric() || c == '_'
}

/// The editor-owned Tile Painter context — `Some` while the dialog is open.
/// The dialog holds the *working* canvas; the shell mirrors it back here after
/// every edited frame, so command paths (`tile-commit`, PNG export) read
/// current pixels and PNG import writes them (bumping [`Self::canvas_rev`] so
/// the dialog re-syncs its copy).
pub struct TilePaintRun {
	pub mode: Mode,
	/// The source/target tile id (the id being edited, or the one cloned from).
	pub tile_id: String,
	/// The pack the source tile belongs to (fixed for Edit/Clone).
	pub pack_name: String,
	/// The family's transparency mask color, if any - the pixel value drawn
	/// see-through, matching the map. `None` = fully opaque (new tiles and
	/// non-shore families).
	pub mask: Option<u8>,
	/// The 64×64 canvas mirror (palette indices, row-major).
	pub canvas: Vec<u8>,
	/// Bumped when the EDITOR writes `canvas` (PNG import); the dialog re-syncs
	/// its working copy when the revision moves.
	pub canvas_rev: u64,
	/// Passability 0..=3 (the initial value; also the script-path commit value).
	pub pass: u8,
	/// The id-field text (initial value; re-stamped by the dialog on export so
	/// the save dialog can suggest `<id>.png`; the script-path commit id).
	pub id_text: String,
	/// Target packs to choose from in [`Mode::New`] (pack names).
	pub packs: Vec<String>,
}

impl TilePaintRun {
	/// The pack a script-path commit targets: the first choice in New mode
	/// (the dialog's dropdown default), else the source pack.
	pub fn target_pack(&self) -> &str {
		match self.mode {
			Mode::New => self.packs.first().map(String::as_str).unwrap_or(""),
			_ => &self.pack_name,
		}
	}
}

// ----- pure paint-tool semantics ---------------------------------------------

/// The dialog's live tool state, mutated by [`apply_canvas_event`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PaintState {
	/// The picked paint color (palette index).
	pub color: u8,
	/// Eyedropper armed: the next canvas press samples its color instead of
	/// painting (one-shot, like a real eyedropper).
	pub eyedrop: bool,
	/// Replace-color armed: a canvas press recolors every pixel of the pressed
	/// color to the current paint color (stays armed until toggled off).
	pub replace: bool,
	/// The current stroke paints (set by a plain press; an eyedrop/replace
	/// press starts no stroke, so its drag never paints).
	pub stroke: bool,
}

/// A pixel-level pointer event reported by [`PixelCanvas`], in canvas
/// coordinates (`0..TILE`). Order matters: a `Drag` belongs to the last
/// `Press`'s stroke decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasEvent {
	Press(u16, u16),
	Drag(u16, u16),
}

/// Applies one canvas event to the working `canvas` under `st`'s armed tool -
/// the legacy painter's exact press semantics. Returns `true` when the canvas
/// changed (the caller re-uploads the texture).
pub fn apply_canvas_event(canvas: &mut [u8], st: &mut PaintState, ev: CanvasEvent) -> bool {
	match ev {
		CanvasEvent::Press(px, py) => {
			let under = canvas[py as usize * TILE + px as usize];
			if st.eyedrop {
				st.color = under;
				st.eyedrop = false; // one-shot
				st.stroke = false;
				false
			} else if st.replace {
				st.stroke = false;
				let mut changed = false;
				for p in canvas.iter_mut() {
					if *p == under && *p != st.color {
						*p = st.color;
						changed = true;
					}
				}
				changed
			} else {
				st.stroke = true;
				paint(canvas, st.color, px, py)
			}
		}
		CanvasEvent::Drag(px, py) => st.stroke && paint(canvas, st.color, px, py),
	}
}

fn paint(canvas: &mut [u8], color: u8, px: u16, py: u16) -> bool {
	let p = &mut canvas[py as usize * TILE + px as usize];
	let changed = *p != color;
	*p = color;
	changed
}

// ----- texture composition -----------------------------------------------------

/// The 64×64 canvas as RGBA through the live palette table (256×4 sRGB bytes):
/// the family's `mask` color is transparent (the well shows through, matching
/// the map), everything else opaque - so a land tile's index-0 pixels show,
/// exactly as on the map.
pub fn compose_canvas_rgba(canvas: &[u8], rgba: &[u8], mask: Option<u8>) -> Vec<u8> {
	let mut out = Vec::with_capacity(canvas.len() * 4);
	for &i in canvas {
		if Some(i) == mask {
			out.extend_from_slice(&[0, 0, 0, 0]);
		} else {
			let o = i as usize * 4;
			out.extend_from_slice(&[rgba[o], rgba[o + 1], rgba[o + 2], 255]);
		}
	}
	out
}

/// The 16×16 palette grid as RGBA (one texel per slot, row-major). Every slot
/// is opaque, so index 0 is still pickable/visible.
pub fn compose_swatches_rgba(rgba: &[u8]) -> Vec<u8> {
	let mut out = Vec::with_capacity(256 * 4);
	for i in 0..256 {
		let o = i * 4;
		out.extend_from_slice(&[rgba[o], rgba[o + 1], rgba[o + 2], 255]);
	}
	out
}

// ----- PixelCanvas -------------------------------------------------------------

/// The paintable canvas viewport: a recessed [`WELL`]-px well with the tile's
/// composed texture centered in it at the current zoom. A dumb brick - it
/// reports pixel-level presses/drags (drained by the dialog, which applies the
/// armed tool) plus wheel steps, and rings the hovered pixel.
pub struct PixelCanvas {
	id: WidgetId,
	tex: TextureId,
	/// Screen px per tile pixel (a [`ZOOMS`] entry).
	px: f32,
	/// The canvas pixel under the cursor (the hover cue + the swatch hint).
	hover: Option<(u16, u16)>,
	/// A press started inside the art (drags then report until release).
	pressed: bool,
	events: Vec<CanvasEvent>,
	/// Accumulated wheel notches over the canvas (the dialog steps the zoom).
	wheel: f32,
	rect: Rect,
}

impl PixelCanvas {
	pub fn new(tex: TextureId) -> Self {
		Self {
			id: wgpu_ui::interact::next_id(),
			tex,
			px: ZOOMS[ZOOMS.len() - 1].0, // start at 600 % (fills the well)
			hover: None,
			pressed: false,
			events: Vec::new(),
			wheel: 0.0,
			rect: Rect::ZERO,
		}
	}

	pub fn id(&self) -> WidgetId {
		self.id
	}

	pub fn set_zoom(&mut self, px: f32) {
		self.px = px;
	}

	/// The canvas pixel under the cursor, if any.
	pub fn hover(&self) -> Option<(u16, u16)> {
		self.hover
	}

	/// Drains this frame's pixel events (presses + stroke drags, in order).
	pub fn take_events(&mut self) -> Vec<CanvasEvent> {
		std::mem::take(&mut self.events)
	}

	/// Drains the accumulated wheel notches (positive = away from the user).
	pub fn take_wheel(&mut self) -> f32 {
		std::mem::take(&mut self.wheel)
	}

	/// The painted area's rect: `TILE × px` square, centered in the well.
	fn art_rect(&self) -> Rect {
		let size = TILE as f32 * self.px;
		Rect::new(self.rect.x + (self.rect.w - size) / 2.0, self.rect.y + (self.rect.h - size) / 2.0, size, size)
	}

	fn pixel_at(&self, pos: Vec2) -> Option<(u16, u16)> {
		pixel_at(self.art_rect(), self.px, pos)
	}

	fn pixel_rect(&self, px: u16, py: u16) -> Rect {
		let art = self.art_rect();
		Rect::new(art.x + px as f32 * self.px, art.y + py as f32 * self.px, self.px, self.px)
	}
}

/// The canvas pixel at `pos` for an `art` rect drawn at `px` screen px per
/// tile pixel (kept free for tests).
pub fn pixel_at(art: Rect, px: f32, pos: Vec2) -> Option<(u16, u16)> {
	let (cx, cy) = (((pos.x - art.x) / px).floor(), ((pos.y - art.y) / px).floor());
	if cx < 0.0 || cy < 0.0 || cx >= TILE as f32 || cy >= TILE as f32 {
		return None;
	}
	Some((cx as u16, cy as u16))
}

impl Widget for PixelCanvas {
	fn measure(&mut self, _avail: Size, _ctx: &mut LayoutCtx) -> Size {
		Size::new(WELL, WELL)
	}

	fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
		self.rect = rect;
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		if !ctx.is_base() {
			return;
		}
		ctx.theme.well(dl, self.rect, WidgetState::default());
		let art = self.art_rect();
		dl.image(self.tex, art, TexRect::FULL, Rgba::WHITE);
		dl.stroke_rect(art, 1.0, ctx.theme.ink_dim());
		if let Some((px, py)) = self.hover {
			dl.stroke_rect(self.pixel_rect(px, py), 1.0, ctx.theme.ink());
		}
	}

	fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
		match ev {
			Event::PointerMoved { pos } => {
				let over = ctx.is_hovered(self.id) || self.pressed;
				self.hover = if over { self.pixel_at(*pos) } else { None };
				if self.pressed {
					if let Some((px, py)) = self.hover {
						self.events.push(CanvasEvent::Drag(px, py));
					}
					ctx.consume_pointer();
					return true;
				}
				false
			}
			Event::PointerButton { button: PointerButton::Primary, pressed: true, pos, .. }
				if ctx.is_target(self.id) =>
			{
				if let Some((px, py)) = self.pixel_at(*pos) {
					self.pressed = true;
					self.events.push(CanvasEvent::Press(px, py));
					ctx.capture(self.id);
				}
				ctx.consume_pointer();
				true
			}
			Event::PointerButton { button: PointerButton::Primary, pressed: false, .. } if self.pressed => {
				self.pressed = false;
				ctx.consume_pointer();
				true
			}
			Event::Scroll { delta, .. } if ctx.is_target(self.id) => {
				self.wheel += match delta {
					ScrollDelta::Lines(v) => v.y,
					ScrollDelta::Pixels(v) => v.y / 40.0,
				};
				ctx.consume_pointer();
				true
			}
			Event::PointerLeft => {
				self.hover = None;
				false
			}
			// The release will never arrive (the window lost focus mid-stroke),
			// so end the drag here — the `MinimapView`/`BlockBar` contract.
			// `pressed` keeps hover alive through the `PointerMoved` arm above,
			// so a stuck stroke would paint on every later move with no button
			// down.
			Event::Focus(false) => {
				self.pressed = false;
				self.hover = None;
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

// ----- SwatchGrid ----------------------------------------------------------------

/// The 16×16 palette picker: one 16×16 texel texture stretched to `16 ×`[`SW`]
/// (NEAREST keeps each texel a crisp swatch). A press picks the swatch under
/// the cursor and fires; the selected swatch is ringed in accent, and `hint`
/// (the slot used by the hovered canvas pixel) in ink - the design's hover cue.
pub struct SwatchGrid {
	id: WidgetId,
	tex: TextureId,
	sel: u8,
	hint: Option<u8>,
	rect: Rect,
}

impl SwatchGrid {
	pub fn new(tex: TextureId, sel: u8) -> Self {
		Self { id: wgpu_ui::interact::next_id(), tex, sel, hint: None, rect: Rect::ZERO }
	}

	pub fn id(&self) -> WidgetId {
		self.id
	}

	/// The picked palette index (read after a fire; kept in sync by the host
	/// when the eyedropper samples).
	pub fn sel(&self) -> u8 {
		self.sel
	}

	pub fn set_sel(&mut self, sel: u8) {
		self.sel = sel;
	}

	/// Rings the swatch a hovered canvas pixel uses (`None` clears it).
	pub fn set_hint(&mut self, hint: Option<u8>) {
		self.hint = hint;
	}

	fn swatch_rect(&self, i: u8) -> Rect {
		let (col, row) = ((i % 16) as f32, (i / 16) as f32);
		Rect::new(self.rect.x + col * SW, self.rect.y + row * SW, SW, SW)
	}

	fn swatch_at(&self, pos: Vec2) -> Option<u8> {
		if !self.rect.contains(pos) {
			return None;
		}
		let col = ((pos.x - self.rect.x) / SW).floor() as i32;
		let row = ((pos.y - self.rect.y) / SW).floor() as i32;
		if !(0..16).contains(&col) || !(0..16).contains(&row) {
			return None;
		}
		Some((row * 16 + col) as u8)
	}
}

impl Widget for SwatchGrid {
	fn measure(&mut self, _avail: Size, _ctx: &mut LayoutCtx) -> Size {
		Size::new(16.0 * SW, 16.0 * SW)
	}

	fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
		self.rect = rect;
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		if !ctx.is_base() {
			return;
		}
		dl.image(self.tex, self.rect, TexRect::FULL, Rgba::WHITE);
		if let Some(hint) = self.hint {
			if hint != self.sel {
				dl.stroke_rect(self.swatch_rect(hint), 1.0, ctx.theme.ink());
			}
		}
		dl.stroke_rect(self.swatch_rect(self.sel), 1.0, ctx.theme.accent());
	}

	fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
		// Commit on press - the palette-swatch gesture (like ColorButton's
		// CommitPolicy::Press), so a paint stroke can start right after.
		if let Event::PointerButton { button: PointerButton::Primary, pressed: true, pos, .. } = ev {
			if ctx.is_target(self.id) {
				if let Some(i) = self.swatch_at(*pos) {
					self.sel = i;
					ctx.fire(self.id, None);
				}
				ctx.consume_pointer();
				return true;
			}
		}
		false
	}

	fn rect(&self) -> Rect {
		self.rect
	}

	fn id(&self) -> WidgetId {
		self.id
	}
}

// ----- Chip ------------------------------------------------------------------------

/// A small recessed swatch showing the current paint color (display-only; the
/// host re-syncs the color each frame through [`Chip::set_color`], reaching it
/// by id).
pub struct Chip {
	id: WidgetId,
	color: Rgba,
	size: f32,
	rect: Rect,
}

impl Chip {
	pub fn new(color: Rgba, size: f32) -> Self {
		Self { id: wgpu_ui::interact::next_id(), color, size, rect: Rect::ZERO }
	}

	pub fn id(&self) -> WidgetId {
		self.id
	}

	pub fn set_color(&mut self, color: Rgba) {
		self.color = color;
	}
}

impl Widget for Chip {
	fn measure(&mut self, _avail: Size, _ctx: &mut LayoutCtx) -> Size {
		Size::new(self.size, self.size)
	}

	fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
		self.rect = rect;
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		if !ctx.is_base() {
			return;
		}
		ctx.theme.well(dl, self.rect, WidgetState::default());
		dl.fill_rect(self.rect.inset(wgpu_ui::Insets::all(2.0)), self.color);
	}

	fn rect(&self) -> Rect {
		self.rect
	}

	fn id(&self) -> WidgetId {
		self.id
	}
}

/// A palette slot's opaque color from the live table ([`Rgba`] carries sRGB
/// bytes; the toolkit shader decodes to linear).
pub fn slot_color(rgba: &[u8], i: u8) -> Rgba {
	let o = i as usize * 4;
	Rgba::rgb(rgba[o], rgba[o + 1], rgba[o + 2])
}

#[cfg(test)]
mod tests {
	use super::*;
	use wgpu_ui::{DrawCmd, FontId, Fonts, Gunmetal, Modifiers, Theme, Ui};

	fn flat_palette() -> Vec<u8> {
		(0..256).flat_map(|i| [i as u8, i as u8, i as u8, 255]).collect()
	}

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
		Event::PointerMoved { pos: Vec2::new(x, y) }
	}

	/// A headless `Ui` hosting one widget arranged into `rect` (a bare theme +
	/// empty fonts suffice: these widgets draw no text), the tabs.rs pattern.
	fn host(w: impl Widget + 'static, rect: Rect) -> (Ui, Gunmetal, Fonts) {
		let mut ui = Ui::new(w);
		let (fonts, theme) = (Fonts::new(), Gunmetal::new(FontId(0)));
		ui.layout_in(rect, &theme, &fonts);
		(ui, theme, fonts)
	}

	/// Whether the list holds a solid of exactly this rect and color (ring edges
	/// are 1px solids, so a ring's top edge identifies it).
	fn solid_at(dl: &DrawList, rect: Rect, color: Rgba) -> bool {
		dl.cmds.iter().any(|c| matches!(c, DrawCmd::Solid { rect: r, color: k } if *r == rect && *k == color))
	}

	fn image_at(dl: &DrawList, rect: Rect) -> bool {
		dl.cmds.iter().any(|c| matches!(c, DrawCmd::Image { rect: r, .. } if *r == rect))
	}

	#[test]
	fn paint_press_and_drag_write_the_picked_color() {
		let mut canvas = vec![0u8; TILE * TILE];
		let mut st = PaintState { color: 42, ..Default::default() };
		assert!(apply_canvas_event(&mut canvas, &mut st, CanvasEvent::Press(0, 0)));
		assert_eq!(canvas[0], 42);
		assert!(st.stroke, "a plain press starts a paint stroke");
		assert!(apply_canvas_event(&mut canvas, &mut st, CanvasEvent::Drag(1, 0)));
		assert_eq!(canvas[1], 42);
		// Repainting the same color reports no change (no texture re-upload).
		assert!(!apply_canvas_event(&mut canvas, &mut st, CanvasEvent::Drag(1, 0)));
	}

	#[test]
	fn eyedropper_samples_once_and_its_drag_never_paints() {
		let mut canvas = vec![0u8; TILE * TILE];
		canvas[0] = 7;
		let mut st = PaintState { color: 3, eyedrop: true, ..Default::default() };
		assert!(!apply_canvas_event(&mut canvas, &mut st, CanvasEvent::Press(0, 0)));
		assert_eq!(st.color, 7, "sampled the pixel under the cursor");
		assert!(!st.eyedrop, "eyedropper is one-shot");
		// The drag belonging to the sampling press paints nothing.
		assert!(!apply_canvas_event(&mut canvas, &mut st, CanvasEvent::Drag(1, 0)));
		assert_eq!(canvas[1], 0);
		// The next plain press paints with the sampled color.
		assert!(apply_canvas_event(&mut canvas, &mut st, CanvasEvent::Press(2, 0)));
		assert_eq!(canvas[2], 7);
	}

	#[test]
	fn replace_recolors_every_matching_pixel_and_stays_armed() {
		let mut canvas = vec![0u8; TILE * TILE];
		canvas[5] = 3;
		canvas[9] = 3;
		let mut st = PaintState { color: 8, replace: true, ..Default::default() };
		assert!(apply_canvas_event(&mut canvas, &mut st, CanvasEvent::Press(5, 0)));
		assert_eq!((canvas[5], canvas[9]), (8, 8), "all pixels of the pressed color recolored");
		assert_eq!(canvas[0], 0, "other colors untouched");
		assert!(st.replace, "replace stays armed until toggled off");
		assert!(!st.stroke, "a replace press starts no paint stroke");
	}

	#[test]
	fn canvas_rgba_skips_only_the_mask_color() {
		let pal = flat_palette();
		let mut canvas = vec![0u8; TILE * TILE];
		canvas[1] = 5;
		let masked = compose_canvas_rgba(&canvas, &pal, Some(0));
		assert_eq!(masked[3], 0, "mask pixels are transparent");
		assert_eq!(&masked[4..8], &[5, 5, 5, 255], "non-mask pixels are opaque");
		let opaque = compose_canvas_rgba(&canvas, &pal, None);
		assert_eq!(opaque[3], 255, "no mask: index 0 draws, as the map renders opaque families");
		let swatches = compose_swatches_rgba(&pal);
		assert_eq!(swatches.len(), 256 * 4);
		assert_eq!(swatches[3], 255, "slot 0 is opaque in the picker");
	}

	#[test]
	fn pixel_at_maps_screen_to_canvas() {
		let art = Rect::new(100.0, 50.0, 384.0, 384.0);
		assert_eq!(pixel_at(art, 6.0, Vec2::new(100.0, 50.0)), Some((0, 0)));
		assert_eq!(pixel_at(art, 6.0, Vec2::new(105.9, 55.9)), Some((0, 0)));
		assert_eq!(pixel_at(art, 6.0, Vec2::new(106.0, 50.0)), Some((1, 0)));
		assert_eq!(pixel_at(art, 6.0, Vec2::new(100.0 + 63.0 * 6.0, 50.0 + 63.0 * 6.0)), Some((63, 63)));
		assert_eq!(pixel_at(art, 6.0, Vec2::new(99.9, 50.0)), None, "left of the art");
		assert_eq!(pixel_at(art, 6.0, Vec2::new(100.0 + 384.0, 50.0)), None, "right of the art");
	}

	#[test]
	fn run_target_pack_prefers_the_first_new_mode_choice() {
		let run = TilePaintRun {
			mode: Mode::New,
			tile_id: String::new(),
			pack_name: String::new(),
			mask: None,
			canvas: vec![0; TILE * TILE],
			canvas_rev: 0,
			pass: 0,
			id_text: String::new(),
			packs: vec!["GREEN".into(), "DESERT".into()],
		};
		assert_eq!(run.target_pack(), "GREEN");
		let edit = TilePaintRun { mode: Mode::Edit, pack_name: "DESERT".into(), packs: Vec::new(), ..run };
		assert_eq!(edit.target_pack(), "DESERT");
	}

	#[test]
	fn mode_titles_and_id_chars() {
		// The dialog title names the gesture that opened the painter.
		assert_eq!(Mode::New.title(), "New Tile");
		assert_eq!(Mode::Clone.title(), "Clone Tile");
		assert_eq!(Mode::Edit.title(), "Edit Tile");
		// The id filter admits exactly [A-Za-z0-9_] (the tile-id alphabet).
		assert!(is_id_char('a') && is_id_char('Z') && is_id_char('0') && is_id_char('_'));
		assert!(!is_id_char('-') && !is_id_char(' ') && !is_id_char('.'), "punctuation never enters a tile id");
	}

	/// The release never arrives when the window loses focus mid-stroke: the
	/// drag must end on `Focus(false)` (the `MinimapView`/`BlockBar` contract),
	/// or `pressed` keeps the hover alive and every later move paints with no
	/// button down.
	#[test]
	fn pixel_canvas_ends_its_stroke_on_focus_loss() {
		let canvas = PixelCanvas::new(TextureId::ATLAS);
		let id = canvas.id();
		let (mut ui, _theme, _fonts) = host(canvas, Rect::new(0.0, 0.0, WELL, WELL));

		ui.dispatch(&[press(3.0, 9.0)]);
		ui.get_mut::<PixelCanvas>(id).unwrap().take_events(); // the press's stroke start
		ui.dispatch(&[Event::Focus(false)]);
		// Refocused, the pointer moves with no button down: nothing paints.
		ui.dispatch(&[Event::Focus(true), moved(10.0, 9.0)]);
		let c = ui.get_mut::<PixelCanvas>(id).unwrap();
		assert!(c.take_events().is_empty(), "no phantom drag after focus loss");
	}

	#[test]
	fn pixel_canvas_reports_strokes_wheel_and_hover() {
		let canvas = PixelCanvas::new(TextureId::ATLAS);
		let id = canvas.id();
		let (mut ui, theme, fonts) = host(canvas, Rect::new(0.0, 0.0, WELL, WELL));

		// A plain move tracks the pixel under the cursor and reports no stroke.
		ui.dispatch(&[moved(3.0, 9.0)]);
		assert_eq!(ui.get_mut::<PixelCanvas>(id).unwrap().hover(), Some((0, 1)), "600%: 6 screen px per canvas px");
		assert!(ui.get_mut::<PixelCanvas>(id).unwrap().take_events().is_empty(), "hovering paints nothing");

		// The hovered pixel is ringed in ink; the art fills the well at 600%.
		let mut dl = DrawList::new();
		ui.draw(&mut dl, &theme, &fonts);
		assert!(solid_at(&dl, Rect::new(0.0, 6.0, 6.0, 1.0), theme.ink()), "hover ring on pixel (0,1)");
		assert!(image_at(&dl, Rect::new(0.0, 0.0, WELL, WELL)), "the 64px tile at 600% fills the 384px well");

		// Press starts a stroke; captured drags report pixels, off-art drags don't.
		ui.dispatch(&[press(3.0, 9.0), moved(10.0, 9.0), moved(-10.0, 9.0), release(10.0, 9.0)]);
		let ev = ui.get_mut::<PixelCanvas>(id).unwrap().take_events();
		assert_eq!(ev, vec![CanvasEvent::Press(0, 1), CanvasEvent::Drag(1, 1)], "press + on-art drag; off-art dropped");

		// Wheel notches accumulate (lines 1:1, pixels /40) and drain once.
		let at = Vec2::new(10.0, 9.0);
		ui.dispatch(&[
			Event::Scroll { delta: ScrollDelta::Lines(Vec2::new(0.0, 2.0)), pos: at, mods: Modifiers::NONE },
			Event::Scroll { delta: ScrollDelta::Pixels(Vec2::new(0.0, -40.0)), pos: at, mods: Modifiers::NONE },
		]);
		let c = ui.get_mut::<PixelCanvas>(id).unwrap();
		assert_eq!(c.take_wheel(), 1.0, "2 notches - 40 px / 40 = 1");
		assert_eq!(c.take_wheel(), 0.0, "the wheel drains once");

		// Leaving the window clears the hover cue; unrelated events change nothing.
		ui.dispatch(&[Event::PointerLeft, Event::Focus(true)]);
		let c = ui.get_mut::<PixelCanvas>(id).unwrap();
		assert_eq!(c.hover(), None, "pointer left: no hovered pixel");
		assert!(c.take_events().is_empty());

		// set_zoom rescales the art (still centered in the well) and the mapping.
		ui.get_mut::<PixelCanvas>(id).unwrap().set_zoom(1.0);
		ui.dispatch(&[moved(192.0, 192.0)]);
		assert_eq!(ui.get_mut::<PixelCanvas>(id).unwrap().hover(), Some((32, 32)), "100%: the well center is (32,32)");
		let mut dl = DrawList::new();
		ui.draw(&mut dl, &theme, &fonts);
		assert!(image_at(&dl, Rect::new(160.0, 160.0, 64.0, 64.0)), "100%: the 64px art centers in the well");
	}

	#[test]
	fn swatch_grid_picks_on_press_and_ignores_the_margin() {
		let grid = SwatchGrid::new(TextureId::ATLAS, 3);
		let id = grid.id();
		// Arranged into a slot larger than the 288px art: the margin is dead.
		let (mut ui, _theme, _fonts) = host(grid, Rect::new(0.0, 0.0, 320.0, 320.0));
		assert_eq!(ui.get_mut::<SwatchGrid>(id).unwrap().sel(), 3, "starts on the given slot");

		// Press on col 3 / row 2 → slot 35 picked, committed on the press.
		ui.dispatch(&[press(63.0, 45.0)]);
		assert!(ui.fired(id), "a swatch press fires immediately, so a paint stroke can start right after");
		assert_eq!(ui.get_mut::<SwatchGrid>(id).unwrap().sel(), 35);
		ui.dispatch(&[release(63.0, 45.0)]);

		// A press in the dead margin right of column 15 picks nothing.
		ui.dispatch(&[press(300.0, 10.0)]);
		assert!(!ui.fired(id), "the margin past the last column is not a swatch");
		assert_eq!(ui.get_mut::<SwatchGrid>(id).unwrap().sel(), 35, "selection unchanged");
		ui.dispatch(&[release(300.0, 10.0)]);

		let g = ui.get::<SwatchGrid>(id).unwrap();
		assert_eq!(g.rect(), Rect::new(0.0, 0.0, 320.0, 320.0), "the widget reports its arranged rect");
		// swatch_at is the hit map: outside the rect / past the 16×16 grid → None.
		assert_eq!(g.swatch_at(Vec2::new(-1.0, 5.0)), None, "left of the grid");
		assert_eq!(g.swatch_at(Vec2::new(10.0, 300.0)), None, "below row 15");
		assert_eq!(g.swatch_at(Vec2::new(9.0, 9.0)), Some(0));
		let c = g.swatch_rect(200);
		assert_eq!(g.swatch_at(Vec2::new(c.x + SW / 2.0, c.y + SW / 2.0)), Some(200), "swatch_rect round-trips");
	}

	#[test]
	fn swatch_grid_rings_the_hint_unless_it_is_the_selection() {
		let grid = SwatchGrid::new(TextureId::ATLAS, 35);
		let id = grid.id();
		let (mut ui, theme, fonts) = host(grid, Rect::new(0.0, 0.0, 16.0 * SW, 16.0 * SW));

		// Slot 7 (col 7, row 0) hints in ink; slot 35 (col 3, row 2) rings in accent.
		ui.get_mut::<SwatchGrid>(id).unwrap().set_hint(Some(7));
		let mut dl = DrawList::new();
		ui.draw(&mut dl, &theme, &fonts);
		assert!(solid_at(&dl, Rect::new(7.0 * SW, 0.0, SW, 1.0), theme.ink()), "the hovered pixel's slot rings in ink");
		assert!(solid_at(&dl, Rect::new(3.0 * SW, 2.0 * SW, SW, 1.0), theme.accent()), "the selection rings in accent");

		// A hint on the selected swatch collapses into the accent ring.
		ui.get_mut::<SwatchGrid>(id).unwrap().set_hint(Some(35));
		let mut dl = DrawList::new();
		ui.draw(&mut dl, &theme, &fonts);
		let ink = theme.ink();
		assert!(
			!dl.cmds.iter().any(|c| matches!(c, DrawCmd::Solid { color, .. } if *color == ink)),
			"no ink ring when the hint is the selection"
		);
		assert!(solid_at(&dl, Rect::new(3.0 * SW, 2.0 * SW, SW, 1.0), theme.accent()), "the accent ring stays");
	}

	#[test]
	fn chip_paints_its_color_inside_the_well() {
		let chip = Chip::new(slot_color(&flat_palette(), 9), 24.0);
		let id = chip.id();
		let (mut ui, theme, fonts) = host(chip, Rect::new(5.0, 6.0, 24.0, 24.0));
		assert_eq!(ui.get::<Chip>(id).unwrap().rect(), Rect::new(5.0, 6.0, 24.0, 24.0), "reports the arranged rect");
		let mut dl = DrawList::new();
		ui.draw(&mut dl, &theme, &fonts);
		assert!(solid_at(&dl, Rect::new(7.0, 8.0, 20.0, 20.0), Rgba::rgb(9, 9, 9)), "the color pad is inset 2px");
		// The host re-syncs the color each frame; the next draw shows it.
		ui.get_mut::<Chip>(id).unwrap().set_color(Rgba::rgb(1, 2, 3));
		let mut dl = DrawList::new();
		ui.draw(&mut dl, &theme, &fonts);
		assert!(solid_at(&dl, Rect::new(7.0, 8.0, 20.0, 20.0), Rgba::rgb(1, 2, 3)), "set_color re-tints the pad");
	}
}
