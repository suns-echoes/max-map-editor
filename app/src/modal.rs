//! One routing surface for the text-field modals.
//!
//! `NewMap`, `Resize`, and friends are near-identical dialogs — a few
//! fields, a confirm/abort pair, a `view`. They used to be routed by
//! near-duplicate keyboard + mouse blocks in `main.rs`; this trait collapses
//! them to one. (File open/save went native — `rfd` — and left this club.) The shell decodes a winit key into a [`ModalKey`], calls the
//! one open modal (`EditorState::active_modal`), and acts on the returned
//! [`ModalAction`]. (The Auto Fix Shore modal stays separate — it owns a live,
//! stepped run rather than a command line.)

use crate::autofix::{self, AutoFix};
use crate::confirm::{self, ConfirmClose};
use crate::convertpalette::{self, ConvertPalette};
use crate::errormodal::{self, ErrorModal};
use crate::generator::{self, Generator};
use crate::newfromimage::{self, NewFromImage};
use crate::newmap::{self, NewMap};
use crate::resize::{self, Resize};
use crate::ui::{self, Rect};

/// A keystroke a modal cares about, decoded from the winit event by the shell.
pub enum ModalKey {
	Enter,
	Escape,
	Backspace,
	Tab,
	Char(char),
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
	/// Begin the Auto Fix Shore live run — a stepped, abortable job,
	/// not a command line; the shell drives it per frame and stamps the clock.
	StartFix,
	/// Stop/finish the Auto Fix Shore run, committing what it found so far.
	/// The modal stays open to show the result.
	StopFix,
	/// Begin the New-from-Image conversion — a stepped, abortable job the
	/// shell drives per frame; on completion it opens the result as a new tab.
	StartConvert,
	/// Abort the running New-from-Image conversion (back to its settings).
	AbortConvert,
	/// Begin a terrain generation run — a stepped, abortable job the
	/// shell drives per frame; the modal stays open showing progress, and for
	/// the next reroll once done.
	StartGenerate,
	/// Abort the running generation, rolling the document back.
	AbortGenerate,
	/// Begin the rasterize palette conversion — a stepped, abortable job the
	/// shell drives per frame; on completion the document content swaps (one
	/// undo unit) and the modal closes.
	StartPaletteConvert,
	/// Abort the running palette conversion (back to the options).
	AbortPaletteConvert,
}

pub trait Modal {
	fn on_key(&mut self, key: ModalKey) -> ModalAction;
	fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction;
	/// The matching release — command buttons arm on press and fire here only
	/// when the release is still inside them, so a
	/// mis-click can be cancelled by dragging off before letting go.
	fn on_release(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) -> ModalAction {
		ModalAction::Consumed
	}
	/// Wheel over the modal. Default: swallow it (a modal blocks the map).
	fn on_wheel(&mut self, _steps: f32) -> ModalAction {
		ModalAction::Consumed
	}
	/// The draggable titlebar band — the inset title strip of the dialog.
	fn titlebar(&self, w: f32, h: f32) -> Rect;
	/// Nudge the dialog by `(dx, dy)` while its titlebar is dragged.
	fn drag(&mut self, dx: f32, dy: f32);
}

/// The shared `Modal::titlebar` + `Modal::drag` bodies, pasted into each impl —
/// every modal centers via `dialog_rect` and carries a `drag_offset`, so the
/// titlebar-drag chrome is identical.
macro_rules! modal_drag {
	() => {
		fn titlebar(&self, w: f32, h: f32) -> Rect {
			ui::titlebar_band(self.dialog_rect(w, h), ui::MODAL_FRAME)
		}
		fn drag(&mut self, dx: f32, dy: f32) {
			self.drag_offset.0 += dx;
			self.drag_offset.1 += dy;
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
			ModalKey::Backspace => {
				self.on_key(None, true, false);
				ModalAction::Consumed
			}
			ModalKey::Tab => {
				self.on_key(None, false, true);
				ModalAction::Consumed
			}
			ModalKey::Char(c) => {
				self.on_key(Some(c), false, false);
				ModalAction::Consumed
			}
		}
	}

	fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
		from_newmap(self.on_press(x, y, w, h))
	}

	fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
		from_newmap(self.on_release(x, y, w, h))
	}

	modal_drag!();
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
			ModalKey::Backspace => {
				self.on_key(None, true, false);
				ModalAction::Consumed
			}
			ModalKey::Tab => {
				self.on_key(None, false, true);
				ModalAction::Consumed
			}
			ModalKey::Char(c) => {
				self.on_key(Some(c), false, false);
				ModalAction::Consumed
			}
		}
	}

	fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
		from_resize(self.on_press(x, y, w, h))
	}

	fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
		from_resize(self.on_release(x, y, w, h))
	}

	modal_drag!();
}

