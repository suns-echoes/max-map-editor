//! Panic-freedom sweep for `Template::from_str`.
//!
//! Templates are the document type users trade with each other - File > Import
//! Template runs this parser on a file someone else wrote - so it is a trust
//! boundary in the same sense the binary decoders are. The contract: any
//! document may be **refused**, none may panic.
//!
//! Promoted from the 2026-08-18 security audit's throwaway harnesses. That
//! audit's actual finding (F1) was not a parser crash but a *logic* bug - an
//! imported `use` name flowed into a path component and chose the directory the
//! editor wrote to. Its regression test lives next to the check, in
//! `template.rs`; this file covers the shape the check does not: that nothing
//! in here reaches an unwrap.

use map_core::Template;

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

/// Fragments chosen to break a parser rather than to look like a document.
/// The non-ASCII entries are deliberate test data (the ASCII rule covers UI
/// strings), and the path-shaped ones aim at the F1 class directly.
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

/// A real template skeleton with fuzzed fields: dimensions that disagree with
/// the cell grid, garbage tile ids, and a `use` name that is usually legal so
/// the body actually gets parsed - since F1 landed, an illegal pack name is
/// refused at the `use` loop and everything after it goes unexercised.
fn template_doc(seed: u64) -> String {
	let mut r = Rng(seed | 1);
	let w = r.next() % 8;
	let h = r.next() % 8;
	let cells: Vec<String> = (0..h)
		.map(|_| {
			let row: Vec<String> = (0..w).map(|_| format!("\"{}\"", soup(r.next(), 2))).collect();
			format!("[{}]", row.join(","))
		})
		.collect();
	let pack = if r.next().is_multiple_of(4) { format!("\"{}\"", soup(r.next(), 2)) } else { "\"GREEN\"".to_string() };
	format!(
		r#"{{"name":{},"width":{},"height":{},"use":[{{"name":{pack},"version":"1"}}],"map":[{}]}}"#,
		if r.next().is_multiple_of(2) { "\"n\"".into() } else { format!("\"{}\"", soup(r.next(), 2)) },
		if r.next().is_multiple_of(3) { format!("{}", r.next() as i32) } else { w.to_string() },
		if r.next().is_multiple_of(3) { format!("{}", r.next() as i32) } else { h.to_string() },
		cells.join(",")
	)
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
fn from_str_is_panic_free_on_structured_documents() {
	if let Some((seed, msg)) = sweep(1..30_000, |seed| {
		let _ = Template::from_str(&template_doc(seed));
	}) {
		panic!("Template::from_str panicked at seed {seed}: {msg}\n--- input ---\n{}", template_doc(seed));
	}
}

#[test]
fn from_str_is_panic_free_on_token_soup() {
	if let Some((seed, msg)) = sweep(1..20_000, |seed| {
		let _ = Template::from_str(&soup(seed, 24));
	}) {
		panic!("Template::from_str panicked at seed {seed}: {msg}\n--- input ---\n{}", soup(seed, 24));
	}
}
