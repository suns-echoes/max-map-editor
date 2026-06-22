//! Tile Painter modal: edit a 64×64 palette-indexed tile by hand.
//!
//! The canvas is a CPU-side `Vec<u8>` of palette indices, drawn as
//! palette-colored quads (no GPU atlas round-trip while painting) and edited
//! in place - left-click / drag paints the picked color, the eyedropper samples
//! a color off the canvas. A 16×16 palette grid picks the paint color (the
//! swatch under the hovered canvas pixel is ringed, so you can see which slot a
//! pixel uses). The preview zooms (100/200/400/600 %) and, with "animate
//! colors" on, cycles MAX's palette ranges live (the shell ticks the shared
//! [`crate::palette::PaletteCycler`] while the modal is open, so the canvas +
//! swatches shimmer exactly as the game would show them). A passability
//! selector (land/water/shore/blocked) rides along so a baked tile carries its
//! movement type.
//!
//! Pure UI state + the working canvas; the shell reads the painted bytes on
//! Save (see [`crate::state::EditorState::tile_paint_commit`]) since a command
//! line can't carry 4 KiB of pixels.

use crate::select::{self, Hit};
use crate::textinput::{Charset, TextInput};
use crate::theme;
use crate::ui::{self, Hot, Rect, UiQuads};

/// One tile is 64×64 palette indices.
pub const TILE: usize = 64;

/// Zoom levels: screen px per tile pixel, and the label. At 600 % a 64-px tile
/// fills the [`WELL`]-px canvas viewport exactly (64 × 6 = 384).
const ZOOMS: [(f32, &str); 4] = [(1.0, "100%"), (2.0, "200%"), (4.0, "400%"), (6.0, "600%")];

/// Passability values + labels (0 land / 1 water / 2 shore / 3 blocked).
const PASSES: [&str; 4] = ["land", "water", "shore", "blocked"];

const W: f32 = 712.0;
const TITLE_H: f32 = 22.0;
const MARGIN: f32 = 12.0;
/// The square canvas viewport (px). 384 = 64 × 6, so 600 % fills it.
const WELL: f32 = 384.0;
/// Palette swatch size; 16 × 18 = 288-px grid.
const SW: f32 = 18.0;
const PAL: f32 = SW * 16.0;
const BTN_H: f32 = 20.0;
const GAP: f32 = 10.0;
/// Current-color chip size + the dim caption height above each right-column row.
const CHIP: f32 = 24.0;
const LABEL: f32 = 14.0;

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
	fn title(self) -> &'static str {
		match self {
			Mode::New => "New Tile",
			Mode::Clone => "Clone Tile",
			Mode::Edit => "Edit Tile",
		}
	}
}

/// The deferred command buttons (everything else is press-fired).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArmedBtn {
	Save,
	Cancel,
	Copy,
	Paste,
	ExportPng,
	ImportPng,
}

/// What a press resolved to (a modal swallows everything).
#[derive(Debug, PartialEq)]
pub enum Press {
	Consumed,
	Cancel,
	/// Commit the painted tile - the shell reads the canvas + metadata.
	Save,
	/// Copy the canvas (raw indices) to the shell's tile clipboard.
	Copy,
	/// Paste the shell's tile clipboard over the canvas.
	Paste,
	/// Export the canvas to a PNG (the shell opens a save dialog).
	ExportPng,
	/// Import a PNG over the canvas (the shell opens an open dialog).
	ImportPng,
}

pub struct TilePainter {
	pub mode: Mode,
	/// The source/target tile id (the id being edited, or the one cloned from).
	pub tile_id: String,
	/// The pack the source tile belongs to (fixed for Edit/Clone).
	pub pack_name: String,
	/// The family's transparency mask color, if any - the pixel value drawn
	/// see-through, matching the map. `None` = fully opaque (new tiles and
	/// non-shore families). The painter doesn't change it.
	mask: Option<u8>,
	/// 64×64 palette indices, row-major.
	canvas: Vec<u8>,
	/// The picked paint color (palette index).
	pub color: u8,
	/// Index into [`ZOOMS`].
	zoom: usize,
	/// Passability 0..=3.
	pub pass: u8,
	/// Eyedropper armed: a canvas click samples its color instead of painting.
	pub eyedrop: bool,
	/// Replace-color armed: a canvas click recolors every pixel of the clicked
	/// color to the current paint color.
	pub replace: bool,
	/// Animate palette cycling in the preview (drives the shared cycler).
	pub animate: bool,
	/// The editable tile id (validated + applied on Save). For Edit it starts at
	/// the current id (a rename); for Clone a suggested fresh id; for New empty.
	id_input: TextInput,
	id_focus: bool,
	/// True between a press inside the id field and the release (mouse-select);
	/// checked before the canvas-paint drag so a field drag selects, not paints.
	id_dragging: bool,
	/// Whether the shell holds copied tile data (greys out Paste otherwise).
	has_clipboard: bool,
	/// Target packs to choose from in [`Mode::New`] (pack names).
	packs: Vec<String>,
	pack_sel: usize,
	pack_open: bool,
	zoom_open: bool,
	/// A canvas paint stroke is in progress (set on press, cleared on release).
	painting: bool,
	armed: Option<ArmedBtn>,
	pub(crate) drag_offset: (f32, f32),
}

