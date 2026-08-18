//! Panic-freedom sweep for the sprite decoders.
//!
//! These run on bytes out of `MAX.RES`, which lives in whatever M.A.X. install
//! the user points MaxPath at - `units.rs` decodes unit sprites straight from
//! archive bytes and handles only `Err`, so a panic there is not caught by
//! anything. With `panic = "abort"` in release, a panic is an immediate process
//! abort and the open map is gone.
//!
//! Two sweeps, because they reach different code:
//!
//! - **random corpus** - bytes biased toward `0x00` / `0xFF` / small ints, the
//!   values that break length and offset fields. Most of these bounce off each
//!   decoder's header gate.
//! - **structure-aware** - builders that *pass* the header gate on purpose, so
//!   the body decoders (row-offset walk, RLE, palette) actually run. This is
//!   the sweep that found F2; the random one never got past the gate.
//!
//! Promoted from the 2026-08-18 security audit's throwaway harnesses. F2's own
//! regression - the crafted negative frame offset, and `i32::MIN`/`MAX` either
//! side of it - lives next to the check it pins, in `image/multi.rs`.

use max_assets::image;

/// xorshift64* - deterministic, no dependency.
struct Rng(u64);

impl Rng {
	fn next(&mut self) -> u64 {
		let mut x = self.0;
		x ^= x >> 12;
		x ^= x << 25;
		x ^= x >> 27;
		self.0 = x;
		x.wrapping_mul(0x2545_F491_4F6C_DD1D)
	}

	/// Biased toward the values that break length and offset arithmetic.
	fn u8(&mut self) -> u8 {
		match self.next() % 5 {
			0 => 0x00,
			1 => 0xFF,
			2 => 0xFE,
			3 => (self.next() % 4) as u8,
			_ => (self.next() >> 24) as u8,
		}
	}

	fn i32_wild(&mut self) -> i32 {
		match self.next() % 6 {
			0 => -1,
			1 => i32::MIN,
			2 => i32::MAX,
			3 => 0,
			4 => (self.next() % 128) as i32,
			_ => self.next() as i32,
		}
	}

	fn dim(&mut self) -> i16 {
		match self.next() % 5 {
			0 => 1,
			1 => 640,
			2 => 480,
			3 => (self.next() % 64) as i16 + 1,
			_ => self.next() as i16,
		}
	}
}

/// Bytes biased toward `0x00` / `0xFF` / small ints.
fn corpus(seed: u64, len: usize) -> Vec<u8> {
	let mut r = Rng(seed | 1);
	(0..len).map(|_| r.u8()).collect()
}

/// A multi-image that PASSES the `first_offset == 2 + 4*count` gate, with wild
/// per-frame offsets, dims and RLE bodies behind it.
fn multi_image(seed: u64) -> Vec<u8> {
	let mut r = Rng(seed | 1);
	let count = (r.next() % 4) as i16 + 1;
	let first = 2 + 4 * count as i32;
	let mut d = Vec::new();
	d.extend_from_slice(&count.to_le_bytes());
	d.extend_from_slice(&first.to_le_bytes());
	for _ in 1..count {
		d.extend_from_slice(&r.i32_wild().to_le_bytes());
	}
	// Frame header at `first`: width, height, hotspots, then a row-offset table.
	let w = r.dim();
	let h = r.dim();
	d.extend_from_slice(&w.to_le_bytes());
	d.extend_from_slice(&h.to_le_bytes());
	d.extend_from_slice(&(r.next() as i16).to_le_bytes());
	d.extend_from_slice(&(r.next() as i16).to_le_bytes());
	let rows = (h.max(0) as usize).min(64);
	let table_end = d.len() + rows * 4;
	for i in 0..rows {
		// Mostly the "correct" sequential offset so the row walk proceeds,
		// sometimes wild so the mismatch/overflow paths get hit.
		let off = if r.next().is_multiple_of(3) { r.i32_wild() } else { (table_end + i * 3) as i32 };
		d.extend_from_slice(&off.to_le_bytes());
	}
	for _ in 0..256 {
		d.push(r.u8());
	}
	d
}

/// A simple image whose `len - 8 == w*h` invariant holds, so the body runs.
fn simple_image(seed: u64) -> Vec<u8> {
	let mut r = Rng(seed | 1);
	let w = (r.next() % 40) as i16 + 1;
	let h = (r.next() % 40) as i16 + 1;
	let mut d = Vec::new();
	d.extend_from_slice(&w.to_le_bytes());
	d.extend_from_slice(&h.to_le_bytes());
	d.extend_from_slice(&(r.next() as i16).to_le_bytes());
	d.extend_from_slice(&(r.next() as i16).to_le_bytes());
	for _ in 0..(w as usize * h as usize) {
		d.push(r.u8());
	}
	d
}

/// A big image: 2-byte w, 2-byte h, hotspots, a 256x3 palette, then an RLE body.
fn big_image(seed: u64) -> Vec<u8> {
	let mut r = Rng(seed | 1);
	let mut d = Vec::new();
	d.extend_from_slice(&r.dim().to_le_bytes());
	d.extend_from_slice(&r.dim().to_le_bytes());
	d.extend_from_slice(&(r.next() as i16).to_le_bytes());
	d.extend_from_slice(&(r.next() as i16).to_le_bytes());
	for _ in 0..(256 * 3) {
		d.push(r.u8());
	}
	for _ in 0..512 {
		d.push(r.u8());
	}
	d
}

