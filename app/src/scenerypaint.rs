//! New Scenery: the editor-owned run state, the pure recolour semantics, and
//! the two custom widgets the dialog composes ([`SpriteView`] + [`SubPalette`]).
//!
//! The dialog itself is built in [`crate::uikit_overlay`] out of stock wgpu-ui
//! bricks plus these widgets, exactly as the Tile Painter is; all interactive
//! state (the derived piece, the recolour map, the selection) lives dialog-side
//! like any other dialog's fields. [`SceneryPaintRun`] on
//! [`crate::state::EditorState`] carries only what command paths need outside a
//! frame: the destination pack list, the source image, and the thresholds it
//! was last rasterized at.
//!
//! ## Why the source image is kept
//!
//! Everything the dialog shows is *derived* - re-rasterizing the source is how
//! a threshold change takes effect, and the recolour is a 256-entry map applied
//! on top rather than a paint over the pixels. So every control is undoable by
//! moving it back, and Save re-derives once from the source rather than trusting
//! whatever the preview accumulated.
//!
//! ## The two planes stay two planes
//!
//! [`map_core::rasterize`] splits the image by alpha into the object's ink and
//! the ground it shades, and they travel separately all the way to the asset -
//! the shadow is never burnt into the body. That is what will let a later
//! feature decide whether two objects' shadows stack or merge.

use wgpu_ui::widget::{DrawCtx, EventCtx, LayoutCtx};
use wgpu_ui::{
	DrawList, Event, Insets, Modifiers, PointerButton, Rect, Rgba, Size, TexRect, TextureId, Vec2, Widget, WidgetId,
	WidgetState,
};

use crate::uikit_theme::rgba;
use map_core::Sprite;

/// The preview viewport (logical px) - square, like the Tile Painter's well, so
/// the two dialogs' left columns line up.
pub const WELL: f32 = 384.0;

/// One sub-palette swatch, in logical px - the palette grid's cell, so the two
/// read as the same kind of thing.
pub const CHIP: f32 = 18.0;

/// Swatches to a run: the palette grid's 16, so the strip lines up under it.
const GRID_COLS: usize = 16;

/// A valid character for a scenery id (the tile rule: ascii letters, digits,
/// `_`, plus `-`, which every shipped cut-out's id already uses).
pub fn is_id_char(c: char) -> bool {
	c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Turn a file stem into a usable id: lowercased, every run of anything else
/// collapsed to a single `-`, trimmed. Empty when nothing survived.
pub fn id_from_stem(stem: &str) -> String {
	let mut out = String::new();
	for c in stem.chars() {
		if is_id_char(c) && c != '-' {
			out.push(c.to_ascii_lowercase());
		} else if !out.ends_with('-') {
			out.push('-');
		}
	}
	out.trim_matches('-').to_string()
}

/// Turn a file stem into a display name: separators become spaces and each word
/// is capitalized, matching how the bake names the shipped pieces ("Mountain 3").
pub fn name_from_stem(stem: &str) -> String {
	let mut out = String::new();
	for word in stem.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()) {
		if !out.is_empty() {
			out.push(' ');
		}
		let mut chars = word.chars();
		if let Some(first) = chars.next() {
			out.extend(first.to_uppercase());
			out.push_str(&chars.as_str().to_ascii_lowercase());
		}
	}
	out
}

/// Why the authoring dialog was opened - it shapes the title, what Save writes,
/// and which controls apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
	/// A piece authored from an image, filed under a chosen pack.
	New,
	/// A copy of an existing piece under a new id - the one way to base your own
	/// work on a **shipped** cut-out, which is otherwise read-only.
	Clone,
	/// An existing piece, rewritten in place. Its id and its pack are fixed: the
	/// id is what a placement stores, so moving it orphans every object already
	/// on a map. Shipped pieces need `--dev`.
	Edit,
}