impl TilePainter {
	/// Edit an existing tile in place. `mask` is the family's transparency
	/// color; the id field starts at the current id (a rename on Save).
	#[allow(clippy::too_many_arguments)]
	pub fn edit(
		tile_id: String,
		pack_name: String,
		mask: Option<u8>,
		pixels: Vec<u8>,
		pass: u8,
		animate: bool,
		has_clipboard: bool,
	) -> Self {
		let id = tile_id.clone();
		Self::with(Mode::Edit, tile_id, pack_name, mask, pixels, pass, Vec::new(), animate, id, has_clipboard)
	}

	/// Clone the selected tile into a new tile of the same pack/family;
	/// `suggested_id` pre-fills the (editable) id field.
	#[allow(clippy::too_many_arguments)]
	pub fn clone_from(
		tile_id: String,
		pack_name: String,
		mask: Option<u8>,
		pixels: Vec<u8>,
		pass: u8,
		animate: bool,
		suggested_id: String,
		has_clipboard: bool,
	) -> Self {
		Self::with(
			Mode::Clone,
			tile_id,
			pack_name,
			mask,
			pixels,
			pass,
			Vec::new(),
			animate,
			suggested_id,
			has_clipboard,
		)
	}

	/// A blank new tile; `packs` are the pack names it may be created in. New
	/// tiles get no mask (fully opaque, as the map renders them).
	pub fn new_tile(packs: Vec<String>, animate: bool, has_clipboard: bool) -> Self {
		let blank = vec![0u8; TILE * TILE];
		Self::with(
			Mode::New,
			String::new(),
			String::new(),
			None,
			blank,
			0,
			packs,
			animate,
			String::new(),
			has_clipboard,
		)
	}

	#[allow(clippy::too_many_arguments)]
	fn with(
		mode: Mode,
		tile_id: String,
		pack_name: String,
		mask: Option<u8>,
		canvas: Vec<u8>,
		pass: u8,
		packs: Vec<String>,
		animate: bool,
		id_text: String,
		has_clipboard: bool,
	) -> Self {
		debug_assert_eq!(canvas.len(), TILE * TILE, "canvas must be 64×64");
		Self {
			mode,
			tile_id,
			pack_name,
			mask,
			canvas,
			color: 1,
			zoom: ZOOMS.len() - 1, // start at 600 % (fills the well)
			pass: pass.min(3),
			eyedrop: false,
			replace: false,
			animate,
			id_input: TextInput::new(&id_text, 24).charset(Charset::Identifier),
			id_focus: false,
			id_dragging: false,
			has_clipboard,
			packs,
			pack_sel: 0,
			pack_open: false,
			zoom_open: false,
			painting: false,
			armed: None,
			drag_offset: (0.0, 0.0),
		}
	}

	// ----- shell accessors -----------------------------------------------------

	/// The painted pixels (64×64 palette indices).
	pub fn pixels(&self) -> &[u8] {
		&self.canvas
	}

	/// The family's transparency mask color, if any (for PNG export/import).
	pub fn mask(&self) -> Option<u8> {
		self.mask
	}

	/// The pack the tile will be committed to (chosen pack in New mode).
	pub fn target_pack(&self) -> &str {
		match self.mode {
			Mode::New => self.packs.get(self.pack_sel).map(String::as_str).unwrap_or(""),
			_ => &self.pack_name,
		}
	}

	// ----- geometry ------------------------------------------------------------

	pub fn dialog_rect(&self, w: f32, h: f32) -> Rect {
		// The dialog grows to hold the taller of its two columns - the right
		// (palette + color + 3 captioned rows) or the left (canvas + 3 control
		// rows) - plus the bottom button row.
		let right_h = PAL + GAP + CHIP + 3.0 * (GAP + LABEL + BTN_H);
		let left_h = WELL + GAP + 3.0 * BTN_H + 2.0 * 8.0;
		let dh = TITLE_H + MARGIN + right_h.max(left_h) + GAP + BTN_H + MARGIN;
		Rect::centered(w, h, W, dh).translate(self.drag_offset.0, self.drag_offset.1)
	}

	fn canvas_well(&self, d: Rect) -> Rect {
		Rect::new(d.x + MARGIN, d.y + TITLE_H + MARGIN, WELL, WELL)
	}

