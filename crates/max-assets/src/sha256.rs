//! Dependency-free SHA-256 (FIPS 180-4).
//!
//! Hand-rolled to honour the project's no-new-dependency rule. Used for the
//! save-load **world compatibility check**: M.A.X. Port identifies a save's
//! world by the SHA-256 of the world `.WRL` file (`World::ComputeHash`,
//! `world.cpp`), and the stock-world hashes are tabulated in
//! [`crate::save`] (`WORLD_HASHES`). [`sha256_file`] reproduces those hashes;
//! the streaming [`Sha256`] context also serves ad-hoc content hashing.
//!
//! Not a constant-time or side-channel-hardened implementation — it hashes
//! local asset files, not secrets.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

/// Initial hash values `H(0)` — the first 32 bits of the fractional parts of the
/// square roots of the first eight primes.
const H0: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];

/// Round constants `K` — the first 32 bits of the fractional parts of the cube
/// roots of the first sixty-four primes.
#[rustfmt::skip]
const K: [u32; 64] = [
	0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
	0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
	0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
	0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
	0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
	0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
	0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
	0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Streaming SHA-256 hasher. Feed bytes with [`Sha256::update`] (any number of
/// calls, any chunking) then [`Sha256::finalize`] once.
#[derive(Clone)]
pub struct Sha256 {
	state: [u32; 8],
	/// Partial 64-byte block awaiting a full block before compression.
	block: [u8; 64],
	block_len: usize,
	/// Total message length in bytes (for the length padding).
	total_len: u64,
}

impl Default for Sha256 {
	fn default() -> Self {
		Self::new()
	}
}

impl Sha256 {
	pub fn new() -> Self {
		Sha256 { state: H0, block: [0; 64], block_len: 0, total_len: 0 }
	}

	/// Absorbs more message bytes.
	pub fn update(&mut self, mut data: &[u8]) {
		self.total_len = self.total_len.wrapping_add(data.len() as u64);

		// Top up a partially-filled block first.
		if self.block_len > 0 {
			let take = (64 - self.block_len).min(data.len());
			self.block[self.block_len..self.block_len + take].copy_from_slice(&data[..take]);
			self.block_len += take;
			data = &data[take..];
			if self.block_len == 64 {
				let block = self.block;
				self.compress(&block);
				self.block_len = 0;
			}
		}

		// Consume as many whole blocks as possible without copying twice.
		while data.len() >= 64 {
			let mut block = [0u8; 64];
			block.copy_from_slice(&data[..64]);
			self.compress(&block);
			data = &data[64..];
		}

		// Stash the remainder.
		if !data.is_empty() {
			self.block[..data.len()].copy_from_slice(data);
			self.block_len = data.len();
		}
	}

	/// Appends the padding and length block and returns the 32-byte digest.
	pub fn finalize(mut self) -> [u8; 32] {
		let bit_len = self.total_len.wrapping_mul(8);

		// Append the mandatory 0x80 terminator (there is always room: a full
		// block would already have been compressed, so block_len <= 63).
		self.block[self.block_len] = 0x80;
		self.block_len += 1;

		// If the 8-byte length won't fit in this block, flush a padded block.
		if self.block_len > 56 {
			for b in &mut self.block[self.block_len..] {
				*b = 0;
			}
			let block = self.block;
			self.compress(&block);
			self.block_len = 0;
		}

		// Zero-fill up to the length field, then the big-endian bit length.
		for b in &mut self.block[self.block_len..56] {
			*b = 0;
		}
		self.block[56..64].copy_from_slice(&bit_len.to_be_bytes());
		let block = self.block;
		self.compress(&block);

		let mut out = [0u8; 32];
		for (chunk, word) in out.chunks_exact_mut(4).zip(self.state) {
			chunk.copy_from_slice(&word.to_be_bytes());
		}
		out
	}

	/// The SHA-256 compression function over one 512-bit block.
	fn compress(&mut self, block: &[u8; 64]) {
		let mut w = [0u32; 64];
		for (word, chunk) in w[..16].iter_mut().zip(block.chunks_exact(4)) {
			*word = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
		}
		for i in 16..64 {
			let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
			let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
			w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
		}

		let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
		for i in 0..64 {
			let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
			let ch = (e & f) ^ ((!e) & g);
			let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
			let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
			let maj = (a & b) ^ (a & c) ^ (b & c);
			let t2 = s0.wrapping_add(maj);
			h = g;
			g = f;
			f = e;
			e = d.wrapping_add(t1);
			d = c;
			c = b;
			b = a;
			a = t1.wrapping_add(t2);
		}

		for (s, v) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
			*s = s.wrapping_add(v);
		}
	}
}

/// The raw 32-byte SHA-256 digest of `data`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
	let mut ctx = Sha256::new();
	ctx.update(data);
	ctx.finalize()
}

/// The lowercase-hex SHA-256 digest of `data` (64 chars), matching the form
/// M.A.X. Port stores for world hashes.
pub fn sha256_hex(data: &[u8]) -> String {
	to_hex(&sha256(data))
}

/// Streams a file through SHA-256 and returns the lowercase-hex digest — the
/// exact operation `World::ComputeHash` performs on a `.WRL` file.
pub fn sha256_file(path: &Path) -> io::Result<String> {
	let mut file = File::open(path)?;
	let mut ctx = Sha256::new();
	let mut buf = [0u8; 8192];
	loop {
		let n = file.read(&mut buf)?;
		if n == 0 {
			break;
		}
		ctx.update(&buf[..n]);
	}
	Ok(to_hex(&ctx.finalize()))
}

fn to_hex(digest: &[u8; 32]) -> String {
	use std::fmt::Write;
	let mut s = String::with_capacity(64);
	for byte in digest {
		let _ = write!(s, "{byte:02x}");
	}
	s
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn matches_known_vectors() {
		// FIPS 180-4 / RFC 6234 published digests.
		assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
		assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
		// A 56-byte message: exercises the two-block padding path (0x80 lands at
		// offset 56, forcing the length into a second block).
		assert_eq!(
			sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
			"248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
		);
	}

	#[test]
	fn matches_one_million_a() {
		// The classic long-message vector — spans thousands of blocks and fed
		// in awkward chunk sizes to exercise buffer carry-over.
		let mut ctx = Sha256::new();
		let chunk = vec![b'a'; 1000];
		for _ in 0..1000 {
			ctx.update(&chunk);
		}
		assert_eq!(to_hex(&ctx.finalize()), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
	}

	#[test]
	fn streaming_matches_one_shot() {
		let data: Vec<u8> = (0..500u32).map(|i| (i * 37) as u8).collect();
		let one_shot = sha256_hex(&data);
		// Feed the same data one byte at a time.
		let mut ctx = Sha256::new();
		for &byte in &data {
			ctx.update(&[byte]);
		}
		assert_eq!(to_hex(&ctx.finalize()), one_shot);
	}
}
