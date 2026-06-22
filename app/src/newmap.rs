//! Create New Map modal (design: features.drawio "Modals").
//! Two stages: the main dialog (preset, W×H fields, fill note, tile-set
//! summary, Abort/Create) and the tile-set picker (a checkbox per installed
//! pack with a 4-tile preview strip). Create builds a `new W H PACK..`
//! command - the same path scripts use.
//!
//! Pure state/geometry here; previews are CPU-composed RGBA strips blitted
//! by the shared [`BlitPass`].

use std::path::Path;

use map_core::{GAME_PALETTE, Rng, TilePack, apply_game_statics};

use crate::blit::BlitPass;
use crate::packlist::{self, PackEntry};
use crate::textinput::{Charset, TextInput};
use crate::theme;
use crate::ui::{self, Hot, Rect, UiQuads};

const TITLE_H: f32 = 22.0;
const ROW_H: f32 = 24.0;
const FIELD_W: f32 = 56.0;
const BTN_H: f32 = 20.0;
/// Preview strip: 4 tiles at this size.
const TILE_PREVIEW: f32 = 44.0;
const PACK_ROW_H: f32 = TILE_PREVIEW + 12.0;
/// Deterministic "random" previews - stable screenshots, still varied.
const PREVIEW_SEED: u64 = 0xC0FFEE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
	Width,
	Height,
}

pub struct NewMap {
	width: TextInput,
	height: TextInput,
	pub focus: Option<Field>,
	/// The field being mouse-drag-selected (press..release).
	drag_field: Option<Field>,
	pub packs: Vec<PackEntry>,
	/// Second stage (tile-set picker) open?
	pub picking: bool,
	/// Chosen palette-owner pack (the radio column). `None` = the first
	/// selected palette-capable pack. WATER never owns the palette.
	palette_owner: Option<String>,
	/// The size-preset dropdown's open state.
	preset_open: bool,
	/// A command button held down, waiting for release-inside
	/// - dragging off cancels.
	armed: Option<ArmedBtn>,
	/// Drag offset from centered (draggable by the titlebar).
	pub(crate) drag_offset: (f32, f32),
}

/// The deferred command buttons (preset/packs/fields stay press-fired).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArmedBtn {
	Abort,
	/// Create on the main stage / Done in the pack picker - the same rect.
	Create,
}

/// Shared map-size presets (also used by the Resize modal).
pub const SIZE_PRESETS: [(&str, u16, u16); 3] =
	[("Classic 112x112", 112, 112), ("Mega 224x224", 224, 224), ("Giga 448x448", 448, 448)];

/// What a press resolved to (everything is consumed while a modal is open).
#[derive(Debug, PartialEq)]
pub enum Press {
	Consumed,
	Abort,
	/// Validated `new …` command line, ready to execute.
	Create(String),
	/// Validation failed - show this in the console, keep the modal open.
	Invalid(String),
}

impl NewMap {
	/// Scan the assets dir for installed packs (dirs with `tiles-data.bin`).
	pub fn new(assets_root: &Path) -> Self {
		let packs = packlist::scan(assets_root);
		Self {
			width: TextInput::new("112", 4).charset(Charset::Digits),
			height: TextInput::new("112", 4).charset(Charset::Digits),
			focus: None,
			drag_field: None,
			packs,
			picking: false,
			palette_owner: None,
			preset_open: false,
			armed: None,
			drag_offset: (0.0, 0.0),
		}
	}

	/// The pack that will own the palette: the radio choice when it's a
	/// selected palette-capable pack, else the first such pack (scan order).
	/// WATER never qualifies (no palette).
	fn effective_owner(&self) -> Option<String> {
		packlist::effective_owner(&self.packs, &self.palette_owner)
	}

	/// The selected pack names: WATER (locked) first, then the palette owner -
	/// `Project::new` makes the first palette-owning pack the owner - then the
	/// rest in scan order.
	pub fn selected_packs(&self) -> Vec<String> {
		packlist::selected(&self.packs, &self.palette_owner)
	}

	fn field_mut(&mut self, f: Field) -> &mut TextInput {
		match f {
			Field::Width => &mut self.width,
			Field::Height => &mut self.height,
		}
	}

	fn field_ref(&self, f: Field) -> &TextInput {
		match f {
			Field::Width => &self.width,
			Field::Height => &self.height,
		}
	}

