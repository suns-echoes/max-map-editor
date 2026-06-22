//! One routing surface for the text-field modals.
//!
//! `NewMap`, `Resize`, and friends are near-identical dialogs - a few
//! fields, a confirm/abort pair, a `view`. They used to be routed by
//! near-duplicate keyboard + mouse blocks in `main.rs`; this trait collapses
//! them to one. (File open/save went native - `rfd` - and left this club.) The shell decodes a winit key into a [`ModalKey`], calls the
//! one open modal (`EditorState::active_modal`), and acts on the returned
//! [`ModalAction`]. (The Auto Fix Shore modal stays separate - it owns a live,
//! stepped run rather than a command line.)

use crate::about::{self, About};
use crate::autofix::{self, AutoFix};
use crate::confirm::{self, ConfirmClose};
use crate::convertpalette::{self, ConvertPalette};
use crate::dedupetemplates::{self, DedupeTemplates};
use crate::deletetemplate::{self, DeleteTemplate};
use crate::errormodal::{self, ErrorModal};
use crate::generator::{self, Generator};
use crate::importwrl::{self, ImportWrl};
use crate::newfromimage::{self, NewFromImage};
use crate::newmap::{self, NewMap};
use crate::palettedelete::{self, PaletteDelete};
use crate::palettename::{self, PaletteName};
use crate::renametemplate::{self, RenameTemplate};
use crate::resize::{self, Resize};
use crate::tilepainter::{self, TilePainter};
use crate::ui::{self, Rect};

/// A keystroke a modal cares about, decoded from the winit event by the shell.
pub enum ModalKey {
	Enter,
	Escape,
	Backspace,
	Tab,
	Char(char),
	// Text-editing keys (the enhanced text fields); `shift` extends the
	// selection. Modals without editable text ignore these.
	Left { shift: bool },
	Right { shift: bool },
	Home { shift: bool },
	End { shift: bool },
	Delete,
	Copy,
	Cut,
	Paste,
	SelectAll,
}

/// What the shell should do after a modal handles an event.
pub enum ModalAction {
	/// Event handled; the modal stays open (redraw).
	Consumed,
	/// Close the modal (Esc / Abort / click-out).
	Close,
	/// Close the modal, then parse + run this command line.
	Run(String),
	/// Push this message to the console; the modal stays open.
	Error(String),
	/// Begin the Auto Fix Shore live run - a stepped, abortable job,
	/// not a command line; the shell drives it per frame and stamps the clock.
	StartFix,
	/// Stop/finish the Auto Fix Shore run, committing what it found so far.
	/// The modal stays open to show the result.
	StopFix,
	/// Begin the New-from-Image conversion - a stepped, abortable job the
	/// shell drives per frame; on completion it opens the result as a new tab.
	StartConvert,
	/// Abort the running New-from-Image conversion (back to its settings).
	AbortConvert,
	/// Begin a terrain generation run - a stepped, abortable job the
	/// shell drives per frame; the modal stays open showing progress, and for
	/// the next reroll once done.
	StartGenerate,
	/// Abort the running generation, rolling the document back.
	AbortGenerate,
	/// Begin the rasterize palette conversion - a stepped, abortable job the
	/// shell drives per frame; on completion the document content swaps (one
	/// undo unit) and the modal closes.
	StartPaletteConvert,
	/// Abort the running palette conversion (back to the options).
	AbortPaletteConvert,
	/// Commit the Map Preferences fields to the document, then close.
	SavePreferences,
	/// Commit the Tile Painter's canvas to its pack (a new/cloned tile, or an
	/// in-place edit), then close - the shell reads the painted bytes since a
	/// command line can't carry them.
	CommitTile,
	/// Copy the open Tile Painter's canvas (raw indices) to the tile clipboard.
	CopyTile,
	/// Paste the tile clipboard over the open Tile Painter's canvas.
	PasteTile,
	/// Open a save dialog and export the painter's tile to a PNG.
	ExportTilePng,
	/// Open an image dialog and import a PNG into the painter (nearest match).
	ImportTilePng,
	/// Run the open Import WRL modal's match against its selected packs; on a
	/// clean match the shell opens the result, else it shows the unmapped list.
	StartWrlMatch,
	/// Commit the open Import WRL modal with its chosen extras destination,
	/// opening the converted map as a new tab.
	FinishWrlImport,
}

/// What a modal's currently-focused text field reports, so the shell can build
/// its right-click edit menu (which items to offer) and only open it when a
/// field actually has focus.
pub struct EditContext {
	pub has_selection: bool,
	pub is_empty: bool,
}