impl Modal for AutoFix {
	fn on_key(&mut self, key: ModalKey) -> ModalAction {
		match key {
			// Esc stops a live run, else closes — the only key it cares about.
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

	modal_drag!();
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
			ModalKey::Backspace => {
				self.on_key(None, true, false);
				ModalAction::Consumed
			}
			ModalKey::Tab => {
				self.on_key(None, false, true);
				ModalAction::Consumed
			}
			ModalKey::Char(c) => {
				self.on_key(Some(c), false, false);
				ModalAction::Consumed
			}
		}
	}

	fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
		from_generator(self.on_press(x, y, w, h))
	}

	fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
		from_generator(self.on_release(x, y, w, h))
	}

	modal_drag!();
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
			ModalKey::Backspace => {
				self.on_key(None, true, false);
				ModalAction::Consumed
			}
			ModalKey::Tab => {
				self.on_key(None, false, true);
				ModalAction::Consumed
			}
			ModalKey::Char(c) => {
				self.on_key(Some(c), false, false);
				ModalAction::Consumed
			}
		}
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

	modal_drag!();
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

impl Modal for ErrorModal {
	fn on_key(&mut self, key: ModalKey) -> ModalAction {
		match key {
			// Any acknowledgement closes it — it carries no input.
			ModalKey::Enter | ModalKey::Escape => ModalAction::Close,
			_ => ModalAction::Consumed,
		}
	}

	fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
		from_error(self.on_press(x, y, w, h))
	}

	fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
		from_error(self.on_release(x, y, w, h))
	}

	modal_drag!();
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
			ModalKey::Enter => ModalAction::Run("save-and-close".into()),
			ModalKey::Escape => ModalAction::Close,
			_ => ModalAction::Consumed,
		}
	}

	fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
		from_confirm(self.on_press(x, y, w, h))
	}

	fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
		from_confirm(self.on_release(x, y, w, h))
	}

	modal_drag!();
}

fn from_confirm(press: confirm::Press) -> ModalAction {
	match press {
		confirm::Press::Consumed => ModalAction::Consumed,
		confirm::Press::Cancel => ModalAction::Close,
		confirm::Press::Discard => ModalAction::Run("close-project!".into()),
		confirm::Press::Save => ModalAction::Run("save-and-close".into()),
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
			ModalKey::Backspace => {
				self.on_key(None, true, false);
				ModalAction::Consumed
			}
			ModalKey::Tab => {
				self.on_key(None, false, true);
				ModalAction::Consumed
			}
			ModalKey::Char(c) => {
				self.on_key(Some(c), false, false);
				ModalAction::Consumed
			}
		}
	}

	fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
		from_convertpalette(self.on_press(x, y, w, h))
	}

	fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> ModalAction {
		from_convertpalette(self.on_release(x, y, w, h))
	}

	modal_drag!();
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

fn from_resize(press: resize::Press) -> ModalAction {
	match press {
		resize::Press::Consumed => ModalAction::Consumed,
		resize::Press::Abort => ModalAction::Close,
		resize::Press::Resize(line) => ModalAction::Run(line),
		resize::Press::Invalid(e) => ModalAction::Error(format!("resize: {e}")),
	}
}
