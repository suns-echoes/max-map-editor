//! Rename Template modal: a live thumbnail of the selected template plus a
//! one-line name field. Save emits `template-rename "from" "to"` (the same
//! command path scripts use); the shell renames the file and rescans.
//!
//! Pure state/geometry. The thumbnail is drawn through the tile pass (like the
//! Templates Explorer) and the editable name is clipped to its well - both are
//! emitted here and rendered by the shell.

use map_core::{Project, Template};

use crate::picker::{self, TileQuad};
use crate::textinput::TextInput;
use crate::theme;
use crate::ui::{self, Hot, Rect, SteelMap, UiQuads};

const W: f32 = 360.0;
const TITLE_H: f32 = 22.0;
/// Thumbnail well side (2x the original, a big readable preview).
const PREVIEW: f32 = 264.0;
const FIELD_H: f32 = 22.0;
const BTN_H: f32 = 24.0;
const PAD: f32 = 12.0;
const GAP: f32 = 8.0;
/// Left column for the row labels ("name", "size").
const LABEL_W: f32 = 44.0;
/// Breathing room between the preview and the first field row.
const PREVIEW_GAP: f32 = 20.0;
/// Gap between the name row and the size row.
const ROW_GAP: f32 = 8.0;
/// The inline-alert line below the rows (shown when a name is rejected).
const ERR_H: f32 = 16.0;

pub struct RenameTemplate {
	/// The template's current (file-stem) name - the rename source.
	from: String,
	/// A copy for the live preview.
	template: Template,
	/// The editable new name.
	input: TextInput,
	/// Other templates' display names - a rename to one of these is rejected so
	/// the user can correct it (no silent overwrite or numeral bump).
	existing: Vec<String>,
	/// The inline validation alert, cleared as soon as the user edits the name.
	error: Option<String>,
	/// Held button: 0 Cancel, 1 Save - dragging off cancels the click.
	armed: Option<usize>,
	/// True between a press inside the field and the release (mouse-select).
	dragging_field: bool,
	pub(crate) drag_offset: (f32, f32),
}

#[derive(Debug, PartialEq)]
pub enum Press {
	Consumed,
	Cancel,
	/// Validated `template-rename …` command line.
	Rename(String),
}

impl RenameTemplate {
	pub fn new(from: &str, template: Template, existing: Vec<String>) -> Self {
		Self {
			from: from.to_string(),
			template,
			input: TextInput::new(from, 64),
			existing,
			error: None,
			armed: None,
			dragging_field: false,
			drag_offset: (0.0, 0.0),
		}
	}

	/// Validate the typed name and build the rename command. The display name
	/// keeps the user's text; the shell sanitizes the *filename*. A name already
	/// used by another template is rejected (the user corrects it in place).
	fn validate(&self) -> Result<String, String> {
		let to = self.input.text().trim();
		if to.is_empty() {
			return Err("the name is empty".into());
		}
		if to == self.from {
			return Err("the name is unchanged".into());
		}
		if self.existing.iter().any(|n| n == to) {
			return Err(format!("a template named \"{to}\" already exists"));
		}
		// Quote both names so spaces survive the tokenizer.
		Ok(format!("template-rename \"{}\" \"{to}\"", self.from))
	}

	/// Validate for a Save: on success returns the command line; on failure
	/// stashes the alert (shown in-modal) and returns `None` so the modal stays
	/// open for the user to fix the name.
	pub fn try_confirm(&mut self) -> Option<String> {
		match self.validate() {
			Ok(line) => Some(line),
			Err(e) => {
				self.error = Some(e);
				None
			}
		}
	}

	// ----- geometry -----------------------------------------------------------

	fn height() -> f32 {
		// title | preview | gap | name | gap | size | gap | alert | gap | buttons
		TITLE_H + PAD + PREVIEW + PREVIEW_GAP + FIELD_H + ROW_GAP + FIELD_H + ROW_GAP + ERR_H + GAP + BTN_H + PAD
	}