	fn pixel_px(&self) -> f32 {
		ZOOMS[self.zoom].0
	}

	/// The painted area's top-left, centered in the well.
	fn canvas_origin(&self, d: Rect) -> (f32, f32) {
		let well = self.canvas_well(d);
		let size = TILE as f32 * self.pixel_px();
		(well.x + (WELL - size) / 2.0, well.y + (WELL - size) / 2.0)
	}

	fn pixel_at(&self, d: Rect, x: f32, y: f32) -> Option<(u16, u16)> {
		let (ox, oy) = self.canvas_origin(d);
		let p = self.pixel_px();
		let (px, py) = (((x - ox) / p).floor(), ((y - oy) / p).floor());
		if px < 0.0 || py < 0.0 || px >= TILE as f32 || py >= TILE as f32 {
			return None;
		}
		Some((px as u16, py as u16))
	}

	fn pixel_rect(&self, d: Rect, px: u16, py: u16) -> Rect {
		let (ox, oy) = self.canvas_origin(d);
		let p = self.pixel_px();
		Rect::new(ox + px as f32 * p, oy + py as f32 * p, p, p)
	}

	fn palette_well(&self, d: Rect) -> Rect {
		Rect::new(d.x + MARGIN + WELL + 16.0, d.y + TITLE_H + MARGIN, PAL, PAL)
	}

	fn swatch_rect(&self, d: Rect, i: u8) -> Rect {
		let pw = self.palette_well(d);
		let (col, row) = ((i % 16) as f32, (i / 16) as f32);
		Rect::new(pw.x + col * SW, pw.y + row * SW, SW, SW)
	}

	fn swatch_at(&self, d: Rect, x: f32, y: f32) -> Option<u8> {
		let pw = self.palette_well(d);
		if !pw.contains(x, y) {
			return None;
		}
		let col = ((x - pw.x) / SW).floor() as i32;
		let row = ((y - pw.y) / SW).floor() as i32;
		if !(0..16).contains(&col) || !(0..16).contains(&row) {
			return None;
		}
		Some((row * 16 + col) as u8)
	}

	/// Y of the first controls row beneath the canvas (zoom/eyedrop/replace).
	fn controls_y(&self, d: Rect) -> f32 {
		self.canvas_well(d).y + WELL + GAP
	}

	/// Y of the second controls row (copy/paste/animate).
	fn controls_y2(&self, d: Rect) -> f32 {
		self.controls_y(d) + BTN_H + 8.0
	}

	/// Y of the third controls row (export/import PNG).
	fn controls_y3(&self, d: Rect) -> f32 {
		self.controls_y2(d) + BTN_H + 8.0
	}

	fn export_btn(&self, d: Rect) -> Rect {
		Rect::new(d.x + MARGIN, self.controls_y3(d), 130.0, BTN_H)
	}

	fn import_btn(&self, d: Rect) -> Rect {
		Rect::new(d.x + MARGIN + 138.0, self.controls_y3(d), 130.0, BTN_H)
	}

	fn zoom_box(&self, d: Rect) -> Rect {
		Rect::new(d.x + MARGIN, self.controls_y(d), 84.0, BTN_H)
	}

	fn eyedrop_btn(&self, d: Rect) -> Rect {
		Rect::new(d.x + MARGIN + 92.0, self.controls_y(d), 100.0, BTN_H)
	}

	fn replace_btn(&self, d: Rect) -> Rect {
		Rect::new(d.x + MARGIN + 200.0, self.controls_y(d), 84.0, BTN_H)
	}

	fn copy_btn(&self, d: Rect) -> Rect {
		Rect::new(d.x + MARGIN, self.controls_y2(d), 100.0, BTN_H)
	}

	fn paste_btn(&self, d: Rect) -> Rect {
		Rect::new(d.x + MARGIN + 108.0, self.controls_y2(d), 100.0, BTN_H)
	}

	fn animate_btn(&self, d: Rect) -> Rect {
		Rect::new(d.x + MARGIN + 216.0, self.controls_y2(d), 120.0, BTN_H)
	}

	/// Right column x (under the palette grid).
	fn rcx(&self, d: Rect) -> f32 {
		d.x + MARGIN + WELL + 16.0
	}

	/// The current-color swatch rect (right column, under the palette).
	fn color_chip(&self, d: Rect) -> Rect {
		Rect::new(self.rcx(d), self.palette_well(d).y + PAL + GAP, CHIP, CHIP)
	}

	fn pass_btn(&self, d: Rect, i: usize) -> Rect {
		let y = self.color_chip(d).y + CHIP + GAP + LABEL;
		Rect::new(self.rcx(d) + i as f32 * 70.0, y, 68.0, BTN_H)
	}

