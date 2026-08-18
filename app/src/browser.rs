//! Open a URL or local file with the system's default handler - no external
//! crate, just the platform launcher (`xdg-open` / `open` / `start`). Used by
//! the Help menu (website, project GitHub, the bundled HTML manual).

use std::process::{Command, Stdio};

/// Characters refused in a launcher target. On Windows the launcher is
/// `cmd /C start`, and `cmd.exe` acts on `& | ^ < >` and `%VAR%` *before* it
/// removes quotes - Rust quotes arguments by the MSVCRT rules, which `cmd` does
/// not follow, so a quoted argument is not a safe one. (Rust's `.bat`/`.cmd`
/// hardening does not apply here: the program is `cmd` itself.) A newline could
/// likewise carry a second command. None of this belongs in a real target - the
/// only ones we pass are two https URLs and the bundled manual's path - so
/// refusing outright costs nothing and does not depend on getting `cmd`'s
/// quoting rules exactly right.
const REFUSED: &[char] = &['&', '|', '^', '<', '>', '"', '\'', '%', '`', '$', '\n', '\r', '\0'];

/// Hand `target` (a URL or file path) to the OS launcher, detached. Returns an
/// error only if the launcher couldn't be spawned (not if the page fails to
/// open later - that's out of our hands).
pub fn open(target: &str) -> Result<(), String> {
	if target.contains(REFUSED) || target.chars().any(char::is_control) {
		return Err(format!("refusing to open '{target}': illegal character in the target"));
	}
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
