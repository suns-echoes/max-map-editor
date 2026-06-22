//! Save / rename a saved palette: a one-line name field with overwrite
//! confirmation. Save writes the working palette to `user/palettes/<name>.json`;
//! Rename moves an existing saved palette's file. Mirrors the Rename Template
//! modal (a [`TextInput`] + inline validation), minus the preview.

use std::path::PathBuf;

use crate::textinput::{Charset, TextInput};
use crate::theme;
use crate::ui::{self, Hot, Rect, SteelMap, UiQuads};

const W: f32 = 360.0;
const TITLE_H: f32 = 22.0;
const FIELD_H: f32 = 22.0;
const BTN_H: f32 = 24.0;
const PAD: f32 = 12.0;
const GAP: f32 = 8.0;
/// Left column for the "name" label.
const LABEL_W: f32 = 44.0;
/// The inline-alert / overwrite-prompt line below the field.
const ERR_H: f32 = 16.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
	/// Save the working palette under a new name.
	Save,
	/// Rename an existing saved palette's file.
	Rename,
}

pub struct PaletteName {
	mode: Mode,
	/// The editable palette name.
	input: TextInput,
	/// Other saved user-palette names, for the overwrite/duplicate check.
	existing: Vec<String>,
	/// Rename source `(name, file)`; `None` for Save.
	from: Option<(String, PathBuf)>,
	/// Inline alert / overwrite prompt (cleared as soon as the user edits).
	error: Option<String>,
	/// Armed for overwrite: the next confirm overwrites the existing file.
	overwrite_armed: bool,
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
	/// Resolved command line (`palette-save-as` / `palette-rename`).
	Run(String),
}

impl PaletteName {
	/// Save the working palette: the name field starts at `suggested`.
	pub fn save(existing: Vec<String>, suggested: &str) -> Self {
		Self::with(Mode::Save, suggested, None, existing)
	}

	/// Rename the saved palette `from` (at `from_path`) - the field pre-fills it.
	pub fn rename(from: &str, from_path: PathBuf, existing: Vec<String>) -> Self {
		Self::with(Mode::Rename, from, Some((from.to_string(), from_path)), existing)
	}

	fn with(mode: Mode, initial: &str, from: Option<(String, PathBuf)>, existing: Vec<String>) -> Self {
		Self {
			mode,
			input: TextInput::new(initial, 48).charset(Charset::Text),
			existing,
			from,
			error: None,
			overwrite_armed: false,
			armed: None,
			dragging_field: false,
			drag_offset: (0.0, 0.0),
		}
	}

