//! Import WRL modal: pick the tilepacks to match a standard-tile WRL against,
//! run the match, then - if any tiles found no home - review the unmapped list
//! and decide what becomes of them.
//!
//! Two stages, keyed off whether the match has run:
//!   * **Settings** (`result` is `None`): the WRL's dimensions/tile count, a
//!     checkbox per installed pack (palette-owner radio like New Map), and
//!     Cancel / Import.
//!   * **Unmapped** (`result` is `Some`): a scrollable list of the tiles that
//!     matched nothing, a destination toggle (this project vs the user
//!     tileset), and Abort / Ignore missing / Import tiles.
//!
//! Pure UI state + geometry. The heavy [`map_core::WrlImport`] (the matched
//! project + unmapped set) is built by the shell on the Import press and parked
//! here in `result`; the shell reads it back to commit on the finish press.

use std::path::PathBuf;

use map_core::ExtrasDest;

use crate::packlist::{self, PackEntry};
use crate::theme;
use crate::ui::{self, Hot, Rect, SteelMap, UiQuads};

const W: f32 = 360.0;
const TITLE_H: f32 = 22.0;
const PAD: f32 = 14.0;
const LINE_H: f32 = 16.0;
const PACK_ROW: f32 = 24.0;
const BTN_H: f32 = 24.0;
const GAP: f32 = 8.0;
const OWNER_W: f32 = 70.0;
/// The unmapped list well shows at most this many rows before it scrolls.
const VISIBLE_ROWS: usize = 7;

/// What a press resolved to (everything is consumed while a modal is open).
#[derive(Debug, PartialEq)]
pub enum Press {
	Consumed,
	/// Close the modal (Cancel / Abort / click-away).
	Cancel,
	/// Run the match against the selected packs (Settings "Import").
	Match,
	/// Commit the import using `finish_dest` (Unmapped finish buttons).
	Finish,
}

/// A held command button, fired on release-inside (drag-off cancels).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Btn {
	Cancel,
	Import,
	Ignore,
}

pub struct ImportWrl {
	pub(crate) path: PathBuf,
	name: String,
	/// (width, height, tile_count) from the WRL header.
	info: (u16, u16, u16),

	// Settings stage.
	packs: Vec<PackEntry>,
	palette_owner: Option<String>,

	// Unmapped stage (populated by `set_result` after a match).
	result: Option<map_core::WrlImport>,
	/// One display row per unmapped tile (id · class · cell count).
	rows: Vec<String>,
	matched: usize,
	used: usize,
	scroll: f32,
	/// The Import-tiles destination toggle (Ignore is a separate button).
	dest: ExtrasDest,
	/// Which destination the pressed finish button commits with.
	finish_dest: ExtrasDest,

	armed: Option<Btn>,
	pub(crate) drag_offset: (f32, f32),
}

impl ImportWrl {
	pub fn new(
		path: PathBuf,
		name: String,
		width: u16,
		height: u16,
		tiles: u16,
		assets_root: &std::path::Path,
	) -> Self {
		Self {
			path,
			name,
			info: (width, height, tiles),
			packs: packlist::scan(assets_root),
			palette_owner: None,
			result: None,
			rows: Vec::new(),
			matched: 0,
			used: 0,
			scroll: 0.0,
			dest: ExtrasDest::ProjectPack,
			finish_dest: ExtrasDest::Ignore,
			armed: None,
			drag_offset: (0.0, 0.0),
		}
	}

	/// The selected packs (WATER first, owner next) for the match.
	pub fn selected_packs(&self) -> Vec<String> {
		packlist::selected(&self.packs, &self.palette_owner)
	}

	/// The WRL's base name (used for the converted project + extras pack id).
	pub fn map_name(&self) -> &str {
		&self.name
	}

	/// The palette-owner pack name - the user-tileset extras target.
	pub fn owner(&self) -> String {
		packlist::effective_owner(&self.packs, &self.palette_owner).unwrap_or_default()
	}

	/// Whether the selection can build a map (at least one palette owner).
	pub fn has_owner(&self) -> bool {
		packlist::has_palette_owner(&self.packs)
	}

	/// Park the match result and switch to the unmapped-review stage.
	pub fn set_result(&mut self, import: map_core::WrlImport) {
		self.used = import.used_tiles();
		self.matched = import.matched_tiles();
		self.rows = import
			.unmapped()
			.iter()
			.map(|u| {
				format!("{}   {}   {} cell{}", u.id, class_name(u.pass), u.cells, if u.cells == 1 { "" } else { "s" })
			})
			.collect();
		self.result = Some(import);
	}