impl Mode {
	pub fn title(self) -> &'static str {
		match self {
			Mode::New => "New Scenery",
			Mode::Clone => "Clone Scenery",
			Mode::Edit => "Edit Scenery",
		}
	}

	/// Whether this run rewrites a piece that already exists (so its identity is
	/// not the user's to change).
	pub fn in_place(self) -> bool {
		self == Mode::Edit
	}
}

/// The editor-owned scenery-authoring context - `Some` while the dialog is open.
///
/// The dialog holds the *derived* piece; the editor holds the source it is
/// derived from, so a PNG chosen through the native file dialog (a command
/// path, outside any frame) can be written here and picked up on the next sync.
pub struct SceneryPaintRun {
	pub mode: Mode,
	/// Destination packs: the map's own tilesets first, then everything else
	/// installed - the dialog's dropdown. Filing under a pack this map does not
	/// use is allowed (and reported), because which map is open is not the same
	/// question as which pack the art belongs to.
	pub packs: Vec<String>,
	/// The ground tone each of those packs paints with, for the preview's
	/// Ground backdrop: a cut-out is also judged by how its shadow sits on the
	/// ground it will land on, not only against the checkerboard.
	pub grounds: Vec<[u8; 3]>,
	/// Which of `packs` the dialog prefills - the map's first for a New piece,
	/// the source's own for a Clone or an Edit.
	pub pack_sel: usize,
	/// The source image, RGBA, tightly packed. Empty when the art comes from
	/// `piece` instead (a Clone or an Edit that has not replaced it).
	pub src: Vec<u8>,
	pub src_w: u32,
	pub src_h: u32,
	/// The piece being cloned or edited - the art when there is no image. An
	/// imported PNG wins over it, so "replace the art" is the same code path as
	/// authoring from scratch.
	pub piece: Option<map_core::SceneryPiece>,
	/// The piece a Clone/Edit came from, as `(pack, id, it is the user's)` - what
	/// an Edit writes back over, and which root it lives in.
	pub from: Option<(String, String, bool)>,
	/// The name and id fields' initial text. Derived from a chosen file's stem
	/// for New, carried from the source piece for Clone/Edit.
	pub name_text: String,
	pub id_text: String,
	/// Bumped whenever the EDITOR writes `src`; the dialog re-derives when the
	/// revision moves (the Tile Painter's `canvas_rev` contract).
	pub rev: u64,
	/// A **painted height map** the user imported, one grey byte per pixel over
	/// `hgt_w` x `hgt_h`. Empty when none has come in this session - which is not
	/// the same as the piece having no relief, since a Clone or an Edit starts
	/// from one that may already carry a drawn one.
	pub hgt_src: Vec<u8>,
	pub hgt_w: u32,
	pub hgt_h: u32,
	/// `rev`'s twin for the height map: the editor writes the source outside a
	/// frame, and the dialog fits it to the sprite when the revision moves.
	pub hgt_rev: u64,
	/// The height map on its way **out**, as a picture: the grey bytes the
	/// dialog handed over when Save PNG was pressed, waiting for the path the
	/// native picker is about to return. The traffic runs this way round because
	/// the dialog owns the derived relief and the editor owns file IO.
	pub hgt_out: Vec<u8>,
	pub hgt_out_w: u32,
	pub hgt_out_h: u32,
}

impl SceneryPaintRun {
	/// The pack a script-path commit targets: the dialog's dropdown default.
	pub fn target_pack(&self) -> &str {
		self.packs.get(self.pack_sel).or_else(|| self.packs.first()).map(String::as_str).unwrap_or("")
	}

	/// Whether the art currently comes from an imported image - the one case
	/// where the alpha thresholds mean anything.
	pub fn uses_image(&self) -> bool {
		!self.src.is_empty()
	}
}

// ----- pure recolour semantics -------------------------------------------------

/// Perceptual luminance of a palette colour, as the sub-palette orders by.
/// Integer weights (Rec. 601 x1000) so the order is exact and stable.
pub fn luma(rgb: [u8; 3]) -> u32 {
	299 * rgb[0] as u32 + 587 * rgb[1] as u32 + 114 * rgb[2] as u32
}

