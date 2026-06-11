//! Regression suite: run every `scripts/*.script` headless through the real
//! binary and check its exit. The scripts already carry their own assertions
//! (`assert-cell` / `assert-hash` / `assert-dirty`), so the exit code *is* the
//! check — this turns the hand-written scripts into golden tests.
//!
//! A script declares its expectation with a `# test:` directive in its first
//! few lines (default `pass`):
//!   - `# test: pass` — must exit 0 (all assertions held).
//!   - `# test: fail` — must exit non-zero *cleanly* (a negative test, e.g. a
//!     dirty-guard that should refuse — see `open-guard.script`).
//!   - `# test: skip` — run for crash-detection only; the exit code isn't
//!     gated (used to park scripts whose golden `assert-hash` values have gone
//!     stale after an intentional algorithm change, pending a refresh).
//!
//! Runs from the workspace root so `resources/` and `scripts/` resolve. The
//! headless renderer needs a GPU adapter and several scripts load the default
//! map, so the suite **self-skips** when it can't run (CI without a GPU, or the
//! default map absent) rather than failing — run it locally for coverage.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

/// The base map every script run opens — passed to the binary explicitly, so
/// the golden `assert-cell` / `assert-hash` values recorded against the
/// original GREEN_1.WRL stay valid even though the *app's* no-argument
/// default is the GREEN_1 template project. The original maps are copyrighted
/// game data and live in the gitignored `testdata/originals/` — run
/// `tools/fetch-testdata.sh` to restore them from a M.A.X. install.
const DEFAULT_MAP: &str = "testdata/originals/GREEN_1.WRL";

enum Expect {
	Pass,
	Fail,
	Skip,
}

/// Read a script's `# test:` directive from its first lines (default `Pass`).
fn expectation(script: &Path) -> Expect {
	let text = std::fs::read_to_string(script).unwrap_or_default();
	for line in text.lines().take(8) {
		if let Some(rest) = line.trim().strip_prefix("# test:") {
			// First token only — a trailing `# …` note may follow the keyword.
			return match rest.split_whitespace().next() {
				Some("fail") => Expect::Fail,
				Some("skip") => Expect::Skip,
				_ => Expect::Pass,
			};
		}
	}
	Expect::Pass
}

/// Did the run meet its expectation? A `None` exit code means a signal killed
/// the process (segfault/abort) — always a failure, even for `fail`/`skip`.
fn met(expect: &Expect, status: ExitStatus) -> bool {
	match expect {
		Expect::Pass => status.success(),
		Expect::Fail => !status.success() && status.code().is_some(),
		Expect::Skip => status.code().is_some(),
	}
}

#[test]
fn scripts_pass_headless() {
	if std::env::var_os("CI").is_some() {
		eprintln!("skipping script suite: CI is set (runners have no GPU)");
		return;
	}
	let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root");
	if !root.join(DEFAULT_MAP).exists() {
		eprintln!("SKIPPED: script suite — base map {DEFAULT_MAP} not present");
		eprintln!("         run tools/fetch-testdata.sh (or set MAX_DIR) to restore this coverage");
		return;
	}

	let bin = env!("CARGO_BIN_EXE_max-map-editor");
	std::fs::create_dir_all(root.join("temp")).expect("temp dir");

	let mut scripts: Vec<PathBuf> = std::fs::read_dir(root.join("scripts"))
		.expect("scripts dir")
		.filter_map(|e| e.ok().map(|e| e.path()))
		.filter(|p| p.extension().is_some_and(|e| e == "script"))
		.collect();
	scripts.sort();
	assert!(!scripts.is_empty(), "no scripts found under scripts/");

	let mut failures = Vec::new();
	for script in &scripts {
		let expect = expectation(script);
		let status = Command::new(bin)
			.current_dir(root)
			.args([DEFAULT_MAP, "--script", script.to_str().unwrap(), "--headless"])
			.status()
			.expect("run editor");
		if !met(&expect, status) {
			failures.push(format!("  {} -> exit {:?}", script.display(), status.code()));
		}
	}
	assert!(failures.is_empty(), "{} script(s) failed:\n{}", failures.len(), failures.join("\n"));
}