	/// Take the parked match result (to commit) and the chosen destination.
	pub fn take_result(&mut self) -> Option<(map_core::WrlImport, ExtrasDest)> {
		self.result.take().map(|r| (r, self.finish_dest))
	}

	fn in_unmapped(&self) -> bool {
		self.result.is_some()
	}

	// ----- geometry ------------------------------------------------------------

	fn well_rows(&self) -> usize {
		self.rows.len().min(VISIBLE_ROWS)
	}

	fn content_h(&self) -> f32 {
		self.rows.len() as f32 * LINE_H
	}

	fn max_scroll(&self) -> f32 {
		(self.content_h() - self.well_rows() as f32 * LINE_H).max(0.0)
	}

	fn height(&self) -> f32 {
		if self.in_unmapped() {
			TITLE_H + PAD + LINE_H + GAP + self.well_rows() as f32 * LINE_H + GAP + BTN_H + GAP + BTN_H + PAD
		} else {
			TITLE_H + PAD + LINE_H + GAP + self.packs.len().max(1) as f32 * PACK_ROW + GAP + BTN_H + PAD
		}
	}

	pub fn dialog_rect(&self, w: f32, h: f32) -> Rect {
		Rect::centered(w, h, W, self.height()).translate(self.drag_offset.0, self.drag_offset.1)
	}

	fn pack_row_rect(&self, d: Rect, i: usize) -> Rect {
		let y = d.y + TITLE_H + PAD + LINE_H + GAP + i as f32 * PACK_ROW;
		Rect::new(d.x + PAD, y, W - 2.0 * PAD - OWNER_W - 6.0, PACK_ROW - 4.0)
	}

	fn owner_rect(&self, d: Rect, i: usize) -> Rect {
		let r = self.pack_row_rect(d, i);
		Rect::new(d.x + W - PAD - OWNER_W, r.y, OWNER_W, r.h)
	}

	fn list_well(&self, d: Rect) -> Rect {
		let y = d.y + TITLE_H + PAD + LINE_H + GAP;
		Rect::new(d.x + PAD, y, W - 2.0 * PAD, self.well_rows() as f32 * LINE_H)
	}

	/// The destination toggle row (Unmapped stage), above the buttons.
	fn dest_rect(&self, d: Rect, i: usize) -> Rect {
		let y = d.y + d.h - PAD - 2.0 * BTN_H - GAP;
		Rect::new(d.x + W - PAD - 2.0 * 96.0 - 6.0 + i as f32 * (96.0 + 6.0), y, 96.0, BTN_H)
	}