pub trait Modal {
	fn on_key(&mut self, key: ModalKey) -> ModalAction;
	/// Like [`Modal::on_key`], but handed the viewport size so a modal can scroll
	/// a focused multiline field's caret into view (only Preferences' description
	/// needs this today). The shell calls this; the default ignores the size.
	fn on_key_at(&mut self, key: ModalKey, _w: f32, _h: f32) -> ModalAction {
		self.on_key(key)
	}
	/// The focused editable field's state, when this modal has a text field
	/// focused - drives the right-click Cut/Copy/Paste/Select-All menu. Default:
	/// the modal has no focused text field (no edit menu).
	fn edit_context(&self) -> Option<EditContext> {
		None
	}
	fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction;
	/// The matching release - command buttons arm on press and fire here only
	/// when the release is still inside them, so a
	/// mis-click can be cancelled by dragging off before letting go.
	fn on_release(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) -> ModalAction {
		ModalAction::Consumed
	}
	/// Wheel over the modal. Default: swallow it (a modal blocks the map).
	fn on_wheel(&mut self, _steps: f32, _w: f32, _h: f32) -> ModalAction {
		ModalAction::Consumed
	}
	/// Cursor drag while a press is held — used by editable text fields to
	/// extend a mouse selection. Default: ignore.
	fn on_drag(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) {}
	/// The draggable titlebar band - the inset title strip of the dialog.
	fn titlebar(&self, w: f32, h: f32) -> Rect;
	/// Nudge the dialog by `(dx, dy)` while its titlebar is dragged.
	fn drag(&mut self, dx: f32, dy: f32);
	/// Downcast hooks - the open modal is stored as a single `Box<dyn Modal>`,
	/// so the render block and the stepped-run drivers (`*_tick`) recover the
	/// concrete modal. All three are generated by `modal_glue!`.
	fn as_any(&self) -> &dyn std::any::Any;
	fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
	fn into_any(self: Box<Self>) -> Box<dyn std::any::Any>;
}

/// The shared `Modal` glue pasted into each impl: the `titlebar` + `drag`
/// bodies (every modal centers via `dialog_rect` and carries a `drag_offset`,
/// so the titlebar-drag chrome is identical) plus the `as_any`/`into_any`
/// downcast hooks (every concrete modal is `'static`, so each is just `self`).
macro_rules! modal_glue {
	() => {
		fn titlebar(&self, w: f32, h: f32) -> Rect {
			ui::titlebar_band(self.dialog_rect(w, h), ui::MODAL_FRAME)
		}
		fn drag(&mut self, dx: f32, dy: f32) {
			self.drag_offset.0 += dx;
			self.drag_offset.1 += dy;
		}
		fn as_any(&self) -> &dyn std::any::Any {
			self
		}
		fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
			self
		}
		fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
			self
		}
	};
}

/// Standard text-modal input glue: `edit_context` (the focused field's edit
/// menu) + `on_press`/`on_release` (buttons armed on press, fired on release,
/// mapped through `$from`) + `on_drag` (a held drag extends the field selection).
macro_rules! text_modal_glue {
	($from:path) => {
		fn edit_context(&self) -> Option<EditContext> {
			self.edit_context()
		}
		fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
			$from(self.on_press(x, y, w, h))
		}
		fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
			$from(self.on_release(x, y, w, h))
		}
		fn on_drag(&mut self, x: f32, y: f32, w: f32, h: f32) {
			self.on_drag(x, y, w, h);
		}
	};
}

/// Button-only press glue (no text field): arm on press, fire on release, both
/// mapped through `$from`. For the plain confirm / acknowledge modals.
macro_rules! modal_press {
	($from:path) => {
		fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
			$from(self.on_press(x, y, w, h))
		}
		fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
			$from(self.on_release(x, y, w, h))
		}
	};
}

impl Modal for NewMap {
	fn on_key(&mut self, key: ModalKey) -> ModalAction {
		match key {
			ModalKey::Enter => match self.create_command() {
				Ok(line) => ModalAction::Run(line),
				Err(e) => ModalAction::Error(format!("new map: {e}")),
			},
			// Esc backs out of the pack picker first, then closes.
			ModalKey::Escape => {
				if self.picking {
					self.picking = false;
					ModalAction::Consumed
				} else {
					ModalAction::Close
				}
			}
			ModalKey::Tab => {
				self.focus_next();
				ModalAction::Consumed
			}
			// Everything else edits the focused W/H field.
			other => {
				self.key(&other);
				ModalAction::Consumed
			}
		}
	}