/// The distinct palette indices a sprite's **body** is painted with, darkest
/// first.
///
/// Ordered by luminance rather than by index because that is the order a ramp
/// remap has to preserve: the strip reads as the object's own shading ramp, and
/// dropping it onto another ramp keeps light where light was.
pub fn sub_palette(sprite: &Sprite, palette: &[u8]) -> Vec<u8> {
	let mut seen = [false; 256];
	for &i in &sprite.body {
		seen[i as usize] = true;
	}
	let mut out: Vec<u8> = (1..=255u8).filter(|&i| seen[i as usize]).collect();
	out.sort_by_key(|&i| (luma(map_core::slot_rgb(palette, i)), i));
	out
}

/// How a palette pick lands on the selected sub-palette entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemapMode {
	/// The selection keeps its shading: entry `k` of the selection (darkest
	/// first) becomes `target + k`. One gesture re-tints a whole object.
	Ramp,
	/// Every selected entry becomes `target`. Flattens the shading - what you
	/// want for fixing one stray colour, not for a re-tint.
	Flat,
}

/// The identity recolour - every index stands for itself.
pub fn identity_remap() -> [u8; 256] {
	std::array::from_fn(|i| i as u8)
}

/// Point the selected entries of `base` at `target` under `mode`, writing into
/// `remap`.
///
/// A ramp walks *up* from `target` in `base` order, which is darkest first, and
/// stops at 255 rather than wrapping - running off the end of the palette
/// should flatten the last few, not send them back to black.
pub fn apply_remap(base: &[u8], sel: &[bool], target: u8, mode: RemapMode, remap: &mut [u8; 256]) {
	let mut step = 0u32;
	for (k, &index) in base.iter().enumerate() {
		if !sel.get(k).copied().unwrap_or(false) {
			continue;
		}
		remap[index as usize] = match mode {
			RemapMode::Flat => target,
			RemapMode::Ramp => (target as u32 + step).min(255) as u8,
		};
		step += 1;
	}
}

/// Apply a recolour to a sprite's body plane. The shade plane is untouched -
/// a shadow is an alpha, not an ink, and recolouring the object must not move
/// the shadow it casts.
pub fn recolor(sprite: &Sprite, remap: &[u8; 256]) -> Sprite {
	Sprite { body: sprite.body.iter().map(|&i| if i == 0 { 0 } else { remap[i as usize] }).collect(), ..sprite.clone() }
}

/// Which selection a click produces, given what was selected and how the click
/// was modified. Pure so the gesture is testable without a widget.
///
/// * plain - just this one
/// * Ctrl - toggle this one, keep the rest
/// * Shift - the run from the anchor to here, replacing the selection
pub fn click_selection(sel: &mut Vec<bool>, anchor: &mut usize, i: usize, mods: Modifiers) {
	if sel.len() <= i {
		sel.resize(i + 1, false);
	}
	if mods.shift {
		let (lo, hi) = if *anchor <= i { (*anchor, i) } else { (i, *anchor) };
		sel.iter_mut().enumerate().for_each(|(k, s)| *s = (lo..=hi).contains(&k));
		return;
	}
	if mods.ctrl {
		sel[i] = !sel[i];
		*anchor = i;
		return;
	}
	sel.iter_mut().for_each(|s| *s = false);
	sel[i] = true;
	*anchor = i;
}

/// Compose a sprite into RGBA for the preview: the object's ink opaque through
/// the working palette, the shadow as black at its own alpha, and everything
/// else clear - so the well's ground backdrop shows through both the holes and
/// the shadow, exactly as the map composites it.
pub fn compose_preview_rgba(sprite: &Sprite, palette_rgba: &[u8]) -> Vec<u8> {
	let mut out = Vec::with_capacity(sprite.body.len() * 4);
	for (&body, &shade) in sprite.body.iter().zip(&sprite.shade) {
		if body != 0 {
			let o = body as usize * 4;
			out.extend_from_slice(&[palette_rgba[o], palette_rgba[o + 1], palette_rgba[o + 2], 255]);
		} else {
			out.extend_from_slice(&[0, 0, 0, shade]);
		}
	}
	out
}