	// Bottom button row. Settings: [Cancel] ........ [Import]. Unmapped:
	// [Abort] .... [Ignore missing] [Import tiles].
	fn cancel_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + PAD, d.y + d.h - PAD - BTN_H, 86.0, BTN_H)
	}

	fn import_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + W - PAD - 104.0, d.y + d.h - PAD - BTN_H, 104.0, BTN_H)
	}

	fn ignore_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + W - PAD - 104.0 - 6.0 - 100.0, d.y + d.h - PAD - BTN_H, 100.0, BTN_H)
	}

	// ----- events --------------------------------------------------------------

	pub fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		if self.cancel_rect(d).contains(x, y) {
			self.armed = Some(Btn::Cancel);
			return Press::Consumed;
		}
		if self.import_rect(d).contains(x, y) {
			self.armed = Some(Btn::Import);
			return Press::Consumed;
		}
		if self.in_unmapped() {
			if self.ignore_rect(d).contains(x, y) {
				self.armed = Some(Btn::Ignore);
				return Press::Consumed;
			}
			for (i, dst) in [ExtrasDest::ProjectPack, ExtrasDest::UserTileset].into_iter().enumerate() {
				if self.dest_rect(d, i).contains(x, y) {
					self.dest = dst;
					return Press::Consumed;
				}
			}
		} else {
			// Pack checkboxes + palette-owner radios.
			for i in 0..self.packs.len() {
				if self.packs[i].has_palette && !self.packs[i].locked && self.owner_rect(d, i).contains(x, y) {
					self.palette_owner = Some(self.packs[i].name.clone());
					self.packs[i].selected = true;
					return Press::Consumed;
				}
			}
			for i in 0..self.packs.len() {
				if !self.packs[i].locked && self.pack_row_rect(d, i).contains(x, y) {
					self.packs[i].selected = !self.packs[i].selected;
					return Press::Consumed;
				}
			}
		}
		if !d.contains(x, y) {
			return Press::Cancel;
		}
		Press::Consumed
	}

	pub fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		match self.armed.take() {
			Some(Btn::Cancel) if self.cancel_rect(d).contains(x, y) => Press::Cancel,
			Some(Btn::Import) if self.import_rect(d).contains(x, y) => {
				if self.in_unmapped() {
					self.finish_dest = self.dest;
					Press::Finish
				} else {
					Press::Match
				}
			}
			Some(Btn::Ignore) if self.in_unmapped() && self.ignore_rect(d).contains(x, y) => {
				self.finish_dest = ExtrasDest::Ignore;
				Press::Finish
			}
			_ => Press::Consumed,
		}
	}

	/// Wheel scrolls the unmapped list (one row per notch).
	pub fn scroll_by(&mut self, steps: f32) {
		self.scroll = (self.scroll - steps * LINE_H).clamp(0.0, self.max_scroll());
	}

	/// Enter: run the match (Settings) or commit with the toggle destination
	/// (Unmapped - the same as the "Import tiles" button).
	pub fn confirm_key(&mut self) -> Press {
		if self.in_unmapped() {
			self.finish_dest = self.dest;
			Press::Finish
		} else {
			Press::Match
		}
	}

	/// Esc steps back from the unmapped review to the settings stage
	/// (discarding the match, which is cheap to redo); returns whether it did.
	pub fn back(&mut self) -> bool {
		if self.in_unmapped() {
			self.result = None;
			self.rows.clear();
			self.scroll = 0.0;
			true
		} else {
			false
		}
	}

	// ----- drawing -------------------------------------------------------------

	pub fn view(&self, w: f32, h: f32, hot: Hot) -> UiQuads {
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(SteelMap::anchored(d));
		ui::modal_scrim(&mut q, w, h);
		ui::modal_frame(&mut q, d, "Import WRL", TITLE_H, w, h);
		let f = ui::FONT_SMALL;
		let x = d.x + PAD;
		let y0 = d.y + TITLE_H + PAD;

		if self.in_unmapped() {
			let (mw, mh, _) = self.info;
			let head =
				format!("{}×{} · {}/{} tiles matched · {} unmapped", mw, mh, self.matched, self.used, self.rows.len());
			q.label(&crate::text::fit_label(&head, f, W - 2.0 * PAD), x, y0, f, w, h, theme::INK);
			let well = self.list_well(d);
			q.field(well, w, h);
			q.scrollbar(well, self.content_h(), self.scroll, w, h, hot);

			q.label("Missing tiles", x, self.dest_rect(d, 0).y + 5.0, f, w, h, theme::INK_DIM);
			let owner = self.cur_owner();
			for (i, (dst, name)) in
				[(ExtrasDest::ProjectPack, "This project"), (ExtrasDest::UserTileset, owner.as_str())]
					.into_iter()
					.enumerate()
			{
				q.toggle_button(self.dest_rect(d, i), name, self.dest == dst, true, f, w, h, hot);
			}

			q.button(self.cancel_rect(d), w, h, hot);
			q.label_in("Abort", self.cancel_rect(d), 8.0, f, w, h, theme::INK_DIM);
			q.button(self.ignore_rect(d), w, h, hot);
			q.label_in("Ignore missing", self.ignore_rect(d), 8.0, f, w, h, theme::INK_DIM);
			q.button_primary(self.import_rect(d), w, h, hot);
			q.label_in("Import tiles", self.import_rect(d), 8.0, f, w, h, theme::INK);
		} else {
			let (mw, mh, tiles) = self.info;
			let head = format!("{}.WRL — {}×{}, {} tiles", self.name, mw, mh, tiles);
			q.label(&head, x, y0, f, w, h, theme::INK);

			let owner = packlist::effective_owner(&self.packs, &self.palette_owner);
			for (i, p) in self.packs.iter().enumerate() {
				let r = self.pack_row_rect(d, i);
				let label = if p.locked { format!("{} (base)", p.name) } else { p.name.clone() };
				q.toggle_button(r, &label, p.selected, !p.locked, f, w, h, hot);
				if p.has_palette && !p.locked {
					let or = self.owner_rect(d, i);
					let owns = owner.as_deref() == Some(p.name.as_str());
					q.toggle_button(or, "palette", owns, true, f, w, h, hot);
				}
			}

			q.button(self.cancel_rect(d), w, h, hot);
			q.label_in("Cancel", self.cancel_rect(d), 8.0, f, w, h, theme::INK_DIM);
			q.button_primary(self.import_rect(d), w, h, hot);
			q.label_in("Import", self.import_rect(d), 8.0, f, w, h, theme::INK);
		}
		q
	}

	/// The scrolling unmapped rows + the clip rect the shell scissors them to
	/// (empty in the Settings stage).
	pub fn list_content(&self, w: f32, h: f32) -> (UiQuads, Rect) {
		let well = self.list_well(self.dialog_rect(w, h));
		let mut q = UiQuads::default();
		if !self.in_unmapped() {
			return (q, well);
		}
		let row_w = well.w - ui::SCROLLBAR_W - 4.0;
		for (i, row) in self.rows.iter().enumerate() {
			let ry = well.y + i as f32 * LINE_H - self.scroll;
			if ry + LINE_H < well.y || ry > well.y + well.h {
				continue;
			}
			q.label_fit(row, Rect::new(well.x + 4.0, ry, row_w, LINE_H), 0.0, ui::FONT_SMALL, w, h, theme::INK_DIM);
		}
		(q, well)
	}

	/// The owner pack label for the user-tileset toggle (falls back to "user").
	fn cur_owner(&self) -> String {
		let o = self.owner();
		if o.is_empty() { "User tileset".to_string() } else { format!("→ {o}") }
	}
}

