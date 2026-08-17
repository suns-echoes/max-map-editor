//! The OS text clipboard (a thin arboard wrapper) — shell verbs that hand
//! text out (the Generate dialog's Copy Seed) and both ends of the wgpu-ui
//! text-field channel (`Ui::take_clipboard` → [`set`], Ctrl+V → [`get`] →
//! `Event::Paste`; the toolkit itself is clipboard-blind).

pub(crate) fn set(s: &str) {
	if let Ok(mut cb) = arboard::Clipboard::new() {
		let _ = cb.set_text(s.to_string());
	}
}

/// Best-effort read (`None` when empty, non-text, or unavailable).
pub(crate) fn get() -> Option<String> {
	arboard::Clipboard::new().ok()?.get_text().ok()
}

#[cfg(test)]
mod tests {
	use super::*;

	/// `set` then `get` round-trips text through the OS clipboard (both ends
	/// of the wgpu-ui text-field channel), restoring the previous contents
	/// afterwards. On a box with no reachable clipboard (display-less CI) the
	/// test skips rather than flakes - `set`/`get` are best-effort no-ops
	/// there by design.
	#[test]
	fn set_then_get_round_trips() {
		if arboard::Clipboard::new().is_err() {
			eprintln!("skipping: no OS clipboard available");
			return;
		}
		let previous = get();
		let probe = format!("max-map-editor clipboard probe {}", std::process::id());
		set(&probe);
		let read = get();
		if let Some(p) = &previous {
			set(p);
		}
		assert_eq!(read.as_deref(), Some(probe.as_str()), "clipboard round-trip");
	}
}
