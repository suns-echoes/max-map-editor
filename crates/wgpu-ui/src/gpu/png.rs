//! Minimal PNG encoder (RGBA8, no compression — deflate "stored" blocks).
//! Hand-rolled to keep the dependency tree minimal (no `image`/`png`);
//! screenshots are a debugging/test aid, file size does not matter.

use std::io::{self, Write};
use std::path::Path;

pub fn write_rgba(path: &Path, width: u32, height: u32, rgba: &[u8]) -> io::Result<()> {
    assert_eq!(rgba.len(), width as usize * height as usize * 4);
    let mut file = io::BufWriter::new(std::fs::File::create(path)?);
    file.write_all(b"\x89PNG\r\n\x1a\n")?;

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA, no interlace
    write_chunk(&mut file, b"IHDR", &ihdr)?;

    // Raw scanlines, each prefixed with filter type 0 (None).
    let stride = width as usize * 4;
    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    for row in rgba.chunks_exact(stride) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    write_chunk(&mut file, b"IDAT", &zlib_stored(&raw))?;
    write_chunk(&mut file, b"IEND", &[])?;
    file.flush()
}

fn write_chunk(out: &mut impl Write, kind: &[u8; 4], data: &[u8]) -> io::Result<()> {
    out.write_all(&(data.len() as u32).to_be_bytes())?;
    out.write_all(kind)?;
    out.write_all(data)?;
    let mut crc = Crc32::new();
    crc.update(kind);
    crc.update(data);
    out.write_all(&crc.finish().to_be_bytes())
}

/// Wraps raw bytes in a zlib stream of uncompressed deflate blocks.
fn zlib_stored(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + raw.len() / 0xffff * 5 + 16);
    out.extend_from_slice(&[0x78, 0x01]);
    let mut chunks = raw.chunks(0xffff).peekable();
    loop {
        let Some(chunk) = chunks.next() else {
            // Zero-length final block for empty input.
            out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
            break;
        };
        let last = chunks.peek().is_none();
        out.push(last as u8);
        out.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
        out.extend_from_slice(&(!(chunk.len() as u16)).to_le_bytes());
        out.extend_from_slice(chunk);
        if last {
            break;
        }
    }
    out.extend_from_slice(&adler32(raw).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

struct Crc32(u32);

impl Crc32 {
    fn new() -> Self {
        Self(0xffff_ffff)
    }

    fn update(&mut self, data: &[u8]) {
        for &byte in data {
            self.0 ^= byte as u32;
            for _ in 0..8 {
                let mask = (self.0 & 1).wrapping_neg();
                self.0 = (self.0 >> 1) ^ (0xedb8_8320 & mask);
            }
        }
    }

    fn finish(self) -> u32 {
        !self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_reference() {
        // CRC-32 of "123456789" is the classic check value 0xCBF43926.
        let mut crc = Crc32::new();
        crc.update(b"123456789");
        assert_eq!(crc.finish(), 0xcbf4_3926);
    }

    #[test]
    fn adler32_matches_reference() {
        // Adler-32 of "Wikipedia" is the documented 0x11E60398.
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }

    /// A zero-height image still writes a valid PNG: the IDAT stream must be
    /// the zero-length final stored block plus the Adler-32 of no bytes (1),
    /// not a missing or truncated deflate stream.
    #[test]
    fn zero_height_image_writes_an_empty_stored_stream() {
        let dir = std::env::temp_dir().join("wgpu-ui-png-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("empty.png");
        write_rgba(&path, 4, 0, &[]).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        // 8 signature bytes + 25-byte IHDR chunk; IDAT data follows its own
        // 8-byte chunk header.
        assert_eq!(&bytes[37..41], b"IDAT");
        assert_eq!(
            &bytes[41..52],
            &[0x78, 0x01, 1, 0, 0, 0xff, 0xff, 0, 0, 0, 1],
            "zlib header, empty final stored block, Adler-32 of nothing"
        );
        assert_eq!(&bytes[bytes.len() - 8..bytes.len() - 4], b"IEND");
    }

    #[test]
    fn writes_parseable_png() {
        let dir = std::env::temp_dir().join("wgpu-ui-png-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.png");
        let pixels: Vec<u8> = (0..16)
            .flat_map(|i| [i * 16, 0, 255 - i * 16, 255])
            .collect();
        write_rgba(&path, 4, 4, &pixels).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&bytes[12..16], b"IHDR");
        assert_eq!(&bytes[bytes.len() - 8..bytes.len() - 4], b"IEND");
    }
}