/// A passability value's human label for the unmapped list.
fn class_name(pass: u8) -> &'static str {
	match pass {
		1 => "water",
		2 => "shore",
		3 => "blocked",
		_ => "land",
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn assets_root() -> std::path::PathBuf {
		std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources/assets/tilepacks")
	}

	fn settings_modal() -> ImportWrl {
		ImportWrl::new(PathBuf::from("X.WRL"), "X".into(), 4, 4, 3, &assets_root())
	}

	/// A modal parked in the unmapped stage: a 1×1 WRL of one bogus tile that
	/// matches nothing, so the match leaves it unmapped.
	fn unmapped_modal() -> ImportWrl {
		let wrl = max_assets::wrl::WrlFile {
			header: vec![0; 5],
			width: 1,
			height: 1,
			minimap: vec![0],
			bigmap: vec![0],
			tile_count: 1,
			tiles: vec![200u8; 4096],
			palette: vec![0; 768],
			pass_table: vec![0],
		};
		let import = map_core::WrlImport::new(wrl, "X", "GREEN", &["GREEN".to_string()], &assets_root(), 0).unwrap();
		assert_eq!(import.unmapped().len(), 1, "the bogus tile is unmapped");
		let mut m = settings_modal();
		m.set_result(import);
		m
	}

	#[test]
	fn settings_import_arms_then_matches_and_clickout_cancels() {
		let (w, h) = (1280.0, 800.0);
		let mut m = settings_modal();
		let r = m.import_rect(m.dialog_rect(w, h));
		assert_eq!(m.on_press(r.x + 2.0, r.y + 2.0, w, h), Press::Consumed);
		assert_eq!(m.on_release(r.x + 2.0, r.y + 2.0, w, h), Press::Match);
		// Drag-off cancels the press; a click outside the dialog closes.
		m.on_press(r.x + 2.0, r.y + 2.0, w, h);
		assert_eq!(m.on_release(1.0, 1.0, w, h), Press::Consumed);
		assert_eq!(m.on_press(1.0, 1.0, w, h), Press::Cancel);
	}

	#[test]
	fn ignore_missing_finishes_with_ignore() {
		let (w, h) = (1280.0, 800.0);
		let mut m = unmapped_modal();
		let r = m.ignore_rect(m.dialog_rect(w, h));
		m.on_press(r.x + 2.0, r.y + 2.0, w, h);
		assert_eq!(m.on_release(r.x + 2.0, r.y + 2.0, w, h), Press::Finish);
		assert_eq!(m.take_result().unwrap().1, ExtrasDest::Ignore);
	}

	#[test]
	fn destination_toggle_drives_the_import_target() {
		let (w, h) = (1280.0, 800.0);
		let mut m = unmapped_modal();
		let d = m.dialog_rect(w, h);
		// Pick "User tileset" (toggle index 1), then Import tiles.
		let t = m.dest_rect(d, 1);
		assert_eq!(m.on_press(t.x + 2.0, t.y + 2.0, w, h), Press::Consumed);
		let r = m.import_rect(d);
		m.on_press(r.x + 2.0, r.y + 2.0, w, h);
		assert_eq!(m.on_release(r.x + 2.0, r.y + 2.0, w, h), Press::Finish);
		assert_eq!(m.take_result().unwrap().1, ExtrasDest::UserTileset);
	}

	#[test]
	fn esc_backs_out_of_the_unmapped_stage() {
		let mut m = unmapped_modal();
		assert!(m.back(), "Esc steps back from the unmapped review");
		assert!(!m.back(), "a second Esc has nowhere to go (would close)");
	}
}
