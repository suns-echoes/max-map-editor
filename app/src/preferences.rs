//! Map Preferences modal: name / players / description / date / version /
//! author - all optional. The text rows are full editors ([`TextInput`]) with
//! caret, selection, and clipboard; players is a small segmented control.
//! Save writes the metadata via `Project::set_info`.
//!
//! Geometry + state here; the editable text is drawn clipped to each field by
//! the shell (so long values scroll within the well).

use crate::textinput::TextInput;
use crate::theme;
use crate::ui::{self, FONT_SMALL, Hot, Rect, UiQuads};

const W: f32 = 460.0;
const TITLE_H: f32 = 22.0;
const ROW_H: f32 = 28.0;
const LABEL_W: f32 = 92.0;
const BTN_H: f32 = 22.0;
/// The description row is a 5-line word-wrapped box.
const DESC_ROW: usize = 2;
const DESC_H: f32 = 82.0;
/// Extra height the tall description adds over a normal row (pushes the rows
/// below it — date/version/author — down).
const DESC_EXTRA: f32 = DESC_H - (ROW_H - 8.0);

/// Text fields, in tab order (players is a separate control).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FieldId {
	Name,
	Description,
	Date,
	Version,
	Author,
}

const FIELDS: [(FieldId, &str, usize); 5] = [
	(FieldId::Name, "Name", 0),
	(FieldId::Description, "Description", 2),
	(FieldId::Date, "Date", 3),
	(FieldId::Version, "Version", 4),
	(FieldId::Author, "Author", 5),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArmedBtn {
	Abort,
	Save,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Press {
	Consumed,
	Close,
	Save,
}

pub struct Preferences {
	name: TextInput,
	players: Option<u8>,
	description: TextInput,
	date: TextInput,
	version: TextInput,
	author: TextInput,
	/// Index into [`FIELDS`] of the focused text row.
	focus: usize,
	/// The field index currently being mouse-drag-selected (press..release).
	drag_field: Option<usize>,
	armed: Option<ArmedBtn>,
	pub(crate) drag_offset: (f32, f32),
}

impl Preferences {
	pub fn from_project(p: &map_core::Project) -> Self {
		Self {
			name: TextInput::new(&p.name, 48),
			players: p.players,
			description: TextInput::new_multiline(&p.description, 400, 5),
			date: TextInput::new(&p.date, 24),
			version: TextInput::new(&p.map_version, 24),
			author: TextInput::new(&p.author, 48),
			focus: 0,
			drag_field: None,
			armed: None,
			drag_offset: (0.0, 0.0),
		}
	}

	/// The collected values, ready for `Project::set_info`.
	pub fn values(&self) -> (String, Option<u8>, String, String, String, String) {
		(
			self.name.text().to_string(),
			self.players,
			self.description.text().to_string(),
			self.date.text().to_string(),
			self.version.text().to_string(),
			self.author.text().to_string(),
		)
	}

	fn field_mut(&mut self, id: FieldId) -> &mut TextInput {
		match id {
			FieldId::Name => &mut self.name,
			FieldId::Description => &mut self.description,
			FieldId::Date => &mut self.date,
			FieldId::Version => &mut self.version,
			FieldId::Author => &mut self.author,
		}
	}

	fn field_ref(&self, id: FieldId) -> &TextInput {
		match id {
			FieldId::Name => &self.name,
			FieldId::Description => &self.description,
			FieldId::Date => &self.date,
			FieldId::Version => &self.version,
			FieldId::Author => &self.author,
		}
	}

	pub fn focus_next(&mut self) {
		self.focus = (self.focus + 1) % FIELDS.len();
	}

	/// The focused text row's edit state.
	pub fn edit_context(&self) -> Option<crate::modal::EditContext> {
		let f = self.field_ref(FIELDS[self.focus].0);
		Some(f.edit_context())
	}

	/// Route an editing key to the focused field.
	pub fn key_focused(&mut self, key: &crate::modal::ModalKey) {
		let id = FIELDS[self.focus].0;
		self.field_mut(id).on_key(key);
	}

	/// Whether the focused field is the multiline description (Enter → newline).
	pub fn focused_wants_newline(&self) -> bool {
		self.field_ref(FIELDS[self.focus].0).wants_newline()
	}

	/// Route an editing key to the focused field, scrolling its caret into view
	/// (the multiline description needs the field rect, hence `w`/`h`).
	pub fn key_focused_at(&mut self, key: &crate::modal::ModalKey, w: f32, h: f32) {
		let (id, _, row) = FIELDS[self.focus];
		let r = self.field_rect(self.dialog_rect(w, h), row);
		self.field_mut(id).on_key_in(key, r);
	}

	/// Insert a newline in the focused (multiline) field and keep the caret visible.
	pub fn newline_focused(&mut self, w: f32, h: f32) {
		let (id, _, row) = FIELDS[self.focus];
		let r = self.field_rect(self.dialog_rect(w, h), row);
		let field = self.field_mut(id);
		field.insert_newline();
		field.scroll_caret_into_view(r);
	}

	/// Wheel scrolls the focused field when it is the multiline description.
	pub fn wheel_focused(&mut self, steps: f32, w: f32, h: f32) {
		let (id, _, row) = FIELDS[self.focus];
		if self.field_ref(id).wants_newline() {
			let r = self.field_rect(self.dialog_rect(w, h), row);
			self.field_mut(id).on_wheel(steps, r);
		}
	}

	// ----- geometry ----------------------------------------------------------

	pub fn dialog_rect(&self, w: f32, h: f32) -> Rect {
		let dh = TITLE_H + 8.0 + 6.0 * ROW_H + DESC_EXTRA + 10.0 + BTN_H + 12.0;
		Rect::centered(w, h, W, dh).translate(self.drag_offset.0, self.drag_offset.1)
	}

	fn row_y(&self, d: Rect, row: usize) -> f32 {
		// Rows below the (tall, multiline) description are pushed down by its
		// extra height.
		let extra = if row > DESC_ROW { DESC_EXTRA } else { 0.0 };
		d.y + TITLE_H + 8.0 + row as f32 * ROW_H + extra
	}

	/// The well rect for a text row — the description is `DESC_H` tall (wrapped).
	fn field_rect(&self, d: Rect, row: usize) -> Rect {
		let height = if row == DESC_ROW { DESC_H } else { ROW_H - 8.0 };
		Rect::new(d.x + LABEL_W, self.row_y(d, row) + 3.0, d.w - LABEL_W - 12.0, height)
	}

	/// One of the three players buttons (`2`, `2-3`, `2-4`) on row 1 — the
	/// stored value is the *max* count (2/3/4).
	fn players_btn_rect(&self, d: Rect, i: usize) -> Rect {
		Rect::new(d.x + LABEL_W + i as f32 * 52.0, self.row_y(d, 1) + 3.0, 46.0, ROW_H - 8.0)
	}

	fn abort_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + 10.0, d.y + d.h - BTN_H - 10.0, 90.0, BTN_H)
	}

	fn save_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + d.w - 100.0, d.y + d.h - BTN_H - 10.0, 90.0, BTN_H)
	}

	// ----- events -------------------------------------------------------------

	pub fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		// Text rows: focus + place the caret at the click.
		for (i, &(id, _, row)) in FIELDS.iter().enumerate() {
			let r = self.field_rect(d, row);
			if r.contains(x, y) {
				self.focus = i;
				self.drag_field = Some(i);
				self.field_mut(id).on_press(x, y, r);
				return Press::Consumed;
			}
		}
		// Players segmented control: buttons map to a max count 2/3/4; clicking
		// the active one again clears it (the field is optional).
		for i in 0..3 {
			if self.players_btn_rect(d, i).contains(x, y) {
				let v = i as u8 + 2;
				self.players = if self.players == Some(v) { None } else { Some(v) };
				return Press::Consumed;
			}
		}
		if self.abort_rect(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Abort);
			return Press::Consumed;
		}
		if self.save_rect(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Save);
			return Press::Consumed;
		}
		Press::Consumed
	}

	/// Mouse drag extends the active field's selection (after a press on it).
	pub fn drag_select(&mut self, x: f32, y: f32, w: f32, h: f32) {
		if let Some(i) = self.drag_field {
			let (id, _, row) = FIELDS[i];
			let r = self.field_rect(self.dialog_rect(w, h), row);
			self.field_mut(id).on_drag(x, y, r);
		}
	}

	pub fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		self.drag_field = None;
		let d = self.dialog_rect(w, h);
		match self.armed.take() {
			Some(ArmedBtn::Abort) if self.abort_rect(d).contains(x, y) => Press::Close,
			Some(ArmedBtn::Save) if self.save_rect(d).contains(x, y) => Press::Save,
			_ => Press::Consumed,
		}
	}

	// ----- drawing ------------------------------------------------------------

	/// The modal chrome (frame, labels, field wells, players control, buttons),
	/// without the editable text - that's drawn clipped by [`Self::field_contents`].
	pub fn view(&self, w: f32, h: f32, hot: Hot) -> UiQuads {
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(ui::SteelMap::anchored(d));
		ui::modal_scrim(&mut q, w, h);
		ui::modal_frame(&mut q, d, "Map Preferences", TITLE_H, w, h);

		for (i, &(_, label, row)) in FIELDS.iter().enumerate() {
			q.label(label, d.x + 12.0, self.row_y(d, row) + 8.0, FONT_SMALL, w, h, theme::INK_DIM);
			let r = self.field_rect(d, row);
			q.field(r, w, h);
			if i == self.focus {
				q.border(r, w, h, theme::ACCENT);
			}
		}

		// Players row: max-count buttons, label centred.
		q.label("Players", d.x + 12.0, self.row_y(d, 1) + 8.0, FONT_SMALL, w, h, theme::INK_DIM);
		for (i, label) in ["2", "2-3", "2-4"].iter().enumerate() {
			let r = self.players_btn_rect(d, i);
			let on = self.players == Some(i as u8 + 2);
			q.button_active(r, w, h, on, hot);
			let tw = crate::text::label_width(label, FONT_SMALL);
			q.label_in(label, r, (r.w - tw) / 2.0, FONT_SMALL, w, h, if on { theme::INK } else { theme::INK_DIM });
		}

		q.button(self.abort_rect(d), w, h, hot);
		q.label_in("Abort", self.abort_rect(d), 8.0, FONT_SMALL, w, h, theme::INK_DIM);
		q.button_primary(self.save_rect(d), w, h, hot);
		q.label_in("Save", self.save_rect(d), 8.0, FONT_SMALL, w, h, theme::INK);
		q
	}

	/// Per-text-field content (text + selection + caret) with the scissor rect
	/// the shell clips it to.
	pub fn field_contents(&self, w: f32, h: f32) -> Vec<(UiQuads, Rect)> {
		let d = self.dialog_rect(w, h);
		FIELDS
			.iter()
			.enumerate()
			.map(|(i, &(id, _, row))| {
				let r = self.field_rect(d, row);
				(self.field_ref(id).content_quads(r, i == self.focus, w, h), r)
			})
			.collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn assets_root() -> std::path::PathBuf {
		std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources/assets/tilepacks")
	}

	#[test]
	fn from_project_round_trips_the_info_fields() {
		let mut p = map_core::Project::new(8, 8, &["GREEN".into()], &assets_root(), 1).unwrap();
		p.name = "Coastline".into();
		p.players = Some(3);
		p.description = "two\nlines".into();
		p.date = "2026".into();
		p.map_version = "1.2".into();
		p.author = "me".into();
		assert_eq!(
			Preferences::from_project(&p).values(),
			("Coastline".into(), Some(3), "two\nlines".into(), "2026".into(), "1.2".into(), "me".into())
		);
	}

	#[test]
	fn players_button_selects_then_toggles_off() {
		let p = map_core::Project::new(8, 8, &["GREEN".into()], &assets_root(), 1).unwrap();
		let mut prefs = Preferences::from_project(&p); // players starts None
		let (w, h) = (1000.0, 800.0);
		let d = prefs.dialog_rect(w, h);
		// Button index 1 → max count 3.
		let click = |prefs: &mut Preferences| {
			let r = prefs.players_btn_rect(d, 1);
			prefs.on_press(r.x + r.w / 2.0, r.y + r.h / 2.0, w, h);
		};
		click(&mut prefs);
		assert_eq!(prefs.values().1, Some(3), "clicking a players button selects it");
		click(&mut prefs);
		assert_eq!(prefs.values().1, None, "re-clicking the active button clears it");
	}
}
