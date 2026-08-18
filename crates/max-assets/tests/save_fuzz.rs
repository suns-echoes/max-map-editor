//! Panic-freedom sweep for the `.DTA` save decoder.
//!
//! `read_save_bytes` is what File > Open Save Game reaches with bytes the user
//! did not write - saves get traded like maps do. The builders below emit files
//! that pass the version/category gate on purpose, so the body decoder (surface
//! map, cargo map, object graph, unit lists) actually runs rather than bouncing
//! off the header.
//!
//! The two guards this pins are `MAX_OBJECT_DEPTH` on the object graph and the
//! `capacity_for` allocation clamps. Both held across the audit's ~47k cases;
//! this keeps them held.
//!
//! Promoted from the 2026-08-18 security audit's throwaway harnesses.

use max_assets::save::read_save_bytes;

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

	fn u8(&mut self) -> u8 {
		match self.next() % 6 {
			0 => 0x00,
			1 => 0xFF,
			2 => 0x01,
			3 => (self.next() % 8) as u8,
			_ => (self.next() >> 24) as u8,
		}
	}

	/// Biased toward the counts and lengths that break allocation clamps.
	fn u32(&mut self) -> u32 {
		match self.next() % 6 {
			0 => 0,
			1 => 1,
			2 => u32::MAX,
			3 => 0x8000_0000,
			4 => (self.next() % 16) as u32,
			_ => self.next() as u32,
		}
	}
}

/// V70: version(u16)=70, game_type(u8), name[30], world(u8), mission(u16),
/// 4x name[30], type[5], clan[5], seed(u32), pre-options(6), options(12x i32).
fn v70(seed: u64) -> Vec<u8> {
	let mut r = Rng(seed | 1);
	let mut d = Vec::new();
	d.extend_from_slice(&70u16.to_le_bytes());
	d.push((r.next() % 6) as u8); // game_type - must map to a category
	d.extend(std::iter::repeat_n(0u8, 30));
	d.push((r.next() % 24) as u8);
	d.extend_from_slice(&(r.next() as u16).to_le_bytes());
	for _ in 0..4 {
		d.extend(std::iter::repeat_n(0u8, 30));
	}
	for _ in 0..10 {
		d.push(r.u8());
	}
	d.extend_from_slice(&r.u32().to_le_bytes()); // rng_seed
	for _ in 0..6 {
		d.push(r.u8());
	}
	for _ in 0..12 {
		d.extend_from_slice(&r.u32().to_le_bytes()); // options
	}
	for _ in 0..4096 {
		d.push(r.u8()); // body: surface map, cargo, object graph, unit lists
	}
	d
}

/// V71: version(u32)=71, category(u32), then length-prefixed script/name/hash,
/// 5 length-prefixed team names, type[5], clan[5], difficulty[5], seed, etc.
fn v71(seed: u64) -> Vec<u8> {
	let mut r = Rng(seed | 1);
	let mut d = Vec::new();
	d.extend_from_slice(&71u32.to_le_bytes());
	d.extend_from_slice(&((r.next() % 6) as u32).to_le_bytes()); // category
	let lenpfx = |d: &mut Vec<u8>, r: &mut Rng| {
		let n = (r.next() % 8) as u32;
		d.extend_from_slice(&n.to_le_bytes());
		for _ in 0..n {
			d.push(r.u8());
		}
	};
	lenpfx(&mut d, &mut r); // script
	lenpfx(&mut d, &mut r); // save_name
	lenpfx(&mut d, &mut r); // world_hash
	for _ in 0..5 {
		lenpfx(&mut d, &mut r); // team names
	}
	for _ in 0..15 {
		d.extend_from_slice(&r.u32().to_le_bytes()); // type/clan/difficulty
	}
	d.extend_from_slice(&r.u32().to_le_bytes()); // rng_seed
	for _ in 0..3 {
		d.extend_from_slice(&r.u32().to_le_bytes()); // pre-options
	}
	for _ in 0..12 {
		d.extend_from_slice(&r.u32().to_le_bytes()); // options
	}
	for _ in 0..4096 {
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

/// The map sizes a save can claim: the stock 112x112, the degenerate 1x1, and
/// a mid-size one. The dimensions come from the slot map, not the save, so a
/// mismatch between them is exactly the attacker-controlled case.
const DIMS: [(u16, u16); 3] = [(112, 112), (1, 1), (64, 64)];

fn sweep(name: &str, seeds: std::ops::Range<u64>, build: impl Fn(u64) -> Vec<u8>) {
	let _hushed = Hushed::new();
	for seed in seeds {
		let d = build(seed);
		for dims in DIMS {
			let probe = d.clone();
			if let Err(e) = std::panic::catch_unwind(move || {
				let _ = read_save_bytes(&probe, dims);
			}) {
				drop(_hushed);
				panic!("{name} panicked at seed {seed}, dims {dims:?}: {}", panic_message(e));
			}
		}
	}
}

#[test]
fn v70_decode_is_panic_free() {
	sweep("read_save_bytes/V70", 1..6_000, v70);
}

#[test]
fn v71_decode_is_panic_free() {
	sweep("read_save_bytes/V71", 1..6_000, v71);
}

/// Truncation at every prefix length of an otherwise well-formed file - the
/// half-written save, and the cheapest way to reach every "ran out of bytes"
/// branch in the decoder.
#[test]
fn truncated_saves_are_panic_free() {
	let _hushed = Hushed::new();
	for seed in 1..60u64 {
		for build in [v70 as fn(u64) -> Vec<u8>, v71 as fn(u64) -> Vec<u8>] {
			let full = build(seed);
			for cut in (0..full.len()).step_by(7) {
				let probe = full[..cut].to_vec();
				if let Err(e) = std::panic::catch_unwind(move || {
					let _ = read_save_bytes(&probe, (112, 112));
				}) {
					drop(_hushed);
					panic!("a truncated save panicked at seed {seed}, cut {cut}: {}", panic_message(e));
				}
			}
		}
	}
}
