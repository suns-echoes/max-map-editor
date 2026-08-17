//! The Confirm family: the generic confirm dialog (plain, labeled, warned,
//! save-with-preview), the error and notice boxes, and Remove Duplicates.

use super::*;

#[derive(Clone, Copy)]
pub(super) struct ConfirmIds {
	pub(super) cancel: WidgetId,
	pub(super) confirm: WidgetId,
	/// The middle "proceed without saving" button of the three-way guard
	/// (`None` for the plain two-button confirm).
	pub(super) discard: Option<WidgetId>,
}

/// Confirm dialog state: the command line the primary fires
/// ([`Outcome::RunCommand`]) and the one the three-way guard's Discard
/// button runs. Reset by [`Overlay::hide`].
#[derive(Default)]
pub(super) struct ConfirmState {
	pub(super) cmd: String,
	pub(super) discard_cmd: String,
}

impl Overlay {
	/// Opens a generic confirm dialog: a `prompt`, an optional dim `note`, and
	/// Cancel / `confirm_label` buttons. Firing the primary returns `command` as
	/// [`Outcome::RunCommand`] - so a confirm-style modal migrates by its trigger
	/// and the command line it emits, nothing more.
	pub fn open_confirm(&mut self, title: &str, prompt: &str, note: &str, confirm_label: &str, command: String) {
		let mut col = column().push(width_strut(300.0)).push(Label::new(prompt.to_string()));
		if !note.is_empty() {
			col = col.push(Label::new(note.to_string()).small());
		}
		let confirm = Button::new(confirm_label).primary();
		self.open_confirm_parts(title, col, confirm, "Cancel", command);
	}

	/// A confirm dialog with a custom secondary (cancel) label — e.g. **Abort** /
	/// **Open Anyway** for the save-open confirm. The `message` word-wraps like a
	/// notice; firing the primary runs `command` via [`Outcome::RunCommand`].
	pub fn open_confirm_labeled(
		&mut self,
		title: &str,
		message: &str,
		cancel_label: &str,
		confirm_label: &str,
		command: String,
	) {
		let col = column().push(width_strut(430.0)).push(Label::new(message.to_string()).small().wrap_at(430.0));
		let confirm = Button::new(confirm_label).primary().focusable();
		self.open_confirm_parts(title, col, confirm, cancel_label, command);
	}

	/// Like [`Self::open_confirm_labeled`], but with a red **warning** line below the
	/// message — the experimental-save gate ("don't report game bugs on modified
	/// saves"). Same Cancel / confirm behaviour.
	pub fn open_confirm_warned(
		&mut self,
		title: &str,
		message: &str,
		warning: &str,
		cancel_label: &str,
		confirm_label: &str,
		command: String,
	) {
		// The warning sits in a dark-red inset (8px padding) with an 8px margin
		// around it (the wrapping column's padding), so the alert reads as its own
		// block. The label wraps inside the inset's interior width.
		let inset = Well::new(Label::new(warning.to_string()).small().wrap_at(394.0).color(WARNING_INK))
			.padding(8.0)
			.wash(WARNING_GROUND);
		let warn_block = Linear::column().padding(Insets::all(8.0)).cross_align(CrossAlign::Stretch).push(inset);
		let col = column()
			.push(width_strut(430.0))
			.push(Label::new(message.to_string()).small().wrap_at(430.0))
			.push(warn_block);
		let confirm = Button::new(confirm_label).primary().focusable();
		self.open_confirm_parts(title, col, confirm, cancel_label, command);
	}

	/// The confirm-dialog core: arbitrary `content` rows above a `cancel_label` /
	/// `confirm` row, with `command` run when the confirm fires — the
	/// list-bearing confirms (duplicate templates) build their body and route
	/// through here.
	pub(super) fn open_confirm_parts(
		&mut self,
		title: &str,
		content: Linear,
		confirm: Button,
		cancel_label: &str,
		command: String,
	) {
		let cancel = Button::new(cancel_label).secondary();
		let ids = ConfirmIds { cancel: cancel.id(), confirm: confirm.id(), discard: None };
		let content = content.push(buttons(cancel, confirm));
		let win = dialog(title, content);
		self.win_id = Some(win.id());
		self.ui = Ui::new(modal(win));
		self.ui.focus_first();
		self.dialog = Dialog::Confirm(ids);
		self.confirm.cmd = command;
		self.events.clear();
		self.visible = true;
	}