	/// The pack row (a target-pack dropdown in New mode, else a label).
	fn pack_box(&self, d: Rect) -> Rect {
		let y = self.pass_btn(d, 0).y + BTN_H + GAP + LABEL;
		Rect::new(self.rcx(d), y, PAL, BTN_H)
	}

	/// The editable tile-id field.
	fn id_field(&self, d: Rect) -> Rect {
		let y = self.pack_box(d).y + BTN_H + GAP + LABEL;
		Rect::new(self.rcx(d), y, PAL, BTN_H)
	}

	fn save_btn(&self, d: Rect) -> Rect {
		Rect::new(d.x + d.w - 110.0, d.y + d.h - BTN_H - 10.0, 100.0, BTN_H)
	}

	fn cancel_btn(&self, d: Rect) -> Rect {
		Rect::new(d.x + 10.0, d.y + d.h - BTN_H - 10.0, 90.0, BTN_H)
	}

	// ----- events --------------------------------------------------------------

	pub fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		// Open dropdowns take priority (their lists float over the rest).
		match select::hit(self.zoom_box(d), self.zoom_open, ZOOMS.len(), false, x, y) {
			Some(Hit::Box) => {
				self.zoom_open = !self.zoom_open;
				return Press::Consumed;
			}
			Some(Hit::Option(i)) => {
				self.zoom = i;
				self.zoom_open = false;
				return Press::Consumed;
			}
			None if self.zoom_open => {
				self.zoom_open = false;
				return Press::Consumed;
			}
			None => {}
		}
		if self.mode == Mode::New {
			match select::hit(self.pack_box(d), self.pack_open, self.packs.len(), false, x, y) {
				Some(Hit::Box) => {
					self.pack_open = !self.pack_open;
					return Press::Consumed;
				}
				Some(Hit::Option(i)) => {
					self.pack_sel = i;
					self.pack_open = false;
					return Press::Consumed;
				}
				None if self.pack_open => {
					self.pack_open = false;
					return Press::Consumed;
				}
				None => {}
			}
		}
		// The id field takes keyboard focus; any other click drops it.
		if self.id_field(d).contains(x, y) {
			self.id_focus = true;
			self.id_input.on_press(x, y, self.id_field(d));
			self.id_dragging = true;
			return Press::Consumed;
		}
		self.id_focus = false;
		self.id_dragging = false;
		// Canvas: paint, sample (eyedropper), or recolor-by-color (replace).
		if let Some((px, py)) = self.pixel_at(d, x, y) {
			let under = self.canvas[py as usize * TILE + px as usize];
			if self.eyedrop {
				self.color = under;
				self.eyedrop = false; // one-shot, like a real eyedropper
			} else if self.replace {
				for p in self.canvas.iter_mut() {
					if *p == under {
						*p = self.color;
					}
				}
			} else {
				self.painting = true;
				self.paint(px, py);
			}
			return Press::Consumed;
		}
		if let Some(i) = self.swatch_at(d, x, y) {
			self.color = i;
			return Press::Consumed;
		}
		// Eyedropper / replace are mutually exclusive modes.
		if self.eyedrop_btn(d).contains(x, y) {
			self.eyedrop = !self.eyedrop;
			self.replace = false;
			return Press::Consumed;
		}
		if self.replace_btn(d).contains(x, y) {
			self.replace = !self.replace;
			self.eyedrop = false;
			return Press::Consumed;
		}
		if self.animate_btn(d).contains(x, y) {
			self.animate = !self.animate;
			return Press::Consumed;
		}
		// Copy / Paste arm here, fire on release-inside (paste only when there's
		// something to paste).
		if self.copy_btn(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Copy);
			return Press::Consumed;
		}
		if self.has_clipboard && self.paste_btn(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Paste);
			return Press::Consumed;
		}
		if self.export_btn(d).contains(x, y) {
			self.armed = Some(ArmedBtn::ExportPng);
			return Press::Consumed;
		}
		if self.import_btn(d).contains(x, y) {
			self.armed = Some(ArmedBtn::ImportPng);
			return Press::Consumed;
		}
		for i in 0..PASSES.len() {
			if self.pass_btn(d, i).contains(x, y) {
				self.pass = i as u8;
				return Press::Consumed;
			}
		}
		if self.save_btn(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Save);
			return Press::Consumed;
		}
		if self.cancel_btn(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Cancel);
			return Press::Consumed;
		}
		Press::Consumed
	}

	/// A held paint stroke follows the cursor; the command buttons only fire on
	/// a release that's still inside them (a drag-off cancels).
	pub fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		self.painting = false;
		self.id_dragging = false;
		match self.armed.take() {
			Some(ArmedBtn::Save) if self.save_btn(d).contains(x, y) => Press::Save,
			Some(ArmedBtn::Cancel) if self.cancel_btn(d).contains(x, y) => Press::Cancel,
			Some(ArmedBtn::Copy) if self.copy_btn(d).contains(x, y) => Press::Copy,
			Some(ArmedBtn::Paste) if self.paste_btn(d).contains(x, y) => Press::Paste,
			Some(ArmedBtn::ExportPng) if self.export_btn(d).contains(x, y) => Press::ExportPng,
			Some(ArmedBtn::ImportPng) if self.import_btn(d).contains(x, y) => Press::ImportPng,
			_ => Press::Consumed,
		}
	}

	/// The id field's edit state when it's focused.
	pub fn edit_context(&self) -> Option<crate::modal::EditContext> {
		self.id_focus.then(|| self.id_input.edit_context())
	}

	/// Keyboard, routed here by the shell while the modal is open: edits the id
	/// field when it has focus (id chars + backspace), otherwise ignored.
	pub fn key(&mut self, key: &crate::modal::ModalKey) {
		if !self.id_focus {
			return;
		}
		self.id_input.on_key(key);
	}

	/// Replace the canvas with `pixels` (Paste). Length must be 64×64.
	pub fn set_pixels(&mut self, pixels: &[u8]) {
		if pixels.len() == TILE * TILE {
			self.canvas.copy_from_slice(pixels);
		}
	}

	/// The (edited) tile id the Save should use, trimmed.
	pub fn new_id(&self) -> &str {
		self.id_input.text().trim()
	}

	pub fn on_drag(&mut self, x: f32, y: f32, w: f32, h: f32) {
		let d = self.dialog_rect(w, h);
		// A drag begun in the id field extends its selection - checked before the
		// canvas-paint drag so dragging the field never paints.
		if self.id_dragging {
			self.id_input.on_drag(x, y, self.id_field(d));
			return;
		}
		if !self.painting {
			return;
		}
		if let Some((px, py)) = self.pixel_at(d, x, y) {
			self.paint(px, py);
		}
	}

	/// Wheel cycles the zoom level.
	pub fn on_wheel(&mut self, steps: f32) {
		let next = self.zoom as i32 + steps.signum() as i32;
		self.zoom = next.clamp(0, ZOOMS.len() as i32 - 1) as usize;
	}

	/// The id field's text/caret/selection, clipped to its well by the shell.
	pub fn field_contents(&self, w: f32, h: f32) -> Vec<(UiQuads, Rect)> {
		let r = self.id_field(self.dialog_rect(w, h));
		vec![(self.id_input.content_quads(r, self.id_focus, w, h), r)]
	}

	fn paint(&mut self, px: u16, py: u16) {
		self.canvas[py as usize * TILE + px as usize] = self.color;
	}

	// ----- drawing -------------------------------------------------------------

	/// Chrome behind the palette swatches + canvas pixels (drawn first).
	pub fn view(&self, w: f32, h: f32, hot: Hot) -> UiQuads {
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(ui::SteelMap::anchored(d));
		ui::modal_scrim(&mut q, w, h);
		ui::modal_frame(&mut q, d, self.mode.title(), TITLE_H, w, h);

		// The two recessed wells the art is painted into.
		q.field(self.canvas_well(d), w, h);
		q.field(self.palette_well(d), w, h);

		// Controls row 1: zoom select + eyedropper + replace toggles.
		select::draw_box(&mut q, self.zoom_box(d), ZOOMS[self.zoom].1, self.zoom_open, w, h, hot);
		q.toggle_button(self.eyedrop_btn(d), "eyedropper", self.eyedrop, true, ui::FONT_SMALL, w, h, hot);
		q.toggle_button(self.replace_btn(d), "replace", self.replace, true, ui::FONT_SMALL, w, h, hot);
		// Controls row 2: copy / paste / animate.
		q.button(self.copy_btn(d), w, h, hot);
		q.label_in("copy", self.copy_btn(d), 8.0, ui::FONT_SMALL, w, h, theme::INK);
		if self.has_clipboard {
			q.button(self.paste_btn(d), w, h, hot);
			q.label_in("paste", self.paste_btn(d), 8.0, ui::FONT_SMALL, w, h, theme::INK);
		} else {
			q.button_disabled(self.paste_btn(d), w, h);
			q.label_in("paste", self.paste_btn(d), 8.0, ui::FONT_SMALL, w, h, theme::INK_DIM);
		}
		q.toggle_button(self.animate_btn(d), "animate colors", self.animate, true, ui::FONT_SMALL, w, h, hot);
		// Controls row 3: export / import PNG.
		q.button(self.export_btn(d), w, h, hot);
		q.label_in("export png", self.export_btn(d), 8.0, ui::FONT_SMALL, w, h, theme::INK);
		q.button(self.import_btn(d), w, h, hot);
		q.label_in("import png", self.import_btn(d), 8.0, ui::FONT_SMALL, w, h, theme::INK);

		// Right column: current color, pass selector, pack, id field.
		let chip = self.color_chip(d);
		q.label(&format!("color {}", self.color), chip.x + 32.0, chip.y + 6.0, ui::FONT_SMALL, w, h, theme::INK);
		q.label("passability", self.rcx(d), self.pass_btn(d, 0).y - LABEL, ui::FONT_SMALL, w, h, theme::INK_DIM);
		for (i, name) in PASSES.iter().enumerate() {
			q.toggle_button(self.pass_btn(d, i), name, self.pass as usize == i, true, ui::FONT_SMALL, w, h, hot);
		}
		let pack_box = self.pack_box(d);
		if self.mode == Mode::New {
			q.label("pack", self.rcx(d), pack_box.y - LABEL, ui::FONT_SMALL, w, h, theme::INK_DIM);
			let label = self.packs.get(self.pack_sel).map(String::as_str).unwrap_or("(none)");
			select::draw_box(&mut q, pack_box, label, self.pack_open, w, h, hot);
		} else {
			q.label("pack", self.rcx(d), pack_box.y - LABEL, ui::FONT_SMALL, w, h, theme::INK_DIM);
			q.label(&self.pack_name, pack_box.x + 2.0, pack_box.y + 4.0, ui::FONT_SMALL, w, h, theme::INK);
		}
		// Editable tile id.
		let id = self.id_field(d);
		q.label("tile id", self.rcx(d), id.y - LABEL, ui::FONT_SMALL, w, h, theme::INK_DIM);
		q.field(id, w, h);
		if self.id_focus {
			q.border(id, w, h, theme::INK);
		}
		// Empty + unfocused shows the "(auto)" hint; the live value/caret is drawn
		// clipped by `field_contents`.
		if self.id_input.text().is_empty() && !self.id_focus {
			q.label_in("(auto)", id, 6.0, ui::FONT_SMALL, w, h, theme::INK_DIM);
		}

		// Cancel / Save.
		q.button(self.cancel_btn(d), w, h, hot);
		q.label_in("Cancel", self.cancel_btn(d), 8.0, ui::FONT_SMALL, w, h, theme::INK_DIM);
		q.button_primary(self.save_btn(d), w, h, hot);
		q.label_in("Save", self.save_btn(d), 8.0, ui::FONT_SMALL, w, h, theme::INK);
		q
	}

	/// The palette swatches + canvas pixels, colored from the live (cycled)
	/// palette `rgba` (256×RGBA). Drawn between [`Self::view`] and
	/// [`Self::overlay`].
	pub fn art(&self, rgba: &[u8], w: f32, h: f32) -> UiQuads {
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(ui::SteelMap::anchored(d));
		// Palette: every slot opaque (so index 0 is still pickable/visible).
		for i in 0..=255u8 {
			q.rect(self.swatch_rect(d, i), w, h, color_of(rgba, i, true));
		}
		// Canvas: the family's mask color is transparent (the well shows
		// through), matching the map; everything else is opaque - so a land
		// tile's index-0 pixels show, exactly as on the map.
		for py in 0..TILE as u16 {
			for px in 0..TILE as u16 {
				let idx = self.canvas[py as usize * TILE + px as usize];
				if Some(idx) == self.mask {
					continue;
				}
				q.rect(self.pixel_rect(d, px, py), w, h, color_of(rgba, idx, true));
			}
		}
		// The current paint color.
		q.rect(self.color_chip(d), w, h, color_of(rgba, self.color, true));
		q
	}

	/// Borders/rings over the art, plus the floating dropdown popups (drawn
	/// last so they sit on top of everything).
	pub fn overlay(&self, w: f32, h: f32, hot: Hot) -> UiQuads {
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(ui::SteelMap::anchored(d));
		// Frame the canvas and the current-color chip.
		q.border(self.canvas_well(d), w, h, theme::PANEL_BORDER);
		q.border(self.color_chip(d), w, h, theme::INK_DIM);
		// The selected swatch is ringed in accent green.
		q.border(self.swatch_rect(d, self.color), w, h, theme::ACCENT);
		// The swatch of the color under the hovered canvas pixel - so you can
		// see which palette slot a pixel uses (the design's hover cue).
		if let Some((cx, cy)) = hot.cursor {
			if let Some((px, py)) = self.pixel_at(d, cx, cy) {
				let idx = self.canvas[py as usize * TILE + px as usize];
				q.border(self.swatch_rect(d, idx), w, h, theme::INK);
				q.border(self.pixel_rect(d, px, py), w, h, theme::INK);
			}
		}
		// Floating option lists.
		if self.zoom_open {
			let labels: Vec<&str> = ZOOMS.iter().map(|z| z.1).collect();
			select::draw_popup(&mut q, self.zoom_box(d), &labels, Some(self.zoom), false, w, h, hot);
		}
		if self.mode == Mode::New && self.pack_open {
			select::draw_popup(&mut q, self.pack_box(d), &self.packs, Some(self.pack_sel), false, w, h, hot);
		}
		q
	}
}

