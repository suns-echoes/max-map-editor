//! Open a URL or local file with the system's default handler - no external
//! crate, just the platform launcher (`xdg-open` / `open` / `start`). Used by
//! the Help menu (website, project GitHub, the bundled HTML manual).

use std::process::{Command, Stdio};

/// Hand `target` (a URL or file path) to the OS launcher, detached. Returns an
/// error only if the launcher couldn't be spawned (not if the page fails to
/// open later - that's out of our hands).
pub fn open(target: &str) -> Result<(), String> {
	// `cfg!` keeps every branch compiling on every platform; the launcher is
	// `xdg-open` on Linux/BSD, `open` on macOS, and `cmd /C start` on Windows
	// (the empty "" is start's window-title argument, so a URL isn't mistaken
	// for one).
	let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "windows") {
		("cmd", vec!["/C", "start", "", target])
	} else if cfg!(target_os = "macos") {
		("open", vec![target])
	} else {
		("xdg-open", vec![target])
	};
	Command::new(program)
		.args(&args)
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.spawn()
		.map(|_| ())
		.map_err(|e| format!("could not open '{target}': {e}"))
}