// ----- SpriteView --------------------------------------------------------------

/// What the preview is judged against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backdrop {
	/// The transparency checkerboard - the default, and the only backdrop that
	/// shows **how** see-through a shadow is. Against a flat tone a 50% shadow
	/// and an opaque dark ink look the same.
	Checker,
	/// The destination pack's own ground tone: what the object will actually
	/// stand on, for judging whether the shadow reads on that terrain.
	Ground,
}

/// One checkerboard square, in logical px.
const CHECK: f32 = 12.0;

/// The preview well: the composed cut-out centred over the checkerboard (or the
/// destination pack's ground tone), scaled to fit.
///
/// A dumb brick - it owns no piece, only the texture the dialog recomposes and
/// the backdrop it is judged against. The scale snaps to a whole number
/// whenever the sprite is small enough to magnify, so pixel art stays pixel art.
pub struct SpriteView {
	id: WidgetId,
	tex: TextureId,
	/// The composed sprite's size in texels (`(0, 0)` = nothing imported yet).
	size: (u32, u32),
	ground: Rgba,
	backdrop: Backdrop,
	rect: Rect,
}

impl SpriteView {
	pub fn new(tex: TextureId) -> Self {
		Self {
			id: wgpu_ui::interact::next_id(),
			tex,
			size: (0, 0),
			ground: Rgba::rgb(64, 64, 64),
			backdrop: Backdrop::Checker,
			rect: Rect::ZERO,
		}
	}

	pub fn id(&self) -> WidgetId {
		self.id
	}

	pub fn set_size(&mut self, w: u32, h: u32) {
		self.size = (w, h);
	}

	pub fn set_ground(&mut self, ground: Rgba) {
		self.ground = ground;
	}

	pub fn set_backdrop(&mut self, backdrop: Backdrop) {
		self.backdrop = backdrop;
	}

	/// Paint the backdrop over `inner`: the checkerboard, or the pack's ground.
	fn draw_backdrop(&self, dl: &mut DrawList, inner: Rect) {
		if self.backdrop == Backdrop::Ground {
			dl.fill_rect(inner, self.ground);
			return;
		}
		dl.fill_rect(inner, rgba(crate::theme::CHECKER_LIGHT));
		let dark = rgba(crate::theme::CHECKER_DARK);
		let (cols, rows) = ((inner.w / CHECK).ceil() as i32, (inner.h / CHECK).ceil() as i32);
		for row in 0..rows {
			// Every other square, offset by a row - the squares are clipped to the
			// well's edge rather than overhanging it.
			for col in (row % 2..cols).step_by(2) {
				let (x, y) = (inner.x + col as f32 * CHECK, inner.y + row as f32 * CHECK);
				let (w, h) = ((inner.right() - x).min(CHECK), (inner.bottom() - y).min(CHECK));
				if w > 0.0 && h > 0.0 {
					dl.fill_rect(Rect::new(x, y, w, h), dark);
				}
			}
		}
	}

	/// Where the art lands: centred, scaled to fit, whole-numbered when it is
	/// being magnified.
	fn art_rect(&self) -> Option<Rect> {
		let (w, h) = (self.size.0 as f32, self.size.1 as f32);
		if w <= 0.0 || h <= 0.0 {
			return None;
		}
		let inner = self.rect.inset(Insets::all(2.0));
		let fit = (inner.w / w).min(inner.h / h);
		let scale = if fit >= 1.0 { fit.floor() } else { fit };
		let (aw, ah) = (w * scale, h * scale);
		Some(Rect::new(inner.x + (inner.w - aw) / 2.0, inner.y + (inner.h - ah) / 2.0, aw, ah))
	}
}