/// A valid character for a tile id (ascii letters, digits, `_`).
pub(crate) fn is_id_char(c: char) -> bool {
	c.is_ascii_alphanumeric() || c == '_'
}

use crate::theme::srgb_to_linear;

/// A palette slot's RGBA color from the live `rgba` table. `opaque` forces
/// alpha 1.0 (the cycler keeps slot 0 transparent for the map; swatches show
/// it as a real color).
fn color_of(rgba: &[u8], i: u8, opaque: bool) -> [f32; 4] {
	let o = i as usize * 4;
	let a = if opaque { 1.0 } else { rgba[o + 3] as f32 / 255.0 };
	[srgb_to_linear(rgba[o]), srgb_to_linear(rgba[o + 1]), srgb_to_linear(rgba[o + 2]), a]
}

#[cfg(test)]
mod tests {
	use super::*;

	fn flat_palette() -> Vec<u8> {
		(0..256).flat_map(|i| [i as u8, i as u8, i as u8, 255]).collect()
	}

	#[test]
	fn paint_writes_the_picked_color() {
		let mut m = TilePainter::new_tile(vec!["GREEN".into()], false, false);
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		m.color = 42;
		// Press the top-left canvas pixel.
		let r = m.pixel_rect(d, 0, 0);
		assert_eq!(m.on_press(r.x + 1.0, r.y + 1.0, w, h), Press::Consumed);
		assert_eq!(m.canvas[0], 42);
		// Drag paints subsequent pixels.
		let r1 = m.pixel_rect(d, 1, 0);
		m.on_drag(r1.x + 1.0, r1.y + 1.0, w, h);
		assert_eq!(m.canvas[1], 42);
		m.on_release(r1.x + 1.0, r1.y + 1.0, w, h);
		// After release, a move no longer paints.
		let r2 = m.pixel_rect(d, 2, 0);
		m.on_drag(r2.x + 1.0, r2.y + 1.0, w, h);
		assert_eq!(m.canvas[2], 0, "drag without a held press does nothing");
	}

