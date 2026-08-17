//! The small one-field prompts: template/scenery rename and delete, the
//! palette name prompt, and the generic verb-carrying name/value fields.

use super::*;

#[derive(Clone, Copy)]
pub(super) struct PaletteNameIds {
	pub(super) cancel: WidgetId,
	pub(super) save: WidgetId,
	pub(super) name: WidgetId,
	pub(super) error: WidgetId,
}

#[derive(Clone, Copy)]
pub(super) struct RenameTemplateIds {
	pub(super) cancel: WidgetId,
	pub(super) save: WidgetId,
	pub(super) name: WidgetId,
	pub(super) error: WidgetId,
}

/// A one-field name prompt that emits `<verb> "typed"` - the shape any
/// "rename this thing" dialog wants, with the thing's identity already
/// resolved by whoever opened it.
#[derive(Clone, Copy)]
pub(super) struct NamePromptIds {
	pub(super) cancel: WidgetId,
	pub(super) save: WidgetId,
	pub(super) name: WidgetId,
	pub(super) error: WidgetId,
}

#[derive(Clone, Copy)]
pub(super) struct ObjectFieldIds {
	pub(super) cancel: WidgetId,
	pub(super) ok: WidgetId,
	pub(super) value: WidgetId,
	pub(super) error: WidgetId,
}

/// Palette Save/Rename prompt state: the rename source `(name, file)`
/// (`None` for Save), the other user-palette names (clash check), and the
/// name armed for overwrite (a second confirm on the same clashing name
/// commits). Reset by [`Overlay::hide`].
#[derive(Default)]
pub(super) struct PaletteNameState {
	pub(super) from: Option<(String, PathBuf)>,
	pub(super) existing: Vec<String>,
	pub(super) armed: Option<String>,
}

/// Rename Template prompt state: the current (source) name + the sibling
/// names a rename can't collide with. Reset by [`Overlay::hide`].
#[derive(Default)]
pub(super) struct RenameTemplateState {
	pub(super) from: String,
	pub(super) existing: Vec<String>,
}

/// Name-prompt state: the command verb Save emits, and the name it opened
/// with (an unchanged name is a quiet close, not a pointless write). Reset by
/// [`Overlay::hide`].
#[derive(Default)]
pub(super) struct NamePromptState {
	pub(super) verb: String,
	pub(super) from: String,
}

/// Object-field prompt state: the command verb (`object-edit` for
/// current-state fields, `object-values` for max-stat fields, S4.5), the
/// target field (`name`/`hits`/`ammo`/...), and its numeric upper bound
/// (`None` = a free-text name). Reset by [`Overlay::hide`].
#[derive(Default)]
pub(super) struct ObjectFieldState {
	pub(super) verb: String,
	pub(super) field: String,
	pub(super) max: Option<u32>,
}

impl Overlay {
	/// Registers (or replaces) the shared template-preview texture with `rgba`
	/// (`tw`×`th` px) and returns an `Image` sized to fit `PREVIEW_SQ`,
	/// preserving the template's aspect. The composed thumbnail is a frozen
	/// snapshot (see `template_preview`).
	fn template_image(&mut self, chrome: &mut MenuChrome, rgba: &[u8], tw: u32, th: u32) -> Image {
		const PREVIEW_SQ: f32 = 264.0;
		let tex = match self.template_tex {
			Some(t) => {
				chrome.replace_texture(t, rgba, tw, th);
				t
			}
			None => {
				let t = chrome.register_texture(rgba, tw, th);
				self.template_tex = Some(t);
				t
			}
		};
		let span = tw.max(th).max(1) as f32;
		let (dw, dh) = (PREVIEW_SQ * tw as f32 / span, PREVIEW_SQ * th as f32 / span);
		Image::sized(tex, dw, dh)
	}

	pub fn open_rename_template(
		&mut self,
		chrome: &mut MenuChrome,
		from: &str,
		footprint: (u16, u16),
		existing: Vec<String>,
		rgba: &[u8],
		tw: u32,
		th: u32,
	) {
		let preview = self.template_image(chrome, rgba, tw, th);
		let (fw, fh) = footprint;
		let name = TextInput::with_text(from);
		let error = Label::new("").small().with_id();
		let cancel = Button::new("Cancel").secondary();
		let save = Button::new("Save").primary();
		let ids = RenameTemplateIds { cancel: cancel.id(), save: save.id(), name: name.id(), error: error.id() };
		let content = column()
			.push(width_strut(340.0))
			.push(Linear::row().main_align(MainAlign::Center).push(preview))
			.push(field_row("Name", name))
			.push(Label::new(format!("size   {fw}x{fh}")).small().muted());
		let content = status_slot(content, error, 340.0, 2).push(buttons(cancel, save));
		let win = dialog("Rename Template", content);
		self.win_id = Some(win.id());
		self.ui = Ui::new(modal(win));
		self.ui.focus_first();
		self.dialog = Dialog::RenameTemplate(ids);
		self.rt.from = from.to_string();
		self.rt.existing = existing;
		self.events.clear();
		self.visible = true;
	}

