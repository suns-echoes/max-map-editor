//! Panic-freedom sweep for the INI parser and its typed accessors.
//!
//! `mme.ini` is user-editable by design and the MAX.RES manifest is INI too, so
//! this parser sits on a trust boundary even though nothing hostile is expected
//! to arrive through it. The contract: any file may be **refused**, none may
//! panic - including the typed reads afterwards, where a value stored as one
//! type is asked for as another.
//!
//! Promoted from the 2026-08-18 security audit's throwaway harnesses. That
//! version wrote each case to a file; `INI::from_str` reaches the same parser
//! without touching the disk.

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

/// Lines chosen to break a line-oriented parser: unterminated sections and
/// quotes, empty keys and values, doubled separators, a BOM, numbers that
/// overflow every integer width, and non-ASCII bytes (deliberate test data -
/// the ASCII rule covers UI strings).
const LINES: &[&str] = &[
	"[s]",
	"[",
	"]",
	"[]",
	"a=b",
	"=b",
	"a=",
	"a",
	";c",
	"#c",
	"\"q",
	"a=\"x",
	"  ",
	"",
	"\r",
	"a=1;2;3",
	"a=\u{1F600}",
	"[\u{e9}]",
	"a==b",
	"[s]]",
	"a=b=c",
	"a=999999999999999999999999",
	"a=-0",
	"a=1e400",
	"[s",
	"s]",
	"a=\"",
	"\"=\"",
	"a=\t",
	"\u{feff}[s]",
	"a=0x10",
	"a=--5",
	"[a][b]",
];

fn document(seed: u64) -> String {
	let mut r = Rng(seed | 1);
	(0..14).map(|_| format!("{}\n", LINES[(r.next() % LINES.len() as u64) as usize])).collect()
}

/// The boxed hook `take_hook` hands back.
type PanicHook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send + 'static>;

/// Silences panic output for as long as it is alive, restoring the previous
/// hook on drop. The hook is global, so a sweep installs it **once** rather
/// than per case: thousands of swaps cost more than the fuzzing itself and
/// would race every other test in the binary.
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

#[test]
fn parser_and_typed_reads_are_panic_free() {
	let _hushed = Hushed::new();
	for seed in 1..8_000u64 {
		let text = document(seed);
		let probe = text.clone();
		let res = std::panic::catch_unwind(move || {
			let Ok(doc) = ini::INI::from_str(&probe) else { return };
			for name in ["s", "missing", "", "a"] {
				let Some(section) = doc.get_section(name) else { continue };
				// Ask for every type regardless of what was stored - the
				// mismatch path is the one a hand-edited mme.ini reaches.
				let _ = section.get_entry::<String>("a");
				let _ = section.get_entry::<i64>("a");
				let _ = section.get_entry::<f64>("a");
			}
		});
		if let Err(e) = res {
			drop(_hushed);
			panic!("parse_ini_str panicked at seed {seed}: {}\n--- input ---\n{text}", panic_message(e));
		}
	}
}