	fn title(&self) -> &'static str {
		match self.mode {
			Mode::Save => "Save Palette",
			Mode::Rename => "Rename Palette",
		}
	}

	/// The focused field's edit state (for the right-click context menu).
	pub fn edit_context(&self) -> Option<crate::modal::EditContext> {
		Some(self.input.edit_context())
	}

	/// Validate the typed name and resolve the command. A first clash with an
	/// existing palette arms overwrite (shows the prompt) and returns `None`; the
	/// next confirm goes through.
	fn try_confirm(&mut self) -> Option<String> {
		let name = self.input.text().trim().to_string();
		if name.is_empty() {
			self.error = Some("the name is empty".into());
			return None;
		}
		if name.contains(['/', '\\']) {
			self.error = Some("no slashes in the name".into());
			return None;
		}
		let from_name = self.from.as_ref().map(|(n, _)| n.as_str());
		if from_name == Some(name.as_str()) {
			self.error = Some("the name is unchanged".into());
			return None;
		}
		let clash = self.existing.iter().any(|n| n == &name);
		if clash && !self.overwrite_armed {
			self.overwrite_armed = true;
			self.error = Some(format!("\"{name}\" exists - confirm again to overwrite"));
			return None;
		}
		Some(match &self.from {
			None => format!("palette-save-as \"{name}\""),
			Some((_, path)) => format!("palette-rename \"{}\" \"{name}\"", path.display()),
		})
	}

	// ----- geometry -----------------------------------------------------------

	fn height() -> f32 {
		// title | name | gap | alert | gap | buttons
		TITLE_H + PAD + FIELD_H + GAP + ERR_H + GAP + BTN_H + PAD
	}

	pub fn dialog_rect(&self, w: f32, h: f32) -> Rect {
		Rect::centered(w, h, W, Self::height()).translate(self.drag_offset.0, self.drag_offset.1)
	}

	fn field_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + PAD + LABEL_W, d.y + TITLE_H + PAD, W - 2.0 * PAD - LABEL_W, FIELD_H)
	}

	fn err_y(&self, d: Rect) -> f32 {
		self.field_rect(d).y + FIELD_H + GAP
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
			Some(1) if self.save_rect(d).contains(x, y) => self.confirm(),
			_ => Press::Consumed,
		}
	}

	/// Enter / Save: resolve the command or stay open with an alert.
	pub fn confirm(&mut self) -> Press {
		match self.try_confirm() {
			Some(line) => Press::Run(line),
			None => Press::Consumed,
		}
	}

	/// Route an editing key to the name field; editing clears the alert and
	/// disarms a pending overwrite (the name changed, so re-check it).
	pub fn key(&mut self, key: &crate::modal::ModalKey) {
		self.error = None;
		self.overwrite_armed = false;
		self.input.on_key(key);
	}

	// ----- drawing ------------------------------------------------------------

	pub fn view(&self, w: f32, h: f32, hot: Hot) -> UiQuads {
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(SteelMap::anchored(d));
		ui::modal_scrim(&mut q, w, h);
		ui::modal_frame(&mut q, d, self.title(), TITLE_H, w, h);

		let field = self.field_rect(d);
		q.label("name", d.x + PAD, field.y + (FIELD_H - 12.0) / 2.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		q.field(field, w, h);
		q.border(field, w, h, theme::ACCENT);

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
		let label = if self.overwrite_armed { "Overwrite" } else { "Save" };
		q.label_in(label, self.save_rect(d), 8.0, crate::ui::FONT_SMALL, w, h, theme::INK);
		q
	}

	/// The name field's text/caret/selection, with its clip rect.
	pub fn field_content(&self, w: f32, h: f32) -> (UiQuads, Rect) {
		let field = self.field_rect(self.dialog_rect(w, h));
		(self.input.content_quads(field, true, w, h), field)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::modal::ModalKey;

	fn retype(m: &mut PaletteName, text: &str) {
		m.key(&ModalKey::SelectAll);
		for c in text.chars() {
			m.key(&ModalKey::Char(c));
		}
	}

	#[test]
	fn save_resolves_command_and_confirms_overwrite() {
		let mut m = PaletteName::save(vec!["forest".into()], "");
		// Empty is refused.
		assert_eq!(m.confirm(), Press::Consumed);
		assert!(m.error.is_some());
		// A free name resolves straight away.
		retype(&mut m, "swamp");
		assert_eq!(m.confirm(), Press::Run("palette-save-as \"swamp\"".into()));
		// An existing name arms overwrite first, then commits on the next confirm.
		retype(&mut m, "forest");
		assert_eq!(m.confirm(), Press::Consumed, "first confirm arms overwrite");
		assert!(m.overwrite_armed);
		assert_eq!(m.confirm(), Press::Run("palette-save-as \"forest\"".into()));
	}

	#[test]
	fn rename_refuses_unchanged_and_quotes_the_path() {
		let mut m = PaletteName::rename("old", PathBuf::from("/u/old.json"), vec!["taken".into()]);
		// Unchanged is refused.
		assert_eq!(m.confirm(), Press::Consumed);
		assert!(m.error.as_deref().unwrap().contains("unchanged"));
		// A new free name resolves with the source path quoted.
		retype(&mut m, "new");
		assert_eq!(m.confirm(), Press::Run("palette-rename \"/u/old.json\" \"new\"".into()));
	}
}