	#[test]
	fn eyedropper_samples_then_disarms() {
		let mut m = TilePainter::new_tile(vec!["GREEN".into()], false, false);
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		m.canvas[0] = 7;
		// Arm the eyedropper, then click the painted pixel.
		let e = m.eyedrop_btn(d);
		m.on_press(e.x + 2.0, e.y + 2.0, w, h);
		assert!(m.eyedrop);
		let r = m.pixel_rect(d, 0, 0);
		m.on_press(r.x + 1.0, r.y + 1.0, w, h);
		assert_eq!(m.color, 7, "sampled the pixel under the cursor");
		assert!(!m.eyedrop, "eyedropper is one-shot");
	}

	#[test]
	fn swatch_pick_and_pass_and_zoom() {
		let mut m = TilePainter::new_tile(vec!["GREEN".into(), "DESERT".into()], false, false);
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		// Pick swatch 200.
		let s = m.swatch_rect(d, 200);
		m.on_press(s.x + 2.0, s.y + 2.0, w, h);
		assert_eq!(m.color, 200);
		// Pass selector.
		let p = m.pass_btn(d, 2);
		m.on_press(p.x + 2.0, p.y + 2.0, w, h);
		assert_eq!(m.pass, 2);
		// Zoom dropdown: open, pick 100 %.
		let z = m.zoom_box(d);
		m.on_press(z.x + 2.0, z.y + 2.0, w, h);
		assert!(m.zoom_open);
		let o = select::option_rect(z, 0, ZOOMS.len(), false);
		m.on_press(o.x + 2.0, o.y + 2.0, w, h);
		assert_eq!((m.zoom, m.zoom_open), (0, false));
		// Pack dropdown picks the target.
		let pb = m.pack_box(d);
		m.on_press(pb.x + 2.0, pb.y + 2.0, w, h);
		let o = select::option_rect(pb, 1, m.packs.len(), false);
		m.on_press(o.x + 2.0, o.y + 2.0, w, h);
		assert_eq!(m.target_pack(), "DESERT");
	}