	/// The preset whose dimensions match the current W×H fields, if any.
	fn preset_match(&self) -> Option<usize> {
		let (w, h) = (self.width.text(), self.height.text());
		SIZE_PRESETS.iter().position(|&(_, pw, ph)| w == pw.to_string().as_str() && h == ph.to_string().as_str())
	}

	/// The dropdown's value label: the matching preset's name, else `Custom`.
	fn preset_label(&self) -> &'static str {
		self.preset_match().map(|i| SIZE_PRESETS[i].0).unwrap_or("Custom")
	}

	/// Build the `new` command, or explain what's missing.
	pub fn create_command(&self) -> Result<String, String> {
		let width: u16 = self.width.text().parse().map_err(|_| "width is not a number".to_string())?;
		let height: u16 = self.height.text().parse().map_err(|_| "height is not a number".to_string())?;
		if !(1..=1024).contains(&width) || !(1..=1024).contains(&height) {
			return Err(format!("bad size {width}x{height} (1..=1024)"));
		}
		let packs = self.selected_packs();
		if !packlist::has_palette_owner(&self.packs) {
			return Err("select at least one palette-owning tileset (e.g. GREEN)".into());
		}
		// Force (`new!`): the modal IS the confirmation surface, so it must
		// not trip the unsaved-changes guard - that bug made Create work only
		// once (consistent with Quick Load's `open!`).
		Ok(format!("new! {width} {height} {}", packs.join(" ")))
	}

	// ----- geometry ----------------------------------------------------------

	/// The main dialog rect, centered.
	pub fn dialog_rect(&self, w: f32, h: f32) -> Rect {
		let r = if self.picking {
			let ph = TITLE_H + 10.0 + self.packs.len() as f32 * PACK_ROW_H + BTN_H + 18.0;
			Rect::centered(w, h, 440.0, ph)
		} else {
			Rect::centered(w, h, 360.0, 204.0)
		};
		r.translate(self.drag_offset.0, self.drag_offset.1)
	}

	fn row_y(d: Rect, row: usize) -> f32 {
		d.y + TITLE_H + 8.0 + row as f32 * (ROW_H + 4.0)
	}

	/// Main dialog controls. Rows: 0 preset, 1 size, 2 fill, 3 tile sets. The
	/// preset is a [`select`](crate::select) dropdown.
	fn preset_rect(d: Rect) -> Rect {
		Rect::new(d.x + 110.0, Self::row_y(d, 0), 170.0, BTN_H)
	}

	fn field_rect(d: Rect, f: Field) -> Rect {
		let y = Self::row_y(d, 1);
		match f {
			Field::Width => Rect::new(d.x + 110.0, y, FIELD_W, BTN_H),
			Field::Height => Rect::new(d.x + 110.0 + FIELD_W + 18.0, y, FIELD_W, BTN_H),
		}
	}

	fn packs_btn_rect(d: Rect) -> Rect {
		Rect::new(d.x + 110.0, Self::row_y(d, 3), 220.0, BTN_H)
	}

	fn abort_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + 10.0, d.y + d.h - BTN_H - 10.0, 90.0, BTN_H)
	}

	fn create_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + d.w - 100.0, d.y + d.h - BTN_H - 10.0, 90.0, BTN_H)
	}

	/// Pack-picker rows: checkbox + name + preview strip.
	fn pack_row(d: Rect, i: usize) -> Rect {
		Rect::new(d.x + 10.0, d.y + TITLE_H + 6.0 + i as f32 * PACK_ROW_H, d.w - 20.0, PACK_ROW_H)
	}

	/// The 4-tile preview strip inside a pack row (left of the radio column).
	pub fn preview_rect(d: Rect, i: usize) -> Rect {
		let r = Self::pack_row(d, i);
		Rect::new(r.x + r.w - 4.0 * TILE_PREVIEW - 32.0, r.y + 4.0, 4.0 * TILE_PREVIEW, TILE_PREVIEW)
	}

	/// The palette-owner radio toggle on the far right of a pack row.
	fn radio_rect(d: Rect, i: usize) -> Rect {
		let r = Self::pack_row(d, i);
		Rect::new(r.x + r.w - 18.0, r.y + (r.h - 14.0) / 2.0, 14.0, 14.0)
	}

	// ----- events --------------------------------------------------------------

	pub fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		if self.picking {
			// Palette-owner radio (far-right column) - palette-capable, non-WATER
			// rows only. Owning the palette implies the pack is selected.
			for i in 0..self.packs.len() {
				if self.packs[i].has_palette && !self.packs[i].locked && Self::radio_rect(d, i).contains(x, y) {
					self.palette_owner = Some(self.packs[i].name.clone());
					self.packs[i].selected = true;
					return Press::Consumed;
				}
			}
			for i in 0..self.packs.len() {
				let r = Self::pack_row(d, i);
				if r.contains(x, y) && !self.packs[i].locked {
					self.packs[i].selected = !self.packs[i].selected;
					return Press::Consumed;
				}
			}
			// Done arms (release-inside closes the picker); click-out backs
			// out immediately.
			if self.create_rect(d).contains(x, y) {
				self.armed = Some(ArmedBtn::Create);
			} else if !d.contains(x, y) {
				self.picking = false;
			}
			return Press::Consumed;
		}

		// Size-preset dropdown: the box toggles; an option rewrites the fields.
		match crate::select::hit(Self::preset_rect(d), self.preset_open, SIZE_PRESETS.len(), false, x, y) {
			Some(crate::select::Hit::Box) => {
				self.preset_open = !self.preset_open;
				return Press::Consumed;
			}
			Some(crate::select::Hit::Option(i)) => {
				let (_, pw, ph) = SIZE_PRESETS[i];
				self.width.set_text(&pw.to_string());
				self.height.set_text(&ph.to_string());
				self.preset_open = false;
				return Press::Consumed;
			}
			None if self.preset_open => {
				// A click off an open list closes it (and is consumed).
				self.preset_open = false;
				return Press::Consumed;
			}
			None => {}
		}
		for f in [Field::Width, Field::Height] {
			let r = Self::field_rect(d, f);
			if r.contains(x, y) {
				self.focus = Some(f);
				self.drag_field = Some(f);
				self.field_mut(f).on_press(x, y, r);
				return Press::Consumed;
			}
		}
		if Self::packs_btn_rect(d).contains(x, y) {
			self.picking = true;
			return Press::Consumed;
		}
		// Abort/Create arm and fire on release-inside.
		if self.abort_rect(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Abort);
			return Press::Consumed;
		}
		if self.create_rect(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Create);
			return Press::Consumed;
		}
		self.focus = None;
		Press::Consumed // modal: everything else is swallowed
	}

	/// Fire the armed command button if the release is still on it;
	/// a release anywhere else just disarms.
	pub fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		self.drag_field = None;
		let d = self.dialog_rect(w, h);
		match self.armed.take() {
			Some(ArmedBtn::Abort) if !self.picking && self.abort_rect(d).contains(x, y) => Press::Abort,
			Some(ArmedBtn::Create) if self.create_rect(d).contains(x, y) => {
				if self.picking {
					// "Done": back to the main stage.
					self.picking = false;
					Press::Consumed
				} else {
					match self.create_command() {
						Ok(line) => Press::Create(line),
						Err(e) => Press::Invalid(e),
					}
				}
			}
			_ => Press::Consumed,
		}
	}

	/// The focused W/H field's edit state (none while the pack picker is open).
	pub fn edit_context(&self) -> Option<crate::modal::EditContext> {
		if self.picking {
			return None;
		}
		let f = self.field_ref(self.focus?);
		Some(f.edit_context())
	}

	/// Route an editing key to the focused size field.
	pub fn key(&mut self, key: &crate::modal::ModalKey) {
		let Some(f) = self.focus else { return };
		self.field_mut(f).on_key(key);
	}

	/// Tab: toggle focus between the two size fields.
	pub fn focus_next(&mut self) {
		self.focus = Some(match self.focus {
			Some(Field::Width) => Field::Height,
			_ => Field::Width,
		});
	}

	/// Mouse drag extends the active field's selection (after a press on it).
	pub fn on_drag(&mut self, x: f32, y: f32, w: f32, h: f32) {
		if let Some(f) = self.drag_field {
			let r = Self::field_rect(self.dialog_rect(w, h), f);
			self.field_mut(f).on_drag(x, y, r);
		}
	}

	// ----- drawing --------------------------------------------------------------

	pub fn view(&self, w: f32, h: f32, hot: Hot) -> UiQuads {
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(ui::SteelMap::anchored(d));
		// Dim everything beneath - it's a modal.
		ui::modal_scrim(&mut q, w, h);
		let title = if self.picking { "Select Tile Sets" } else { "Create New Map" };
		ui::modal_frame(&mut q, d, title, TITLE_H, w, h);

		if self.picking {
			for (i, p) in self.packs.iter().enumerate() {
				let r = Self::pack_row(d, i);
				if !p.locked && hot.hover(r) {
					q.rect(r, w, h, theme::HOVER);
				}
				// Checkbox.
				let cb = Rect::new(r.x + 2.0, r.y + (r.h - 14.0) / 2.0, 14.0, 14.0);
				q.field(cb, w, h);
				if p.selected {
					q.rect(
						Rect::new(cb.x + 3.0, cb.y + 3.0, 8.0, 8.0),
						w,
						h,
						if p.locked { theme::INK_DIM } else { theme::INK },
					);
				}
				let label = format!(
					"{}{}",
					p.name,
					if p.locked {
						" (always)"
					} else if p.has_palette {
						" (palette)"
					} else {
						""
					},
				);
				let ink = if p.locked { theme::INK_DIM } else { theme::INK };
				// Fit between the checkbox and the preview strip.
				let avail = Self::preview_rect(d, i).x - (cb.x + 20.0) - 6.0;
				let label = crate::text::fit_label(&label, crate::ui::FONT_SMALL, avail);
				q.label(&label, cb.x + 20.0, r.y + (r.h - 12.0) / 2.0, crate::ui::FONT_SMALL, w, h, ink);
				// The preview strip area is an inset well; tiles blit beneath.
				q.field(Self::preview_rect(d, i), w, h);
				// Palette-owner radio (palette-capable, non-WATER rows only).
				if p.has_palette && !p.locked {
					let rr = Self::radio_rect(d, i);
					q.field(rr, w, h);
					if self.effective_owner().as_deref() == Some(p.name.as_str()) {
						q.rect(Rect::new(rr.x + 3.0, rr.y + 3.0, 8.0, 8.0), w, h, theme::ACCENT);
					}
				}
			}
			// Header for the radio column ("palette owner").
			q.label_in(
				"palette",
				Rect::new(d.x + d.w - 76.0, d.y, 66.0, TITLE_H),
				0.0,
				crate::ui::FONT_SMALL,
				w,
				h,
				theme::INK_DIM,
			);
			q.button_primary(self.create_rect(d), w, h, hot);
			q.label_in("Done", self.create_rect(d), 8.0, crate::ui::FONT_SMALL, w, h, theme::INK);
			return q;
		}

		let label_x = d.x + 10.0;
		let rows: [(&str, usize); 4] = [("preset", 0), ("size", 1), ("fill", 2), ("tile sets", 3)];
		for (name, row) in rows {
			q.label(name, label_x, Self::row_y(d, row) + 4.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		}

		crate::select::draw_box(&mut q, Self::preset_rect(d), self.preset_label(), self.preset_open, w, h, hot);

		// The size fields' wells + focus borders; their text is drawn clipped by
		// the shell (see `field_contents`).
		for f in [Field::Width, Field::Height] {
			let r = Self::field_rect(d, f);
			q.field(r, w, h);
			if self.focus == Some(f) {
				q.border(r, w, h, theme::INK);
			}
		}
		let x_label = Rect::new(Self::field_rect(d, Field::Width).x + FIELD_W + 4.0, Self::row_y(d, 1), 12.0, BTN_H);
		q.label_in("x", x_label, 0.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);

		q.label("water", d.x + 110.0, Self::row_y(d, 2) + 4.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);

		let pb = Self::packs_btn_rect(d);
		q.button(pb, w, h, hot);
		// Many selected packs ellipsize inside the button.
		q.label_fit(
			&format!("{}...", self.selected_packs().join(" + ")),
			pb,
			6.0,
			crate::ui::FONT_SMALL,
			w,
			h,
			theme::INK,
		);

		q.button(self.abort_rect(d), w, h, hot);
		q.label_in("Abort", self.abort_rect(d), 8.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		q.button_primary(self.create_rect(d), w, h, hot);
		q.label_in("Create", self.create_rect(d), 8.0, crate::ui::FONT_SMALL, w, h, theme::INK);
		q
	}

	/// The open size-preset dropdown, as its own layer. The shell draws this
	/// *after* `field_contents`, so the floating list (opaque well + border)
	/// sits above the W/H field text - which is painted last and would otherwise
	/// bleed through it. `None` when closed or on the pack-picker stage.
	pub fn popup(&self, w: f32, h: f32, hot: Hot) -> Option<UiQuads> {
		if self.picking || !self.preset_open {
			return None;
		}
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(ui::SteelMap::anchored(d));
		let labels: Vec<&str> = SIZE_PRESETS.iter().map(|&(name, _, _)| name).collect();
		crate::select::draw_popup(&mut q, Self::preset_rect(d), &labels, self.preset_match(), false, w, h, hot);
		Some(q)
	}

	/// Each size field's text/caret/selection with its clip rect. Empty on the
	/// pack-picker stage (no size fields shown there).
	pub fn field_contents(&self, w: f32, h: f32) -> Vec<(UiQuads, Rect)> {
		if self.picking {
			return Vec::new();
		}
		let d = self.dialog_rect(w, h);
		[Field::Width, Field::Height]
			.into_iter()
			.map(|f| {
				let r = Self::field_rect(d, f);
				(self.field_ref(f).content_quads(r, self.focus == Some(f), w, h), r)
			})
			.collect()
	}
}

// ----- pack previews (CPU-composed, blitted) ----------------------------------

/// One preview strip per installed pack: 4 seeded-random tiles, composed
/// with the pack's palette (palette-less packs borrow the first owner's),
/// game statics applied. Built once per app run.
pub struct Previews {
	bind_group: Option<wgpu::BindGroup>,
	rows: Vec<String>,
}

impl Previews {
	pub fn new() -> Self {
		Self { bind_group: None, rows: Vec::new() }
	}

	fn build_rgba(packs: &[PackEntry], assets_root: &Path) -> (Vec<u8>, Vec<String>) {
		let loaded: Vec<Option<TilePack>> = packs.iter().map(|p| TilePack::load(assets_root, &p.name).ok()).collect();
		// The borrowed palette for palette-less packs (WATER): GREEN's if
		// installed (the canonical planet colors), else the first owner.
		let fallback: Vec<u8> = loaded
			.iter()
			.flatten()
			.find(|p| p.name == "GREEN")
			.and_then(|p| p.palette.clone())
			.or_else(|| loaded.iter().flatten().find_map(|p| p.palette.clone()))
			.unwrap_or_else(|| GAME_PALETTE.to_vec());

		let n = packs.len().max(1);
		let (tw, th) = (4 * 64usize, n * 64);
		let mut rgba = vec![0u8; tw * th * 4];
		let mut rows = Vec::with_capacity(packs.len());
		for (row, pack) in loaded.iter().enumerate() {
			let Some(pack) = pack else {
				rows.push(String::new());
				continue;
			};
			rows.push(packs[row].name.clone());
			let mut palette = pack.palette.clone().unwrap_or_else(|| fallback.clone());
			apply_game_statics(&mut palette);
			let mut rng = Rng::new(PREVIEW_SEED + row as u64);
			for slot in 0..4usize {
				let tile = rng.below(pack.tile_count() as u32) as u16;
				let pixels = pack.tile_pixels(tile);
				for y in 0..64usize {
					for x in 0..64usize {
						let p = pixels[y * 64 + x] as usize;
						let at = ((row * 64 + y) * tw + slot * 64 + x) * 4;
						if p == 0 {
							// Transparent: dim checker against the panel.
							let dim = if (x / 8 + y / 8) % 2 == 0 { 26 } else { 34 };
							rgba[at..at + 4].copy_from_slice(&[dim, dim, dim, 255]);
						} else {
							rgba[at..at + 3].copy_from_slice(&palette[p * 3..p * 3 + 3]);
							rgba[at + 3] = 255;
						}
					}
				}
			}
		}
		(rgba, rows)
	}

	/// Blit every pack's strip into its row of the open picker dialog.
	#[allow(clippy::too_many_arguments)]
	pub fn draw(
		&mut self,
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		encoder: &mut wgpu::CommandEncoder,
		target: &wgpu::TextureView,
		blit: &BlitPass,
		modal: &NewMap,
		assets_root: &Path,
		screen: (u32, u32),
		scale: f32,
	) {
		if modal.packs.is_empty() {
			return;
		}
		if self.bind_group.is_none() {
			let (rgba, rows) = Self::build_rgba(&modal.packs, assets_root);
			let size = (4 * 64u32, (modal.packs.len() as u32) * 64);
			self.bind_group = Some(blit.upload(device, queue, &rgba, size));
			self.rows = rows;
		}
		let Some(bind_group) = &self.bind_group else { return };
		// The modal lays out in **logical** px (physical / scale) - match its
		// `view()`/`field_contents()` so the previews land in their wells; `blit`
		// then projects logical → physical.
		let (w, h) = (screen.0 as f32 / scale, screen.1 as f32 / scale);
		let d = modal.dialog_rect(w, h);
		let n = modal.packs.len() as f32;
		for i in 0..modal.packs.len() {
			let dst = NewMap::preview_rect(d, i);
			let (v0, v1) = (i as f32 / n, (i + 1) as f32 / n);
			blit.draw(device, encoder, target, bind_group, dst, [0.0, v0, 1.0, v1], dst, screen, scale);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::modal::ModalKey;

	fn assets_root() -> std::path::PathBuf {
		Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources/assets/tilepacks")
	}

	fn modal() -> NewMap {
		NewMap::new(&assets_root())
	}

	#[test]
	fn scans_packs_with_defaults() {
		let m = modal();
		let names: Vec<&str> = m.packs.iter().map(|p| p.name.as_str()).collect();
		assert_eq!(names, ["CRATER", "DESERT", "GREEN", "SNOW", "SNOW_DARK", "WATER"]);
		let water = m.packs.iter().find(|p| p.name == "WATER").unwrap();
		assert!(water.locked && water.selected && !water.has_palette);
		let green = m.packs.iter().find(|p| p.name == "GREEN").unwrap();
		assert!(green.selected && green.has_palette);
		assert_eq!(m.selected_packs(), ["WATER", "GREEN"], "WATER first");
	}

	#[test]
	fn create_command_validates() {
		let mut m = modal();
		// `new!` forces past the unsaved-changes guard (the New Map fix).
		assert_eq!(m.create_command().unwrap(), "new! 112 112 WATER GREEN");
		m.width.set_text("64");
		m.height.set_text("48");
		assert_eq!(m.create_command().unwrap(), "new! 64 48 WATER GREEN");
		// The command must parse (same contract as menu actions).
		let line = m.create_command().unwrap();
		assert!(crate::command::parse_line(&line).unwrap().is_some());

		m.height.set_text("");
		assert!(m.create_command().is_err());
		m.height.set_text("2000");
		assert!(m.create_command().is_err());
		m.height.set_text("48");
		// Deselect every palette owner → must complain.
		for p in &mut m.packs {
			if !p.locked {
				p.selected = false;
			}
		}
		assert!(m.create_command().unwrap_err().contains("palette"));
	}

	#[test]
	fn palette_owner_radio_leads_the_pack_order() {
		let mut m = modal();
		for p in &mut m.packs {
			if p.name == "SNOW" {
				p.selected = true;
			}
		}
		// Default owner = GREEN (first selected palette-capable pack).
		assert_eq!(m.selected_packs(), ["WATER", "GREEN", "SNOW"]);
		// Choosing SNOW as owner leads it ahead of the other tilesets, so
		// `Project::new` (first palette pack wins) makes SNOW the owner.
		m.palette_owner = Some("SNOW".into());
		assert_eq!(m.selected_packs(), ["WATER", "SNOW", "GREEN"]);
		assert_eq!(m.create_command().unwrap(), "new! 112 112 WATER SNOW GREEN");
		// An owner choice that isn't selected falls back to the first owner.
		m.palette_owner = Some("DESERT".into());
		assert_eq!(m.selected_packs()[1], "GREEN");
	}

	#[test]
	fn typing_edits_the_focused_field() {
		let mut m = modal();
		m.focus = Some(Field::Width);
		m.width.set_text("");
		for c in "256".chars() {
			m.key(&ModalKey::Char(c));
		}
		assert_eq!(m.width.text(), "256");
		m.key(&ModalKey::Backspace);
		assert_eq!(m.width.text(), "25");
		m.key(&ModalKey::Char('x')); // non-digit ignored
		assert_eq!(m.width.text(), "25");
		m.key(&ModalKey::Char('1'));
		m.key(&ModalKey::Char('2'));
		m.key(&ModalKey::Char('9')); // 5th digit ignored (cap 4)
		assert_eq!(m.width.text(), "2512");
		m.focus_next(); // tab → height
		assert_eq!(m.focus, Some(Field::Height));
	}

	#[test]
	fn press_flow_fields_packs_create() {
		let mut m = modal();
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		// Focus the width field.
		let f = NewMap::field_rect(d, Field::Width);
		assert_eq!(m.on_press(f.x + 2.0, f.y + 2.0, w, h), Press::Consumed);
		assert_eq!(m.focus, Some(Field::Width));
		// Size preset is a dropdown: clicking the box opens the list, clicking an
		// option rewrites the fields and closes it.
		let pbox = NewMap::preset_rect(d);
		m.on_press(pbox.x + 2.0, pbox.y + 2.0, w, h);
		assert!(m.preset_open, "the box click opens the list");
		let mega = crate::select::option_rect(pbox, 1, SIZE_PRESETS.len(), false);
		m.on_press(mega.x + 2.0, mega.y + 2.0, w, h);
		assert_eq!((m.width.text(), m.height.text()), ("224", "224"));
		assert!(!m.preset_open, "picking an option closes the list");
		m.on_press(pbox.x + 2.0, pbox.y + 2.0, w, h); // reopen
		let giga = crate::select::option_rect(pbox, 2, SIZE_PRESETS.len(), false);
		m.on_press(giga.x + 2.0, giga.y + 2.0, w, h);
		assert_eq!((m.width.text(), m.height.text()), ("448", "448"));
		// Back to Classic (112) for the Create assertion below.
		m.on_press(pbox.x + 2.0, pbox.y + 2.0, w, h); // reopen
		let classic = crate::select::option_rect(pbox, 0, SIZE_PRESETS.len(), false);
		m.on_press(classic.x + 2.0, classic.y + 2.0, w, h);
		assert_eq!((m.width.text(), m.height.text()), ("112", "112"));
		// Open the picker, toggle SNOW on, WATER refuses, Done closes.
		let pb = NewMap::packs_btn_rect(d);
		m.on_press(pb.x + 2.0, pb.y + 2.0, w, h);
		assert!(m.picking);
		let dp = m.dialog_rect(w, h);
		let snow = m.packs.iter().position(|p| p.name == "SNOW").unwrap();
		let r = NewMap::pack_row(dp, snow);
		m.on_press(r.x + 2.0, r.y + 2.0, w, h);
		assert!(m.packs[snow].selected);
		let water = m.packs.iter().position(|p| p.name == "WATER").unwrap();
		let r = NewMap::pack_row(dp, water);
		m.on_press(r.x + 2.0, r.y + 2.0, w, h);
		assert!(m.packs[water].selected, "WATER stays on");
		// Done fires on release-inside (press only arms).
		let done = m.create_rect(dp);
		m.on_press(done.x + 2.0, done.y + 2.0, w, h);
		assert!(m.picking, "press alone does not close the picker");
		m.on_release(done.x + 2.0, done.y + 2.0, w, h);
		assert!(!m.picking);
		// Create returns the validated command on release-inside.
		let d = m.dialog_rect(w, h);
		let c = m.create_rect(d);
		assert_eq!(m.on_press(c.x + 2.0, c.y + 2.0, w, h), Press::Consumed);
		match m.on_release(c.x + 2.0, c.y + 2.0, w, h) {
			Press::Create(line) => assert_eq!(line, "new! 112 112 WATER GREEN SNOW"),
			other => panic!("expected Create, got {other:?}"),
		}
		// Dragging off before release cancels the click.
		m.on_press(c.x + 2.0, c.y + 2.0, w, h);
		assert_eq!(m.on_release(5.0, 5.0, w, h), Press::Consumed, "drag-off cancels");
		// Abort bubbles (on release); clicks outside the dialog are swallowed.
		let a = m.abort_rect(d);
		m.on_press(a.x + 2.0, a.y + 2.0, w, h);
		assert_eq!(m.on_release(a.x + 2.0, a.y + 2.0, w, h), Press::Abort);
		assert_eq!(m.on_press(5.0, 5.0, w, h), Press::Consumed);
	}

	#[test]
	fn preview_strip_composes_all_packs() {
		let m = modal();
		let (rgba, rows) = Previews::build_rgba(&m.packs, &assets_root());
		assert_eq!(rows.len(), 6);
		assert_eq!(rgba.len(), 4 * 64 * 6 * 64 * 4);
		// Every pack row has at least some non-checker pixels.
		for row in 0..6 {
			let any = (0..64 * 256).any(|i| {
				let at = (row * 64 * 256 + i) * 4;
				rgba[at + 3] == 255 && rgba[at] > 40
			});
			assert!(any, "row {row} ({}) looks empty", m.packs[row].name);
		}
	}
}