	text_modal_glue!(from_newmap);

	modal_glue!();
}

fn from_newmap(press: newmap::Press) -> ModalAction {
	match press {
		newmap::Press::Consumed => ModalAction::Consumed,
		newmap::Press::Abort => ModalAction::Close,
		newmap::Press::Create(line) => ModalAction::Run(line),
		newmap::Press::Invalid(e) => ModalAction::Error(format!("new map: {e}")),
	}
}

impl Modal for Resize {
	fn on_key(&mut self, key: ModalKey) -> ModalAction {
		match key {
			ModalKey::Enter => from_resize(self.confirm()),
			ModalKey::Escape => ModalAction::Close,
			ModalKey::Tab => {
				self.focus_next();
				ModalAction::Consumed
			}
			// Everything else edits the focused W/H field.
			other => {
				self.key(&other);
				ModalAction::Consumed
			}
		}
	}

	text_modal_glue!(from_resize);

	modal_glue!();
}

impl Modal for AutoFix {
	fn on_key(&mut self, key: ModalKey) -> ModalAction {
		match key {
			// Esc stops a live run, else closes - the only key it cares about.
			ModalKey::Escape if self.running => ModalAction::StopFix,
			ModalKey::Escape => ModalAction::Close,
			_ => ModalAction::Consumed,
		}
	}

	fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
		match self.on_press(x, y, w, h) {
			// Mode selection is internal to the modal.
			autofix::Press::SetMode(m) => {
				self.mode = m;
				ModalAction::Consumed
			}
			press => from_autofix(press),
		}
	}

	fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
		from_autofix(self.on_release(x, y, w, h))
	}

	modal_glue!();
}

fn from_autofix(press: autofix::Press) -> ModalAction {
	match press {
		autofix::Press::Consumed | autofix::Press::SetMode(_) => ModalAction::Consumed,
		autofix::Press::Close => ModalAction::Close,
		autofix::Press::Start => ModalAction::StartFix,
		autofix::Press::Stop => ModalAction::StopFix,
	}
}

impl Modal for Generator {
	fn on_key(&mut self, key: ModalKey) -> ModalAction {
		match key {
			// Enter generates (when idle); Esc aborts a run, else closes.
			ModalKey::Enter if !self.running => ModalAction::StartGenerate,
			ModalKey::Enter => ModalAction::Consumed,
			ModalKey::Escape if self.running => ModalAction::AbortGenerate,
			ModalKey::Escape => ModalAction::Close,
			_ if self.running => ModalAction::Consumed,
			ModalKey::Tab => {
				self.focus_next();
				ModalAction::Consumed
			}
			// Everything else edits the focused field.
			other => {
				self.key(&other);
				ModalAction::Consumed
			}
		}
	}

	text_modal_glue!(from_generator);

	modal_glue!();
}

fn from_generator(press: generator::Press) -> ModalAction {
	match press {
		generator::Press::Consumed => ModalAction::Consumed,
		generator::Press::Close => ModalAction::Close,
		generator::Press::Start => ModalAction::StartGenerate,
		generator::Press::Abort => ModalAction::AbortGenerate,
		generator::Press::Invalid(e) => ModalAction::Error(format!("generate: {e}")),
	}
}

impl Modal for NewFromImage {
	fn on_key(&mut self, key: ModalKey) -> ModalAction {
		match key {
			// Enter converts (when idle); Esc aborts a run, else closes.
			ModalKey::Enter if !self.running => ModalAction::StartConvert,
			ModalKey::Enter => ModalAction::Consumed,
			ModalKey::Escape if self.running => ModalAction::AbortConvert,
			ModalKey::Escape => ModalAction::Close,
			_ if self.running => ModalAction::Consumed,
			ModalKey::Tab => {
				self.focus_next();
				ModalAction::Consumed
			}
			// Everything else edits the focused field.
			other => {
				self.key(&other);
				ModalAction::Consumed
			}
		}
	}

	fn edit_context(&self) -> Option<EditContext> {
		self.edit_context()
	}

	fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
		match self.on_press(x, y, w, h) {
			newfromimage::Press::SetCoverage(c) => {
				self.coverage = c;
				ModalAction::Consumed
			}
			newfromimage::Press::SetDedupe(d) => {
				self.dedupe = d;
				ModalAction::Consumed
			}
			press => from_newfromimage(press),
		}
	}

	fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
		from_newfromimage(self.on_release(x, y, w, h))
	}

	fn on_drag(&mut self, x: f32, y: f32, w: f32, h: f32) {
		self.on_drag(x, y, w, h);
	}

	modal_glue!();
}