	/// Opens the Remove Duplicate Templates confirm: the duplicate names in a
	/// scrolling well + Cancel / Remove (danger, fires `template-dedupe!`).
	/// With nothing duplicated it is just an acknowledgement.
	pub fn open_dedupe(&mut self, names: &[String]) {
		if names.is_empty() {
			self.open_notice("Remove Duplicate Templates", "Close", "No duplicate templates found.");
			return;
		}
		let n = names.len();
		let heading = format!("Found {n} exact-duplicate template{}:", if n == 1 { "" } else { "s" });
		let mut rows = Linear::column().spacing(2.0).cross_align(CrossAlign::Stretch);
		for name in names {
			rows = rows.push(Label::new(name.clone()).small().muted());
		}
		// Cap the well at ~8 rows; more scroll into view.
		let cap = 8.0 * 17.0;
		let col = column()
			.push(width_strut(340.0))
			.push(Label::new(heading).small())
			.child(Well::new(ScrollArea::new(rows)), Length::Fixed(cap));
		let remove = Button::new("Remove").danger();
		self.open_confirm_parts("Remove Duplicate Templates", col, remove, "Cancel", "template-dedupe!".into());
	}

	/// Opens the error dialog: a word-wrapped message + OK. Dismiss-only (OK /
	/// Enter / Escape); the caller mirrors the message to the console for the
	/// scrollback.
	pub fn open_error(&mut self, message: &str) {
		self.open_notice("Error", "OK", message);
	}

	/// Opens a dismiss-only acknowledgement: a word-wrapped message and one
	/// `button` (Enter/Escape dismiss too).
	pub fn open_notice(&mut self, title: &str, button: &str, message: &str) {
		let ok = Button::new(button).primary().focusable();
		let ids = ok.id();
		let row = Linear::row().main_align(MainAlign::End).push(ok);
		// wrap_at pins the wrap width: a long message grows the dialog DOWN
		// (more lines at 430), never wider than the strut.
		let content =
			column().push(width_strut(430.0)).push(Label::new(message.to_string()).small().wrap_at(430.0)).push(row);
		let win = dialog(title, content);
		self.win_id = Some(win.id());
		self.ui = Ui::new(modal(win));
		self.ui.focus_first();
		self.dialog = Dialog::Error(ids);
		self.events.clear();
		self.visible = true;
	}

	/// Opens the three-way unsaved-changes guard: Cancel / `discard_label`
	/// (proceed without saving) / `save_label` (primary). Each of
	/// `save_cmd`/`discard_cmd` runs via [`Outcome::RunCommand`] when its button
	/// fires. Save is keyboard-focused so Enter saves; Escape cancels (as for
	/// every overlay dialog) — the legacy modal's key map.
	pub fn open_confirm_save(
		&mut self,
		title: &str,
		prompt: &str,
		save_label: &str,
		save_cmd: String,
		discard_label: &str,
		discard_cmd: String,
	) {
		let cancel = Button::new("Cancel").secondary();
		let discard = Button::new(discard_label);
		let save = Button::new(save_label).primary().focusable();
		let ids = ConfirmIds { cancel: cancel.id(), confirm: save.id(), discard: Some(discard.id()) };
		let row = Linear::row().spacing(8.0).main_align(MainAlign::End).push(cancel).push(discard).push(save);
		let content = column().push(width_strut(340.0)).push(Label::new(prompt.to_string())).push(row);
		let win = dialog(title, content);
		self.win_id = Some(win.id());
		self.ui = Ui::new(modal(win));
		self.ui.focus_first();
		self.dialog = Dialog::Confirm(ids);
		self.confirm.cmd = save_cmd;
		self.confirm.discard_cmd = discard_cmd;
		self.events.clear();
		self.visible = true;
	}

	/// One frame of the dialog's control dispatch (the render match arm).
	pub(super) fn dispatch_confirm(&mut self, ids: ConfirmIds) -> Outcome {
		let mut outcome = Outcome::Idle;
		if self.ui.fired(ids.cancel) {
			self.hide();
		} else if self.ui.fired(ids.confirm) {
			outcome = Outcome::RunCommand(self.confirm.cmd.clone());
			self.hide();
		} else if ids.discard.is_some_and(|d| self.ui.fired(d)) {
			outcome = Outcome::RunCommand(self.confirm.discard_cmd.clone());
			self.hide();
		}
		outcome
	}
}
