//! Panic-freedom sweep for the JSON parser and serializer.
//!
//! Every document the editor loads that is not a binary game file is JSON -
//! projects, templates, palettes, tile packs, scenery tuning - and those files
//! get shared between users, so `parse` is a trust boundary. The contract this
//! pins is narrow and absolute: **`parse` may return `Err`, but it may never
//! panic**, and neither may re-parsing whatever `to_pretty` emitted.
//!
//! Promoted from the 2026-08-18 security audit's throwaway harnesses. The RNG
//! is a hand-rolled xorshift64* so the sweep stays dependency-free and every
//! reported seed reproduces exactly.

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
}

/// Fragments chosen to break a parser rather than to look like JSON: unbalanced
/// brackets, lone escapes, a leading surrogate, out-of-range exponents, and
/// non-ASCII bytes (deliberate test data - the ASCII rule covers UI strings).
const TOKENS: &[&str] = &[
	"{",
	"}",
	"[",
	"]",
	",",
	":",
	"\"",
	"\"a\"",
	"\"name\"",
	"\"width\"",
	"\"height\"",
	"\"use\"",
	"\"map\"",
	"\"GRA000\"",
	"0",
	"1",
	"-1",
	"1e400",
	"99999999999",
	"null",
	"true",
	"false",
	"\\u",
	"\\ud800",
	"\u{e9}",
	"\u{1F600}",
	" ",
	"\n",
	"\t",
	"\"\\\\\"",
	"0.5",
	"-0.0",
	"\"../..\"",
	"\"/etc\"",
	"\"\"",
	"1.7976931348623157e308",
];

fn soup(seed: u64, n: usize) -> String {
	let mut r = Rng(seed | 1);
	(0..n).map(|_| TOKENS[(r.next() % TOKENS.len() as u64) as usize]).collect()
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

/// Run `case` over `seeds`, reporting the first seed that panicked. Returns
/// after the first failure - one reproducible seed is worth more than a count.
fn sweep(seeds: std::ops::Range<u64>, case: impl Fn(u64) + Copy + std::panic::UnwindSafe) -> Option<(u64, String)> {
	let _hushed = Hushed::new();
	for seed in seeds {
		if let Err(e) = std::panic::catch_unwind(move || case(seed)) {
			return Some((seed, panic_message(e)));
		}
	}
	None
}

#[test]
fn parser_is_panic_free_on_token_soup() {
	if let Some((seed, msg)) = sweep(1..40_000, |seed| {
		if let Ok(v) = json::parse(&soup(seed, 24)) {
			let _ = v.to_pretty();
		}
	}) {
		panic!("json::parse panicked at seed {seed}: {msg}\n--- input ---\n{}", soup(seed, 24));
	}
}

#[test]
fn round_trip_never_panics() {
	// Anything the serializer emits, the parser must survive.
	if let Some((seed, msg)) = sweep(1..20_000, |seed| {
		if let Ok(v) = json::parse(&soup(seed, 16)) {
			let _ = json::parse(&v.to_pretty());
		}
	}) {
		panic!("json round-trip panicked at seed {seed}: {msg}\n--- input ---\n{}", soup(seed, 16));
	}
}

/// The recursion guard, from the outside: 200k nested arrays must come back as
/// an error, not as a blown stack. This is the check that keeps a shared
/// project file from taking the editor down.
#[test]
fn deep_nesting_errors_instead_of_overflowing_the_stack() {
	let deep = format!("{}{}", "[".repeat(200_000), "]".repeat(200_000));
	let e = json::parse(&deep).expect_err("200k levels must be refused");
	assert!(e.contains("nesting deeper than"), "expected the depth guard, got: {e}");
}
