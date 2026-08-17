//! The write/edit side's error type.
//!
//! The read side has [`super::read::SaveError`]; this is its counterpart for
//! everything that mutates or re-serializes a save — unit insertion/removal,
//! stat overrides, complex repair, settings, synthesis, `write_save`. Two
//! failure families exist and callers may want to treat them differently,
//! which a bare `String` never let them do (audit 2026-08-07):
//!
//! - [`EditError::Tail`] — the save's undecoded tail section would not
//!   decompose (or re-follow the moved object graph). The *file* defeated the
//!   editor: nothing was written, and no different input would have helped.
//! - [`EditError::InvalidInput`] — the *request* names something the save or
//!   unit library does not have (an unknown type, an off-map position, a team
//!   that owns nothing). A corrected input can succeed.
//!
//! Both are `#[error("{0}")]`-transparent over the message the site already
//! wrote, so every console line and test expectation reads exactly as before
//! the split.

/// An error from the save write/edit paths — see the module doc for the two
/// families and what a caller may conclude from each.
#[derive(Debug, thiserror::Error)]
pub enum EditError {
	/// The save's tail section would not decompose or follow the graph; the
	/// edit was abandoned with nothing written.
	#[error("{0}")]
	Tail(String),
	/// The request itself is impossible against this save/library; correct
	/// the input and the same edit can succeed.
	#[error("{0}")]
	InvalidInput(String),
	/// The save's own structures are inconsistent (a table references the
	/// wrong object class); the edit refuses rather than guess.
	#[error("{0}")]
	Corrupt(String),
}

/// The bridge into the string-error callers above this crate (map-core's
/// project ops, the editor console): `?` keeps working there, message
/// unchanged, while a caller that cares can match the variant instead.
impl From<EditError> for String {
	fn from(e: EditError) -> String {
		e.to_string()
	}
}