	pub fn dialog_rect(&self, w: f32, h: f32) -> Rect {
		Rect::centered(w, h, W, Self::height()).translate(self.drag_offset.0, self.drag_offset.1)
	}

	/// The centered, square thumbnail well below the title.
	fn preview_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + (W - PREVIEW) / 2.0, d.y + TITLE_H + PAD, PREVIEW, PREVIEW)
	}

	/// The editable name field (right of the "name" label).
	fn field_rect(&self, d: Rect) -> Rect {
		let y = self.preview_rect(d).y + PREVIEW + PREVIEW_GAP;
		Rect::new(d.x + PAD + LABEL_W, y, W - 2.0 * PAD - LABEL_W, FIELD_H)
	}

	/// The read-only size row (right of the "size" label).
	fn size_rect(&self, d: Rect) -> Rect {
		let y = self.field_rect(d).y + FIELD_H + ROW_GAP;
		Rect::new(d.x + PAD + LABEL_W, y, W - 2.0 * PAD - LABEL_W, FIELD_H)
	}

	/// The inline-alert line, below the size row.
	fn err_y(&self, d: Rect) -> f32 {
		self.size_rect(d).y + FIELD_H + ROW_GAP
	}

	fn cancel_rect(&self, d: Rect) -> Rect {
		crate::ui::button_pair(d, W, PAD, BTN_H).0
	}

	fn save_rect(&self, d: Rect) -> Rect {
		crate::ui::button_pair(d, W, PAD, BTN_H).1
	}

	// ----- events -------------------------------------------------------------

	pub fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		let field = self.field_rect(d);
		if field.contains(x, y) {
			self.input.on_press(x, y, field);
			self.dragging_field = true;
			return Press::Consumed;
		}
		if self.cancel_rect(d).contains(x, y) {
			self.armed = Some(0);
			return Press::Consumed;
		}
		if self.save_rect(d).contains(x, y) {
			self.armed = Some(1);
			return Press::Consumed;
		}
		if !d.contains(x, y) {
			return Press::Cancel; // click-out cancels
		}
		Press::Consumed
	}

	pub fn on_drag(&mut self, x: f32, y: f32, w: f32, h: f32) {
		if self.dragging_field {
			let field = self.field_rect(self.dialog_rect(w, h));
			self.input.on_drag(x, y, field);
		}
	}

	pub fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		self.dragging_field = false;
		let d = self.dialog_rect(w, h);
		match self.armed.take() {
			Some(0) if self.cancel_rect(d).contains(x, y) => Press::Cancel,
			Some(1) if self.save_rect(d).contains(x, y) => match self.try_confirm() {
				Some(line) => Press::Rename(line),
				None => Press::Consumed, // alert shown in-modal; stay open
			},
			_ => Press::Consumed,
		}
	}

	/// The (always-focused) name field's edit state.
	pub fn edit_context(&self) -> Option<crate::modal::EditContext> {
		Some(self.input.edit_context())
	}

	/// Route an editing key to the (always-focused) name field; editing clears
	/// any pending alert.
	pub fn key(&mut self, key: &crate::modal::ModalKey) {
		self.error = None;
		self.input.on_key(key);
	}

	// ----- drawing ------------------------------------------------------------

	pub fn view(&self, w: f32, h: f32, hot: Hot) -> UiQuads {
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(SteelMap::anchored(d));
		ui::modal_scrim(&mut q, w, h);
		ui::modal_frame(&mut q, d, "Rename Template", TITLE_H, w, h);

		// Preview well (tiles blit over it).
		let pr = self.preview_rect(d);
		q.field(pr, w, h);

		// Name row: label + editable field.
		let field = self.field_rect(d);
		q.label("name", d.x + PAD, field.y + (FIELD_H - 12.0) / 2.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		q.field(field, w, h);
		q.border(field, w, h, theme::ACCENT);

		// Size row: a read-only footprint, laid out like the name row.
		let size = self.size_rect(d);
		q.label("size", d.x + PAD, size.y + (FIELD_H - 12.0) / 2.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		let dims = format!("{} x {}", self.template.width, self.template.height);
		q.label(&dims, size.x + 2.0, size.y + (FIELD_H - 12.0) / 2.0, crate::ui::FONT_SMALL, w, h, theme::INK);

		// Inline alert (e.g. name already exists) - the user fixes it in place.
		if let Some(msg) = &self.error {
			q.label_fit(
				msg,
				Rect::new(d.x + PAD, self.err_y(d), W - 2.0 * PAD, ERR_H),
				0.0,
				crate::ui::FONT_SMALL,
				w,
				h,
				theme::CLOSE_INK,
			);
		}

		q.button(self.cancel_rect(d), w, h, hot);
		q.label_in("Cancel", self.cancel_rect(d), 8.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		q.button_primary(self.save_rect(d), w, h, hot);
		q.label_in("Save", self.save_rect(d), 8.0, crate::ui::FONT_SMALL, w, h, theme::INK);
		q
	}

	/// The name field's text/caret/selection, with its clip rect.
	pub fn field_content(&self, w: f32, h: f32) -> (UiQuads, Rect) {
		let field = self.field_rect(self.dialog_rect(w, h));
		(self.input.content_quads(field, true, w, h), field)
	}

	/// The template's cells as tile quads scaled into the preview well, plus the
	/// clip rect to scissor them to. Resolves against the open `project` (the
	/// rename target is always a visible, compatible template).
	pub fn preview_tiles(&self, project: &Project, w: f32, h: f32) -> (Vec<TileQuad>, Rect) {
		let pr = self.preview_rect(self.dialog_rect(w, h));
		let t = &self.template;
		let span = t.width.max(t.height).max(1) as f32;
		let px = (PREVIEW - 8.0) / span;
		let (ox, oy) = (
			pr.x + 4.0 + (PREVIEW - 8.0 - t.width as f32 * px) / 2.0,
			pr.y + 4.0 + (PREVIEW - 8.0 - t.height as f32 * px) / 2.0,
		);
		let mut tiles = Vec::new();
		for dy in 0..t.height {
			for dx in 0..t.width {
				for tile in t.cell_layers(project, dx, dy).into_iter().flatten() {
					tiles.push(TileQuad {
						index: picker::global_index(project, tile),
						transform: tile.transform.bits(),
						rect: Rect::new(ox + dx as f32 * px, oy + dy as f32 * px, px, px),
					});
				}
			}
		}
		(tiles, pr)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::modal::ModalKey;

	fn tmpl() -> Template {
		Template { name: "Lake".into(), width: 1, height: 1, uses: Vec::new(), cells: vec![String::new()] }
	}

	/// Type `text` into the (always-focused) name field, replacing what's there.
	fn retype(m: &mut RenameTemplate, text: &str) {
		m.key(&ModalKey::SelectAll);
		for c in text.chars() {
			m.key(&ModalKey::Char(c));
		}
	}

	#[test]
	fn rename_rejects_duplicate_name_and_keeps_modal_open() {
		let mut m = RenameTemplate::new("Lake", tmpl(), vec!["Forest".into(), "Hill".into()]);
		// A name already in use is rejected (alert set, no command, stays open).
		retype(&mut m, "Forest");
		assert_eq!(m.try_confirm(), None);
		assert!(m.error.as_deref().unwrap().contains("already exists"));
		// Editing clears the alert; a free name confirms.
		retype(&mut m, "Brook");
		assert!(m.error.is_none(), "typing clears the alert");
		assert_eq!(m.try_confirm(), Some("template-rename \"Lake\" \"Brook\"".to_string()));
		// Empty and unchanged are also refused.
		retype(&mut m, "Lake");
		assert!(m.try_confirm().is_none(), "unchanged refused");
		retype(&mut m, "");
		assert!(m.try_confirm().is_none(), "empty refused");
	}
}
