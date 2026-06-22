//! Embed the app icon into the Windows executable - hand-rolled, no build
//! dependencies (house rule). On any other target this script is a no-op.
//!
//! How: write a one-line `.rc` referencing `assets/icon.ico`, compile it with
//! the Windows SDK's `rc.exe` into a `.res`, and hand that to the MSVC linker
//! (`link.exe` accepts `.res` files as plain linker arguments). If `rc.exe`
//! can't be found the build still succeeds - just without an icon - so
//! cross-checks and unusual setups never break on this.

use std::path::PathBuf;
use std::process::Command;

fn main() {
	println!("cargo:rerun-if-changed=assets/icon.ico");
	println!("cargo:rerun-if-changed=build.rs");
	if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
		return;
	}

	let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
	let ico = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("assets/icon.ico");
	let rc = out_dir.join("icon.rc");
	let res = out_dir.join("icon.res");

	// Resource id 1 = the first icon; Explorer and the taskbar pick it up.
	std::fs::write(&rc, format!("1 ICON \"{}\"\n", ico.display().to_string().replace('\\', "\\\\")))
		.expect("write icon.rc");

	let Some(rc_exe) = find_rc_exe() else {
		println!("cargo:warning=rc.exe not found - building without an embedded icon");
		return;
	};
	let status = Command::new(&rc_exe).arg("/nologo").arg(format!("/fo{}", res.display())).arg(&rc).status();
	match status {
		Ok(s) if s.success() => println!("cargo:rustc-link-arg-bins={}", res.display()),
		other => println!("cargo:warning=rc.exe failed ({other:?}) - building without an embedded icon"),
	}
}

/// `rc.exe` lives in the Windows 10/11 SDK, not on PATH. Try PATH first, then
/// glob the standard SDK install location, newest kit version last (sorted).
fn find_rc_exe() -> Option<PathBuf> {
	if Command::new("rc.exe").arg("/?").output().is_ok() {
		return Some(PathBuf::from("rc.exe"));
	}
	let kits = PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\bin");
	let mut versions: Vec<PathBuf> =
		std::fs::read_dir(&kits).ok()?.flatten().map(|e| e.path()).filter(|p| p.join("x64/rc.exe").is_file()).collect();
	versions.sort();
	versions.pop().map(|p| p.join("x64/rc.exe"))
}