	#[test]
	fn save_and_cancel_fire_on_release_inside() {
		let mut m = TilePainter::new_tile(vec!["GREEN".into()], false, false);
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		let s = m.save_btn(d);
		assert_eq!(m.on_press(s.x + 2.0, s.y + 2.0, w, h), Press::Consumed);
		assert_eq!(m.on_release(s.x + 2.0, s.y + 2.0, w, h), Press::Save);
		// Drag-off cancels the click.
		m.on_press(s.x + 2.0, s.y + 2.0, w, h);
		assert_eq!(m.on_release(2.0, 2.0, w, h), Press::Consumed);
		let c = m.cancel_btn(d);
		m.on_press(c.x + 2.0, c.y + 2.0, w, h);
		assert_eq!(m.on_release(c.x + 2.0, c.y + 2.0, w, h), Press::Cancel);
	}

	#[test]
	fn art_skips_only_the_mask_color() {
		let pal = flat_palette();
		// A masked family (mask color 0): a canvas full of 0 draws no pixels,
		// just swatches + the chip. A non-mask pixel adds a quad.
		let mut m = TilePainter::clone_from(
			"GSa000".into(),
			"GREEN".into(),
			Some(0),
			vec![0u8; TILE * TILE],
			2,
			false,
			"GSa900".into(),
			false,
		);
		let base = m.art(&pal, 1280.0, 800.0).verts.len();
		m.canvas[0] = 5;
		assert!(m.art(&pal, 1280.0, 800.0).verts.len() > base, "a non-mask pixel adds a quad");
		// No mask (a land tile / new tile): index 0 is opaque, so it IS drawn -
		// aligned with how the map renders an opaque family.
		let opaque = TilePainter::edit("GLa000".into(), "GREEN".into(), None, vec![0u8; TILE * TILE], 0, false, false);
		let solid = opaque.art(&pal, 1280.0, 800.0).verts.len();
		assert!(solid > base, "an opaque tile draws its index-0 pixels too");
	}