impl Widget for SpriteView {
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
		// The backdrop fills the whole well, not just the art's footprint: the
		// shadow has to be read where it falls, not only under the object.
		self.draw_backdrop(dl, self.rect.inset(Insets::all(2.0)));
		if let Some(art) = self.art_rect() {
			dl.image(self.tex, art, TexRect::FULL, Rgba::WHITE);
			dl.stroke_rect(art, 1.0, ctx.theme.ink_dim());
		}
	}

	fn rect(&self) -> Rect {
		self.rect
	}

	fn id(&self) -> WidgetId {
		self.id
	}
}

// ----- SubPalette ---------------------------------------------------------------

/// The colours the imported object is actually painted with, as a wrapping
/// strip of swatches - the *sub*-palette, the only part of 256 colours the user
/// has any reason to retarget.
///
/// A **content widget** over a domain surface: it owns its cell geometry and
/// its hit oracle ("which used colour is under this point") and nothing else.
/// It draws no button faces and hosts no children; the mode radios, the palette
/// grid and the Select-All button are its siblings in the tree.
pub struct SubPalette {
	id: WidgetId,
	/// One swatch per used colour, in the sub-palette's order (darkest first),
	/// already showing where it currently points.
	cells: Vec<Rgba>,
	sel: Vec<bool>,
	hover: Option<usize>,
	/// Picks made this frame, drained by the dialog with their modifiers.
	picks: Vec<(usize, Modifiers)>,
	cols: usize,
	rect: Rect,
}

impl Default for SubPalette {
	fn default() -> Self {
		Self::new()
	}
}

impl SubPalette {
	pub fn new() -> Self {
		Self {
			id: wgpu_ui::interact::next_id(),
			cells: Vec::new(),
			sel: Vec::new(),
			hover: None,
			picks: Vec::new(),
			cols: 1,
			rect: Rect::ZERO,
		}
	}

	pub fn id(&self) -> WidgetId {
		self.id
	}

	/// Push one frame's swatches and selection.
	pub fn set_cells(&mut self, cells: Vec<Rgba>, sel: Vec<bool>) {
		self.cells = cells;
		self.sel = sel;
	}

	/// The entry under the cursor, for the palette grid's hover ring.
	pub fn hover(&self) -> Option<usize> {
		self.hover
	}

	/// Drains this frame's picks, in order.
	pub fn take_picks(&mut self) -> Vec<(usize, Modifiers)> {
		std::mem::take(&mut self.picks)
	}

	fn rows(&self) -> usize {
		self.cells.len().div_ceil(self.cols.max(1))
	}

	fn cell_rect(&self, i: usize) -> Rect {
		let cols = self.cols.max(1);
		let (col, row) = ((i % cols) as f32, (i / cols) as f32);
		Rect::new(self.rect.x + col * CHIP, self.rect.y + row * CHIP, CHIP, CHIP)
	}

	fn cell_at(&self, pos: Vec2) -> Option<usize> {
		if !self.rect.contains(pos) {
			return None;
		}
		let col = ((pos.x - self.rect.x) / CHIP).floor() as i32;
		let row = ((pos.y - self.rect.y) / CHIP).floor() as i32;
		if col < 0 || row < 0 || col >= self.cols.max(1) as i32 {
			return None;
		}
		let i = row as usize * self.cols.max(1) + col as usize;
		(i < self.cells.len()).then_some(i)
	}
}

impl Widget for SubPalette {
	fn measure(&mut self, avail: Size, _ctx: &mut LayoutCtx) -> Size {
		// Sixteen to a run, the palette grid's width - so the strip lines up
		// under it, and asks for a **fixed** width rather than all of it. (A
		// widget that measures to `avail.w` inside an auto-sizing window claims
		// the unbounded width it is offered and drags the whole dialog with it.)
		self.cols = ((avail.w / CHIP).floor() as usize).clamp(1, GRID_COLS);
		Size::new(self.cols as f32 * CHIP, (self.rows().max(1) as f32) * CHIP)
	}

	fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
		self.rect = rect;
		// Only ever narrower than measured: a wider arrangement must not re-flow
		// into fewer rows than the height that was reserved for it.
		self.cols = self.cols.min(((rect.w / CHIP).floor() as usize).max(1));
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		if !ctx.is_base() {
			return;
		}
		if self.cells.is_empty() {
			ctx.theme.text_fit(
				dl,
				ctx.fonts,
				self.rect,
				0.0,
				"import an image to see its colors",
				wgpu_ui::TextRole::Small,
				wgpu_ui::Emboss::Engraved,
				ctx.theme.ink_dim(),
			);
			return;
		}
		let hovered = ctx.is_hovered(self.id).then_some(self.hover).flatten();
		for (i, &color) in self.cells.iter().enumerate() {
			let r = self.cell_rect(i);
			dl.fill_rect(r.inset(Insets::all(1.0)), color);
			if self.sel.get(i).copied().unwrap_or(false) {
				dl.stroke_rect(r, 1.0, ctx.theme.accent());
			} else if hovered == Some(i) {
				dl.stroke_rect(r, 1.0, ctx.theme.ink());
			}
		}
	}

	fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
		match ev {
			Event::PointerMoved { pos } => {
				self.hover = ctx.is_hovered(self.id).then(|| self.cell_at(*pos)).flatten();
				false
			}
			Event::PointerLeft | Event::Focus(false) => {
				self.hover = None;
				false
			}
			// Commit on press, like the palette grid next to it: picking a colour
			// to retarget and then clicking its new value is one gesture.
			Event::PointerButton { button: PointerButton::Primary, pressed: true, pos, mods }
				if ctx.is_target(self.id) =>
			{
				if let Some(i) = self.cell_at(*pos) {
					self.picks.push((i, *mods));
					ctx.fire(self.id, None);
				}
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

	/// Only the swatches claim the pointer - the tail of the last run is inert,
	/// so a click past the colours does not eat the dialog's dismissal.
	fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
		self.cell_at(pos).map(|_| self.id)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn palette() -> Vec<u8> {
		// A grey ramp, so luminance order is index order, plus two off-ramp inks.
		let mut p = vec![0u8; 768];
		for i in 0..256usize {
			p[i * 3..i * 3 + 3].copy_from_slice(&[i as u8, i as u8, i as u8]);
		}
		p
	}

	fn sprite_of(body: &[u8]) -> Sprite {
		Sprite {
			width: body.len() as u16,
			height: 1,
			origin_x: 0,
			origin_y: 0,
			body: body.to_vec(),
			shade: vec![0; body.len()],
		}
	}

	#[test]
	fn the_sub_palette_is_the_used_inks_darkest_first() {
		let s = sprite_of(&[40, 10, 40, 0, 200, 10]);
		assert_eq!(sub_palette(&s, &palette()), vec![10, 40, 200], "each used ink once, ascending luminance");
		assert_eq!(sub_palette(&sprite_of(&[0, 0]), &palette()), Vec::<u8>::new(), "nothing painted, nothing listed");
	}

	/// A ramp remap keeps the object's shading: the darkest used ink lands on
	/// the target and the rest walk up from there, in the same order.
	#[test]
	fn a_ramp_remap_preserves_the_shading_order() {
		let base = vec![10u8, 40, 200];
		let mut remap = identity_remap();
		apply_remap(&base, &[true, true, true], 70, RemapMode::Ramp, &mut remap);
		assert_eq!([remap[10], remap[40], remap[200]], [70, 71, 72]);
		// Everything else is untouched.
		assert_eq!(remap[11], 11);

		// Only the selected part moves, and it walks from the target regardless
		// of where in the strip the selection started.
		let mut remap = identity_remap();
		apply_remap(&base, &[false, true, true], 90, RemapMode::Ramp, &mut remap);
		assert_eq!([remap[10], remap[40], remap[200]], [10, 90, 91]);

		// Running off the end flattens rather than wrapping back to black.
		let mut remap = identity_remap();
		apply_remap(&base, &[true, true, true], 254, RemapMode::Ramp, &mut remap);
		assert_eq!([remap[10], remap[40], remap[200]], [254, 255, 255]);
	}

	#[test]
	fn a_flat_remap_sends_the_whole_selection_to_one_ink() {
		let base = vec![10u8, 40, 200];
		let mut remap = identity_remap();
		apply_remap(&base, &[true, false, true], 33, RemapMode::Flat, &mut remap);
		assert_eq!([remap[10], remap[40], remap[200]], [33, 40, 33]);
	}

	/// Recolouring moves the object's ink and nothing else - a shadow is an
	/// alpha, and re-tinting a tree must not move the shadow it casts.
	#[test]
	fn recolor_touches_the_body_and_leaves_the_shadow_alone() {
		let mut s = sprite_of(&[10, 0, 40]);
		s.shade = vec![0, 128, 0];
		let mut remap = identity_remap();
		remap[10] = 99;
		remap[40] = 98;
		let out = recolor(&s, &remap);
		assert_eq!(out.body, vec![99, 0, 98]);
		assert_eq!(out.shade, vec![0, 128, 0], "the shade plane is untouched");
		assert_eq!((out.width, out.height, out.origin_x, out.origin_y), (3, 1, 0, 0));
		// Index 0 means "nothing here" and can never be remapped into ink.
		let mut remap = identity_remap();
		remap[0] = 5;
		assert_eq!(recolor(&s, &remap).body, vec![10, 0, 40]);
	}

	#[test]
	fn clicks_select_one_toggle_or_a_run() {
		let (mut sel, mut anchor) = (vec![false; 5], 0usize);
		let plain = Modifiers::NONE;
		let ctrl = Modifiers { ctrl: true, ..Modifiers::NONE };
		let shift = Modifiers { shift: true, ..Modifiers::NONE };

		click_selection(&mut sel, &mut anchor, 1, plain);
		assert_eq!(sel, [false, true, false, false, false]);
		assert_eq!(anchor, 1);

		click_selection(&mut sel, &mut anchor, 3, ctrl);
		assert_eq!(sel, [false, true, false, true, false], "ctrl adds without clearing");
		click_selection(&mut sel, &mut anchor, 3, ctrl);
		assert_eq!(sel, [false, true, false, false, false], "and toggles back off");

		// Shift takes the run from the anchor, in either direction, replacing.
		anchor = 3;
		click_selection(&mut sel, &mut anchor, 1, shift);
		assert_eq!(sel, [false, true, true, true, false]);
		click_selection(&mut sel, &mut anchor, 4, shift);
		assert_eq!(sel, [false, false, false, true, true]);

		// A plain click collapses back to one.
		click_selection(&mut sel, &mut anchor, 0, plain);
		assert_eq!(sel, [true, false, false, false, false]);
	}

	#[test]
	fn a_file_stem_seeds_a_usable_id_and_name() {
		assert_eq!(id_from_stem("Oak Stand 3"), "oak-stand-3");
		assert_eq!(name_from_stem("Oak Stand 3"), "Oak Stand 3");
		assert_eq!(id_from_stem("my_tree--v2"), "my_tree-v2");
		assert_eq!(name_from_stem("my_tree--v2"), "My Tree V2");
		assert_eq!(id_from_stem("  ??  "), "", "nothing usable yields nothing, not a row of dashes");
		assert_eq!(name_from_stem("!!!"), "");
	}

	/// The preview is what the map will composite: ink opaque, shadow black at
	/// its own alpha, holes clear.
	#[test]
	fn the_preview_composes_ink_shadow_and_holes() {
		let mut s = sprite_of(&[7, 0, 0]);
		s.shade = vec![0, 200, 0];
		let rgba: Vec<u8> = (0..256).flat_map(|i| [i as u8, 0, 0, 255]).collect();
		assert_eq!(compose_preview_rgba(&s, &rgba), vec![7, 0, 0, 255, 0, 0, 0, 200, 0, 0, 0, 0]);
	}
}