impl Modal for ImportWrl {
	fn on_key(&mut self, key: ModalKey) -> ModalAction {
		match key {
			ModalKey::Enter => from_importwrl(self.confirm_key()),
			// Esc backs out of the unmapped review to the picker first, then closes.
			ModalKey::Escape => {
				if self.back() {
					ModalAction::Consumed
				} else {
					ModalAction::Close
				}
			}
			_ => ModalAction::Consumed,
		}
	}

	modal_press!(from_importwrl);

	fn on_wheel(&mut self, steps: f32, _w: f32, _h: f32) -> ModalAction {
		self.scroll_by(steps);
		ModalAction::Consumed
	}

	modal_glue!();
}

fn from_importwrl(press: importwrl::Press) -> ModalAction {
	match press {
		importwrl::Press::Consumed => ModalAction::Consumed,
		importwrl::Press::Cancel => ModalAction::Close,
		importwrl::Press::Match => ModalAction::StartWrlMatch,
		importwrl::Press::Finish => ModalAction::FinishWrlImport,
	}
}

fn from_newfromimage(press: newfromimage::Press) -> ModalAction {
	match press {
		newfromimage::Press::Consumed | newfromimage::Press::SetCoverage(_) | newfromimage::Press::SetDedupe(_) => {
			ModalAction::Consumed
		}
		newfromimage::Press::Cancel => ModalAction::Close,
		newfromimage::Press::Convert => ModalAction::StartConvert,
		newfromimage::Press::Abort => ModalAction::AbortConvert,
	}
}

impl Modal for About {
	fn on_key(&mut self, key: ModalKey) -> ModalAction {
		match key {
			// Any acknowledgement closes it; it carries no input.
			ModalKey::Enter | ModalKey::Escape => ModalAction::Close,
			_ => ModalAction::Consumed,
		}
	}

	modal_press!(from_about);

	modal_glue!();
}

fn from_about(press: about::Press) -> ModalAction {
	match press {
		about::Press::Consumed => ModalAction::Consumed,
		about::Press::Close => ModalAction::Close,
		// The links open in the browser via the same command as the Help menu.
		about::Press::Website => ModalAction::Run(format!("open-url {}", about::WEBSITE)),
		about::Press::GitHub => ModalAction::Run(format!("open-url {}", about::GITHUB)),
	}
}

impl Modal for ErrorModal {
	fn on_key(&mut self, key: ModalKey) -> ModalAction {
		match key {
			// Any acknowledgement closes it - it carries no input.
			ModalKey::Enter | ModalKey::Escape => ModalAction::Close,
			_ => ModalAction::Consumed,
		}
	}

	modal_press!(from_error);

	modal_glue!();
}

fn from_error(press: errormodal::Press) -> ModalAction {
	match press {
		errormodal::Press::Consumed => ModalAction::Consumed,
		errormodal::Press::Dismiss => ModalAction::Close,
	}
}

impl Modal for ConfirmClose {
	fn on_key(&mut self, key: ModalKey) -> ModalAction {
		match key {
			ModalKey::Enter => ModalAction::Run(self.save_line().into()),
			ModalKey::Escape => ModalAction::Close,
			_ => ModalAction::Consumed,
		}
	}

	fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
		let press = self.on_press(x, y, w, h);
		self.confirm_action(press)
	}

	fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
		let press = self.on_release(x, y, w, h);
		self.confirm_action(press)
	}

	modal_glue!();
}

impl ConfirmClose {
	/// Map a button press to its action, using the confirm's purpose for the
	/// Discard/Save command lines (close-a-tab vs quit-the-editor).
	fn confirm_action(&self, press: confirm::Press) -> ModalAction {
		match press {
			confirm::Press::Consumed => ModalAction::Consumed,
			confirm::Press::Cancel => ModalAction::Close,
			confirm::Press::Discard => ModalAction::Run(self.discard_line().into()),
			confirm::Press::Save => ModalAction::Run(self.save_line().into()),
		}
	}
}