	/// Opens the Delete Scenery confirm. Says how many placements the deletion
	/// makes inert, because a cut-out is referenced by name from every map that
	/// uses it and the count is the only thing the user cannot see for
	/// themselves.
	pub fn open_delete_scenery(&mut self, pack: &str, id: &str, name: &str, placed: usize) {
		let note = match placed {
			0 => "It is not placed on this map.".to_string(),
			1 => "One object on this map uses it, and will stop drawing.".to_string(),
			n => format!("{n} objects on this map use it, and will stop drawing."),
		};
		let content = column()
			.push(width_strut(430.0))
			.push(Label::new(format!("\"{name}\"   ({pack} / {id})")).small())
			.push(Label::new(note).small().muted().wrap_at(430.0))
			.push(Label::new("Deleting a scenery object cannot be undone.").small().muted().wrap_at(430.0));
		let delete = Button::new("Delete").danger();
		self.open_confirm_parts("Delete Scenery", content, delete, "Cancel", "scenery-delete!".into());
	}

	/// Opens the Rename Scenery dialog. Only the display name moves: the id is
	/// what a placement stores, so renaming it would orphan every object
	/// already on a map - the dialog says so rather than leaving the user to
	/// discover it.
	pub fn open_rename_scenery(&mut self, pack: &str, id: &str, from: &str) {
		let name = TextInput::with_text(from).max_len(48);
		let error = Label::new("").small().with_id();
		let cancel = Button::new("Cancel").secondary();
		let save = Button::new("Save").primary();
		let ids = NamePromptIds { cancel: cancel.id(), save: save.id(), name: name.id(), error: error.id() };
		let content = column()
			.push(width_strut(340.0))
			.push(field_row("Name", name))
			.push(Label::new(format!("id     {pack} / {id}")).small().muted())
			.push(
				Label::new("The id is what placed objects refer to, and never changes.").small().muted().wrap_at(340.0),
			);
		let content = status_slot(content, error, 340.0, 2).push(buttons(cancel, save));
		let win = dialog("Rename Scenery", content);
		self.win_id = Some(win.id());
		self.ui = Ui::new(modal(win));
		self.ui.focus_first();
		self.dialog = Dialog::NamePrompt(ids);
		self.np.verb = "scenery-rename".to_string();
		self.np.from = from.to_string();
		self.events.clear();
		self.visible = true;
	}

	/// Opens the Delete Template confirm: a frozen thumbnail of the template,
	/// its name + footprint, and Cancel / Delete (danger, fires
	/// `template-delete!`).
	pub fn open_delete_template(
		&mut self,
		chrome: &mut MenuChrome,
		name: &str,
		footprint: (u16, u16),
		rgba: &[u8],
		tw: u32,
		th: u32,
	) {
		let preview = self.template_image(chrome, rgba, tw, th);
		let (fw, fh) = footprint;
		let content = column()
			.push(width_strut(340.0))
			.push(Linear::row().main_align(MainAlign::Center).push(preview))
			.push(Label::new(format!("{name}   ({fw}x{fh})")).small())
			.push(Label::new("Deleting a template cannot be undone.").small().muted());
		let delete = Button::new("Delete").danger();
		self.open_confirm_parts("Delete Template", content, delete, "Cancel", "template-delete!".into());
	}

	/// Opens the Save / Rename palette name dialog: a name field, an inline alert
	/// line, and Cancel / Save. `from` is the rename source `(name, file)` or
	/// `None` for Save; `existing` are the other user-palette names (the clash
	/// check). Confirm emits `palette-save-as` / `palette-rename` once the name
	/// validates (mirrors the bespoke `PaletteName`).
	pub fn open_palette_name(
		&mut self,
		title: &str,
		initial: &str,
		from: Option<(String, PathBuf)>,
		existing: Vec<String>,
	) {
		let name = TextInput::with_text(initial);
		let error = Label::new("").small().with_id();
		let cancel = Button::new("Cancel").secondary();
		let save = Button::new("Save").primary();
		let ids = PaletteNameIds { cancel: cancel.id(), save: save.id(), name: name.id(), error: error.id() };
		let content = column().push(width_strut(320.0)).push(field_row("Name", name));
		let content = status_slot(content, error, 320.0, 2).push(buttons(cancel, save));
		let win = dialog(title, content);
		self.win_id = Some(win.id());
		self.ui = Ui::new(modal(win));
		self.ui.focus_first();
		self.dialog = Dialog::PaletteName(ids);
		self.pn.from = from;
		self.pn.existing = existing;
		self.pn.armed = None;
		self.events.clear();
		self.visible = true;
	}

	/// Validate the typed palette name → the command line to run, or an alert to
	/// show (delegates to [`resolve_palette_name`], which owns the rules).
	pub(super) fn pn_resolve(&mut self, name: &str) -> Result<String, String> {
		resolve_palette_name(name, &self.pn.from, &self.pn.existing, &mut self.pn.armed)
	}

