//! MAX asset decoders.
//!
//! Pure decoders for M.A.X. file formats - RES archives, WRL maps, indexed
//! images (simple / big / multi), and base-unit-data files. No game logic,
//! no rendering - just bytes in, typed values out.
//!
//! Kept deliberately free of `wgpu` / `winit` dependencies so the binary
//! asset extractor and headless tests can link against it cheaply.

pub mod attribs;
pub mod base64;
pub mod color;
pub mod image;
pub mod res;
pub mod save;
pub mod sha256;
pub mod units;
pub mod wrl;

pub use color::{indexed_to_color, rgb_to_bgra};

#[cfg(test)]
pub(crate) mod testutil {
	/// Report that a fixture-gated test is standing down.
	///
	/// The strongest proofs in this crate - the byte-exact round trip over every
	/// `~/MAX` save, the repair-is-a-no-op sweeps - can only run where the game's
	/// (copyrighted, unshippable) files are, and elsewhere they print a line and
	/// pass. That makes a suite that proved nothing indistinguishable from one
	/// that proved everything. Set `MAX_REQUIRE_FIXTURES=1` on a machine that
	/// *has* the fixtures and every such skip becomes a failure instead, so the
	/// proofs cannot quietly stop running.
	#[track_caller]
	pub fn skip_fixture(what: &str) {
		assert!(
			std::env::var_os("MAX_REQUIRE_FIXTURES").is_none_or(|v| v != "1"),
			"MAX_REQUIRE_FIXTURES=1, but this test skipped: {what}"
		);
		eprintln!("skipping: {what}");
	}
}