impl Modal for ConvertPalette {
	fn on_key(&mut self, key: ModalKey) -> ModalAction {
		match key {
			// Enter converts (when idle); Esc aborts a run, else closes.
			ModalKey::Enter if !self.running => from_convertpalette(self.confirm()),
			ModalKey::Enter => ModalAction::Consumed,
			ModalKey::Escape if self.running => ModalAction::AbortPaletteConvert,
			ModalKey::Escape => ModalAction::Close,
			// Everything else edits the threshold field (when focused).
			other => {
				self.key(&other);
				ModalAction::Consumed
			}
		}
	}

	text_modal_glue!(from_convertpalette);

	modal_glue!();
}

fn from_convertpalette(press: convertpalette::Press) -> ModalAction {
	match press {
		convertpalette::Press::Consumed => ModalAction::Consumed,
		convertpalette::Press::Close => ModalAction::Close,
		convertpalette::Press::Convert(line) => ModalAction::Run(line),
		convertpalette::Press::StartRasterize => ModalAction::StartPaletteConvert,
		convertpalette::Press::Abort => ModalAction::AbortPaletteConvert,
		convertpalette::Press::Invalid(e) => ModalAction::Error(format!("convert-palette: {e}")),
	}
}

impl Modal for crate::preferences::Preferences {
	fn on_key(&mut self, key: ModalKey) -> ModalAction {
		match key {
			ModalKey::Enter => ModalAction::SavePreferences,
			ModalKey::Escape => ModalAction::Close,
			ModalKey::Tab => {
				self.focus_next();
				ModalAction::Consumed
			}
			// Everything else edits the focused text field.
			other => {
				self.key_focused(&other);
				ModalAction::Consumed
			}
		}
	}

	fn on_key_at(&mut self, key: ModalKey, w: f32, h: f32) -> ModalAction {
		match key {
			// In the multiline description, Enter inserts a newline; elsewhere it
			// saves. Editing keys scroll the focused field's caret into view.
			ModalKey::Enter if self.focused_wants_newline() => {
				self.newline_focused(w, h);
				ModalAction::Consumed
			}
			ModalKey::Enter => ModalAction::SavePreferences,
			ModalKey::Escape => ModalAction::Close,
			ModalKey::Tab => {
				self.focus_next();
				ModalAction::Consumed
			}
			other => {
				self.key_focused_at(&other, w, h);
				ModalAction::Consumed
			}
		}
	}

	fn edit_context(&self) -> Option<EditContext> {
		self.edit_context()
	}

	fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
		from_preferences(self.on_press(x, y, w, h))
	}

	fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
		from_preferences(self.on_release(x, y, w, h))
	}

	fn on_drag(&mut self, x: f32, y: f32, w: f32, h: f32) {
		self.drag_select(x, y, w, h);
	}

	fn on_wheel(&mut self, steps: f32, w: f32, h: f32) -> ModalAction {
		self.wheel_focused(steps, w, h);
		ModalAction::Consumed
	}

	modal_glue!();
}

fn from_preferences(press: crate::preferences::Press) -> ModalAction {
	match press {
		crate::preferences::Press::Consumed => ModalAction::Consumed,
		crate::preferences::Press::Close => ModalAction::Close,
		crate::preferences::Press::Save => ModalAction::SavePreferences,
	}
}

fn from_resize(press: resize::Press) -> ModalAction {
	match press {
		resize::Press::Consumed => ModalAction::Consumed,
		resize::Press::Abort => ModalAction::Close,
		resize::Press::Resize(line) => ModalAction::Run(line),
		resize::Press::Invalid(e) => ModalAction::Error(format!("resize: {e}")),
	}
}

impl Modal for RenameTemplate {
	fn on_key(&mut self, key: ModalKey) -> ModalAction {
		match key {
			// On a bad/duplicate name the alert is shown in-modal; stay open.
			ModalKey::Enter => match self.try_confirm() {
				Some(line) => ModalAction::Run(line),
				None => ModalAction::Consumed,
			},
			ModalKey::Escape => ModalAction::Close,
			// Everything else edits the (always-focused) name field.
			other => {
				self.key(&other);
				ModalAction::Consumed
			}
		}
	}

	text_modal_glue!(from_renametemplate);

	modal_glue!();
}

fn from_renametemplate(press: renametemplate::Press) -> ModalAction {
	match press {
		renametemplate::Press::Consumed => ModalAction::Consumed,
		renametemplate::Press::Cancel => ModalAction::Close,
		renametemplate::Press::Rename(line) => ModalAction::Run(line),
	}
}

impl Modal for DeleteTemplate {
	fn on_key(&mut self, key: ModalKey) -> ModalAction {
		match key {
			ModalKey::Enter => ModalAction::Run("template-delete!".into()),
			ModalKey::Escape => ModalAction::Close,
			_ => ModalAction::Consumed,
		}
	}