	/// A one-field editor for the resource brush's exact amount (S5.4): OK emits
	/// `resource-brush amount <value>`, bounded to 0-31.
	pub fn open_resource_amount(&mut self, initial: &str) {
		self.open_field_with_verb("resource-brush", "amount", "Amount", initial, Some(31));
	}

	/// Shared body of the one-field object editors: `verb` is the command the OK
	/// button runs against the field (`object-edit` / `object-values`).
	fn open_field_with_verb(&mut self, verb: &str, field: &str, label: &str, initial: &str, max: Option<u32>) {
		let value = match max {
			Some(_) => {
				TextInput::with_text(initial).charset(Charset::Digits).max_len(5).align(wgpu_ui::TextAlign::Right)
			}
			None => TextInput::with_text(initial).max_len(30),
		};
		let error = Label::new("").small().with_id();
		let cancel = Button::new("Cancel").secondary();
		let ok = Button::new("OK").primary();
		let ids = ObjectFieldIds { cancel: cancel.id(), ok: ok.id(), value: value.id(), error: error.id() };
		let content = column().push(width_strut(300.0)).push(field_row(label, value));
		let content = status_slot(content, error, 300.0, 2).push(buttons(cancel, ok));
		let win = dialog(&format!("Edit {label}"), content);
		self.win_id = Some(win.id());
		self.ui = Ui::new(modal(win));
		self.ui.focus_first();
		self.dialog = Dialog::ObjectField(ids);
		self.of.verb = verb.to_string();
		self.of.field = field.to_string();
		self.of.max = max;
		self.events.clear();
		self.visible = true;
	}

	/// Validate the [`Dialog::ObjectField`] value (OK fired) → the `object-edit`
	/// line to run, or [`Outcome::Idle`] with an inline alert set. A numeric field
	/// must parse within `of_max`; a name is quoted (spaces kept, embedded quotes
	/// stripped so the tokenizer sees one argument; empty clears the name).
	pub(super) fn object_field_confirm(&mut self, ids: ObjectFieldIds) -> Outcome {
		let raw = self.text(ids.value);
		let value = raw.trim();
		match self.of.max {
			Some(max) => match value.parse::<u32>() {
				Ok(v) if v <= max => Outcome::RunCommand(format!("{} {} {v}", self.of.verb, self.of.field)),
				_ => {
					self.set_label(ids.error, &format!("Enter a number 0-{max}."));
					Outcome::Idle
				}
			},
			None => Outcome::RunCommand(format!("{} {} \"{}\"", self.of.verb, self.of.field, value.replace('"', ""))),
		}
	}

	/// One frame of the dialog's control dispatch (the render match arm).
	pub(super) fn dispatch_palette_name(&mut self, ids: PaletteNameIds) -> Outcome {
		let mut outcome = Outcome::Idle;
		if self.ui.fired(ids.cancel) {
			self.hide();
		} else if self.ui.fired(ids.save) {
			let name = self.text(ids.name);
			match self.pn_resolve(&name) {
				Ok(line) => {
					outcome = Outcome::RunCommand(line);
					self.hide();
				}
				Err(msg) => self.set_label(ids.error, &msg),
			}
		}
		outcome
	}

	/// One frame of the dialog's control dispatch (the render match arm).
	pub(super) fn dispatch_rename_template(&mut self, ids: RenameTemplateIds) -> Outcome {
		let mut outcome = Outcome::Idle;
		if self.ui.fired(ids.cancel) {
			self.hide();
		} else if self.ui.fired(ids.save) {
			let name = self.text(ids.name);
			match resolve_template_rename(&name, &self.rt.from, &self.rt.existing) {
				Ok(line) => {
					outcome = Outcome::RunCommand(line);
					self.hide();
				}
				Err(msg) => self.set_label(ids.error, &msg),
			}
		}
		outcome
	}

	/// One frame of the dialog's control dispatch (the render match arm).
	pub(super) fn dispatch_name_prompt(&mut self, ids: NamePromptIds) -> Outcome {
		let mut outcome = Outcome::Idle;
		if self.ui.fired(ids.cancel) {
			self.hide();
		} else if self.ui.fired(ids.save) || self.ui.fired(ids.name) {
			// Save, or Enter in the field (the TextInput fires a commit).
			let name = self.text(ids.name).trim().to_string();
			if name.is_empty() {
				self.set_label(ids.error, "the name is empty");
			} else if name == self.np.from {
				self.hide(); // unchanged: nothing to write
			} else {
				outcome = Outcome::RunCommand(format!("{} \"{name}\"", self.np.verb));
				self.hide();
			}
		}
		outcome
	}

	/// One frame of the dialog's control dispatch (the render match arm).
	pub(super) fn dispatch_object_field(&mut self, ids: ObjectFieldIds) -> Outcome {
		let mut outcome = Outcome::Idle;
		if self.ui.fired(ids.cancel) {
			self.hide();
		} else if self.ui.fired(ids.ok) || self.ui.fired(ids.value) {
			// OK, or Enter in the field (the TextInput fires a commit).
			if let Outcome::RunCommand(line) = self.object_field_confirm(ids) {
				outcome = Outcome::RunCommand(line);
				self.hide();
			}
		}
		outcome
	}
}