/// The boxed hook `take_hook` hands back.
type PanicHook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send + 'static>;

/// Silences panic output for as long as it is alive, restoring the previous
/// hook on drop. The hook is global, so a sweep installs it **once** rather
/// than per case: tens of thousands of swaps cost more than the fuzzing itself
/// and would race every other test in the binary.
struct Hushed(Option<PanicHook>);

impl Hushed {
	fn new() -> Self {
		let prev = std::panic::take_hook();
		std::panic::set_hook(Box::new(|_| {}));
		Self(Some(prev))
	}
}

impl Drop for Hushed {
	fn drop(&mut self) {
		if let Some(prev) = self.0.take() {
			std::panic::set_hook(prev);
		}
	}
}

fn panic_message(e: Box<dyn std::any::Any + Send>) -> String {
	e.downcast_ref::<String>()
		.cloned()
		.or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_string()))
		.unwrap_or_else(|| "<non-string panic payload>".to_string())
}

/// Feed `build(seed)` to `call` for every seed, failing on the first panic with
/// the seed that reproduces it.
fn sweep(
	name: &str,
	seeds: std::ops::Range<u64>,
	build: impl Fn(u64) -> Vec<u8>,
	call: impl Fn(&[u8]) + Copy + std::panic::UnwindSafe,
) {
	let _hushed = Hushed::new();
	for seed in seeds {
		let d = build(seed);
		let probe = d.clone();
		if let Err(e) = std::panic::catch_unwind(move || call(&probe)) {
			drop(_hushed);
			panic!("{name} panicked at seed {seed}: {}\n--- {} bytes ---\n{d:02x?}", panic_message(e), d.len());
		}
	}
}

/// A decoder's name paired with a call that runs it and drops whatever it
/// returns - the return types differ, the panic-freedom contract does not.
type Decoder = (&'static str, fn(&[u8]));

/// Every byte-slice decoder the app exposes, named so a failure says which one
/// broke rather than just that something did.
const DECODERS: &[Decoder] = &[
	("parse_simple_image", |d| {
		let _ = image::parse_simple_image(d);
	}),
	("parse_simple_image_indexed", |d| {
		let _ = image::parse_simple_image_indexed(d);
	}),
	("parse_big_image", |d| {
		let _ = image::parse_big_image(d);
	}),
	("parse_big_image_indexed", |d| {
		let _ = image::parse_big_image_indexed(d);
	}),
	("parse_multi_image", |d| {
		let _ = image::parse_multi_image(d);
	}),
	("parse_multi_image_all_frames", |d| {
		let _ = image::parse_multi_image_all_frames(d);
	}),
	("decode_multi_image_indexed", |d| {
		let _ = image::decode_multi_image_indexed(d);
	}),
	("decode_multi_image_shadow_indexed", |d| {
		let _ = image::decode_multi_image_shadow_indexed(d);
	}),
	("image_rle_decode", |d| {
		let _ = image::image_rle_decode(d);
	}),
];

#[test]
fn decoders_never_panic_on_random_garbage() {
	let _hushed = Hushed::new();
	for seed in 1..3_000u64 {
		// Lengths straddling each decoder's header size, so the "one byte short"
		// cases - where a `len - 8` underflows - are all hit.
		for len in [0usize, 1, 8, 9, 12, 20, 21, 40, 64, 300, 2048] {
			let d = corpus(seed, len);
			for (name, call) in DECODERS {
				let probe = d.clone();
				if let Err(e) = std::panic::catch_unwind(move || call(&probe)) {
					drop(_hushed);
					panic!("{name} panicked at seed {seed}, len {len}: {}", panic_message(e));
				}
			}
		}
	}
}

#[test]
fn multi_image_body_is_panic_free() {
	sweep("parse_multi_image", 1..20_000, multi_image, |d| {
		let _ = image::parse_multi_image(d);
		let _ = image::parse_multi_image_all_frames(d);
	});
}

#[test]
fn multi_image_indexed_body_is_panic_free() {
	sweep("decode_multi_image_indexed", 1..20_000, multi_image, |d| {
		let _ = image::decode_multi_image_indexed(d);
	});
}

#[test]
fn multi_image_shadow_body_is_panic_free() {
	sweep("decode_multi_image_shadow_indexed", 1..20_000, multi_image, |d| {
		let _ = image::decode_multi_image_shadow_indexed(d);
	});
}

#[test]
fn simple_image_body_is_panic_free() {
	sweep("parse_simple_image", 1..20_000, simple_image, |d| {
		let _ = image::parse_simple_image(d);
		let _ = image::parse_simple_image_indexed(d);
	});
}

#[test]
fn big_image_body_is_panic_free() {
	sweep("parse_big_image", 1..20_000, big_image, |d| {
		let _ = image::parse_big_image(d);
		let _ = image::parse_big_image_indexed(d);
	});
}