	modal_press!(from_deletetemplate);

	modal_glue!();
}

fn from_deletetemplate(press: deletetemplate::Press) -> ModalAction {
	match press {
		deletetemplate::Press::Consumed => ModalAction::Consumed,
		deletetemplate::Press::Cancel => ModalAction::Close,
		deletetemplate::Press::Delete => ModalAction::Run("template-delete!".into()),
	}
}

impl Modal for DedupeTemplates {
	fn on_key(&mut self, key: ModalKey) -> ModalAction {
		match key {
			// Enter removes when there are dupes, else just acknowledges.
			ModalKey::Enter => from_dedupetemplates(dedupetemplates::Press::Remove),
			ModalKey::Escape => ModalAction::Close,
			_ => ModalAction::Consumed,
		}
	}

	modal_press!(from_dedupetemplates);

	fn on_wheel(&mut self, steps: f32, _w: f32, _h: f32) -> ModalAction {
		self.scroll_by(steps);
		ModalAction::Consumed
	}

	modal_glue!();
}

fn from_dedupetemplates(press: dedupetemplates::Press) -> ModalAction {
	match press {
		dedupetemplates::Press::Consumed => ModalAction::Consumed,
		dedupetemplates::Press::Cancel => ModalAction::Close,
		// The shell ignores the command harmlessly when there are no dupes.
		dedupetemplates::Press::Remove => ModalAction::Run("template-dedupe!".into()),
	}
}

impl Modal for TilePainter {
	fn on_key(&mut self, key: ModalKey) -> ModalAction {
		match key {
			ModalKey::Enter => ModalAction::CommitTile,
			ModalKey::Escape => ModalAction::Close,
			// Everything else edits the id field when it's focused (else ignored).
			other => {
				self.key(&other);
				ModalAction::Consumed
			}
		}
	}

	text_modal_glue!(from_tilepainter);

	fn on_wheel(&mut self, steps: f32, _w: f32, _h: f32) -> ModalAction {
		self.on_wheel(steps);
		ModalAction::Consumed
	}

	modal_glue!();
}

fn from_tilepainter(press: tilepainter::Press) -> ModalAction {
	match press {
		tilepainter::Press::Consumed => ModalAction::Consumed,
		tilepainter::Press::Cancel => ModalAction::Close,
		tilepainter::Press::Save => ModalAction::CommitTile,
		tilepainter::Press::Copy => ModalAction::CopyTile,
		tilepainter::Press::Paste => ModalAction::PasteTile,
		tilepainter::Press::ExportPng => ModalAction::ExportTilePng,
		tilepainter::Press::ImportPng => ModalAction::ImportTilePng,
	}
}

impl Modal for PaletteName {
	fn on_key(&mut self, key: ModalKey) -> ModalAction {
		match key {
			// On a bad/duplicate name the alert is shown in-modal; stay open.
			ModalKey::Enter => from_palettename(self.confirm()),
			ModalKey::Escape => ModalAction::Close,
			// Everything else edits the (always-focused) name field.
			other => {
				self.key(&other);
				ModalAction::Consumed
			}
		}
	}

	text_modal_glue!(from_palettename);

	modal_glue!();
}

fn from_palettename(press: palettename::Press) -> ModalAction {
	match press {
		palettename::Press::Consumed => ModalAction::Consumed,
		palettename::Press::Cancel => ModalAction::Close,
		palettename::Press::Run(line) => ModalAction::Run(line),
	}
}

impl Modal for PaletteDelete {
	fn on_key(&mut self, key: ModalKey) -> ModalAction {
		match key {
			ModalKey::Enter => ModalAction::Run(self.command()),
			ModalKey::Escape => ModalAction::Close,
			_ => ModalAction::Consumed,
		}
	}

	fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
		let press = self.on_press(x, y, w, h);
		self.action_for(press)
	}

	fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
		let press = self.on_release(x, y, w, h);
		self.action_for(press)
	}

	modal_glue!();
}

impl PaletteDelete {
	/// Map a button press to its action, reading the path for the Delete command.
	fn action_for(&self, press: palettedelete::Press) -> ModalAction {
		match press {
			palettedelete::Press::Consumed => ModalAction::Consumed,
			palettedelete::Press::Cancel => ModalAction::Close,
			palettedelete::Press::Delete => ModalAction::Run(self.command()),
		}
	}
}