	#[test]
	fn replace_recolors_every_matching_pixel() {
		let mut m = TilePainter::new_tile(vec!["GREEN".into()], false, false);
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		m.canvas[5] = 3;
		m.canvas[9] = 3;
		m.color = 8;
		// Arm replace, click a pixel of color 3 → every 3 becomes 8.
		let rb = m.replace_btn(d);
		m.on_press(rb.x + 2.0, rb.y + 2.0, w, h);
		assert!(m.replace && !m.eyedrop);
		let p = m.pixel_rect(d, 5, 0);
		m.on_press(p.x + 1.0, p.y + 1.0, w, h);
		assert_eq!(m.canvas[5], 8);
		assert_eq!(m.canvas[9], 8, "all pixels of the clicked color are recolored");
		assert_eq!(m.canvas[0], 0, "other colors untouched");
	}

	#[test]
	fn copy_paste_round_trip() {
		let (w, h) = (1280.0, 800.0);
		// Source painter: paint a pixel, Copy.
		let mut src = TilePainter::new_tile(vec!["GREEN".into()], false, false);
		src.canvas[3] = 9;
		let d = src.dialog_rect(w, h);
		let cb = src.copy_btn(d);
		src.on_press(cb.x + 2.0, cb.y + 2.0, w, h);
		assert_eq!(src.on_release(cb.x + 2.0, cb.y + 2.0, w, h), Press::Copy);
		let clip = src.pixels().to_vec();
		// Dest painter knows the clipboard is full → Paste enabled and fires.
		let mut dst = TilePainter::new_tile(vec!["GREEN".into()], false, true);
		let pb = dst.paste_btn(d);
		dst.on_press(pb.x + 2.0, pb.y + 2.0, w, h);
		assert_eq!(dst.on_release(pb.x + 2.0, pb.y + 2.0, w, h), Press::Paste);
		dst.set_pixels(&clip);
		assert_eq!(dst.canvas[3], 9);
		// With an empty clipboard, paste is inert.
		let mut empty = TilePainter::new_tile(vec!["GREEN".into()], false, false);
		empty.on_press(pb.x + 2.0, pb.y + 2.0, w, h);
		assert_eq!(empty.on_release(pb.x + 2.0, pb.y + 2.0, w, h), Press::Consumed);
	}

	#[test]
	fn id_field_edits_when_focused() {
		use crate::modal::ModalKey;
		let mut m = TilePainter::new_tile(vec!["GREEN".into()], false, false);
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		// Typing does nothing until the field is focused.
		m.key(&ModalKey::Char('x'));
		assert_eq!(m.new_id(), "");
		let f = m.id_field(d);
		m.on_press(f.x + 2.0, f.y + 2.0, w, h);
		assert!(m.id_focus);
		for c in "GLa42!".chars() {
			m.key(&ModalKey::Char(c)); // '!' is rejected (not an id char)
		}
		assert_eq!(m.new_id(), "GLa42");
		m.key(&ModalKey::Backspace);
		assert_eq!(m.new_id(), "GLa4");
		// Clicking elsewhere (a swatch) drops focus.
		let s = m.swatch_rect(d, 1);
		m.on_press(s.x + 2.0, s.y + 2.0, w, h);
		assert!(!m.id_focus);
	}
}
