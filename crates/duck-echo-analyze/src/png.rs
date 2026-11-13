//! A small PNG encoder for 8-bit palette images, so pictures of sounds
//! need no image or compression dependency.
//!
//! The image data is compressed with deflate using LZ77 matching and the
//! fixed Huffman codes from RFC 1951, inside a zlib stream (RFC 1950).
//! Spectrograms and level plots have long runs of one colour, which this
//! compresses well; it is not as small as a full encoder, and it does not
//! need to be.

/// A palette image: one byte per pixel, each an index into `palette`.
#[derive(Debug, Clone, PartialEq)]
pub struct Indexed {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Up to 256 RGB colours.
    pub palette: Vec<[u8; 3]>,
    /// `width × height` palette indices, row by row from the top.
    pub pixels: Vec<u8>,
}

/// Encode an indexed image as PNG bytes.
pub fn encode(img: &Indexed) -> Vec<u8> {
    assert!(!img.palette.is_empty() && img.palette.len() <= 256);
    assert_eq!(img.pixels.len(), (img.width * img.height) as usize);
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend(img.width.to_be_bytes());
    ihdr.extend(img.height.to_be_bytes());
    // Bit depth 8, colour type 3 (palette), deflate, adaptive filters, no
    // interlace.
    ihdr.extend([8, 3, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    let plte: Vec<u8> = img.palette.iter().flatten().copied().collect();
    chunk(&mut out, b"PLTE", &plte);
    // Every row uses filter 0 (none), the usual choice for palette images.
    let w = img.width as usize;
    let mut raw = Vec::with_capacity(img.pixels.len() + img.height as usize);
    for row in img.pixels.chunks(w.max(1)) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    chunk(&mut out, b"IDAT", &zlib(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend(crc.to_be_bytes());
}

/// CRC-32 (ISO 3309, as PNG uses it).
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Adler-32, the zlib checksum.
pub fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in bytes.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

/// A zlib stream holding one fixed-Huffman deflate block.
pub fn zlib(data: &[u8]) -> Vec<u8> {
    // CMF 0x78: deflate with a 32 KiB window. FLG 0x01 makes the header a
    // multiple of 31, with no preset dictionary.
    let mut out = vec![0x78, 0x01];
    let mut bits = BitWriter::default();
    bits.put(1, 1); // final block
    bits.put(1, 2); // fixed Huffman codes
    lz77(data, &mut bits);
    put_literal(&mut bits, 256); // end of block
    out.extend(bits.finish());
    out.extend(adler32(data).to_be_bytes());
    out
}

#[derive(Default)]
struct BitWriter {
    bytes: Vec<u8>,
    acc: u32,
    count: u32,
}

impl BitWriter {
    /// Append the low `n` bits of `value`, least significant first.
    fn put(&mut self, value: u32, n: u32) {
        self.acc |= value << self.count;
        self.count += n;
        while self.count >= 8 {
            self.bytes.push(self.acc as u8);
            self.acc >>= 8;
            self.count -= 8;
        }
    }

    /// Append a Huffman code of `n` bits, most significant bit first.
    fn put_code(&mut self, code: u32, n: u32) {
        let reversed = code.reverse_bits() >> (32 - n);
        self.put(reversed, n);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.count > 0 {
            self.bytes.push(self.acc as u8);
        }
        self.bytes
    }
}

/// Fixed literal/length code for symbol 0..=287.
fn put_literal(bits: &mut BitWriter, sym: u32) {
    match sym {
        0..=143 => bits.put_code(0x30 + sym, 8),
        144..=255 => bits.put_code(0x190 + sym - 144, 9),
        256..=279 => bits.put_code(sym - 256, 7),
        _ => bits.put_code(0xC0 + sym - 280, 8),
    }
}

const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

fn put_match(bits: &mut BitWriter, length: usize, distance: usize) {
    let li = LENGTH_BASE
        .iter()
        .rposition(|&b| b as usize <= length)
        .unwrap();
    put_literal(bits, 257 + li as u32);
    if LENGTH_EXTRA[li] > 0 {
        bits.put(
            (length - LENGTH_BASE[li] as usize) as u32,
            LENGTH_EXTRA[li] as u32,
        );
    }
    let di = DIST_BASE
        .iter()
        .rposition(|&b| b as usize <= distance)
        .unwrap();
    bits.put_code(di as u32, 5);
    if DIST_EXTRA[di] > 0 {
        bits.put(
            (distance - DIST_BASE[di] as usize) as u32,
            DIST_EXTRA[di] as u32,
        );
    }
}

/// Greedy LZ77 with hash chains over 3-byte prefixes.
fn lz77(data: &[u8], bits: &mut BitWriter) {
    const WINDOW: usize = 32 * 1024;
    const MAX_LEN: usize = 258;
    const MAX_CHAIN: usize = 48;
    const HASH_BITS: u32 = 15;
    let hash = |i: usize| {
        let v = (data[i] as u32) << 16 | (data[i + 1] as u32) << 8 | data[i + 2] as u32;
        (v.wrapping_mul(2_654_435_761) >> (32 - HASH_BITS)) as usize
    };
    let mut head = vec![usize::MAX; 1 << HASH_BITS];
    let mut prev = vec![usize::MAX; data.len()];
    let insert = |i: usize, head: &mut Vec<usize>, prev: &mut Vec<usize>| {
        if i + 2 < data.len() {
            let h = hash(i);
            prev[i] = head[h];
            head[h] = i;
        }
    };
    let mut i = 0;
    while i < data.len() {
        let mut best = (0usize, 0usize);
        if i + 2 < data.len() {
            let mut cand = head[hash(i)];
            let mut chain = 0;
            let limit = MAX_LEN.min(data.len() - i);
            while cand != usize::MAX && i - cand <= WINDOW && chain < MAX_CHAIN {
                let len = data[cand..]
                    .iter()
                    .zip(&data[i..i + limit])
                    .take_while(|(a, b)| a == b)
                    .count();
                if len > best.0 {
                    best = (len, i - cand);
                    if len == limit {
                        break;
                    }
                }
                cand = prev[cand];
                chain += 1;
            }
        }
        if best.0 >= 3 {
            put_match(bits, best.0, best.1);
            for j in i..i + best.0 {
                insert(j, &mut head, &mut prev);
            }
            i += best.0;
        } else {
            put_literal(bits, data[i] as u32);
            insert(i, &mut head, &mut prev);
            i += 1;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A minimal inflater for the streams [`zlib`] writes (fixed Huffman
    /// blocks only), to prove they decode to the input.
    pub(crate) fn inflate_zlib(z: &[u8]) -> Vec<u8> {
        assert_eq!((z[0] as u32 * 256 + z[1] as u32) % 31, 0, "zlib header");
        let body = &z[2..z.len() - 4];
        let mut pos = 0usize; // bit position
        let mut bit = |n: u32| -> u32 {
            let mut v = 0;
            for k in 0..n {
                let b = (body[pos / 8] >> (pos % 8)) & 1;
                v |= (b as u32) << k;
                pos += 1;
            }
            v
        };
        let mut out = Vec::new();
        loop {
            let last = bit(1);
            assert_eq!(bit(2), 1, "fixed Huffman block");
            loop {
                // Read a fixed literal/length code MSB first.
                let mut code = 0u32;
                let mut len = 0;
                let sym = loop {
                    code = (code << 1) | bit(1);
                    len += 1;
                    match len {
                        7 if code <= 0x17 => break code + 256,
                        8 if (0x30..=0xBF).contains(&code) => break code - 0x30,
                        8 if (0xC0..=0xC7).contains(&code) => break code - 0xC0 + 280,
                        9 if code >= 0x190 => break code - 0x190 + 144,
                        _ => assert!(len < 9, "bad code"),
                    }
                };
                if sym < 256 {
                    out.push(sym as u8);
                } else if sym == 256 {
                    break;
                } else {
                    let li = (sym - 257) as usize;
                    let length = LENGTH_BASE[li] as usize + bit(LENGTH_EXTRA[li] as u32) as usize;
                    let mut d = 0;
                    for _ in 0..5 {
                        d = (d << 1) | bit(1);
                    }
                    let di = d as usize;
                    let dist = DIST_BASE[di] as usize + bit(DIST_EXTRA[di] as u32) as usize;
                    for _ in 0..length {
                        out.push(out[out.len() - dist]);
                    }
                }
            }
            if last == 1 {
                break;
            }
        }
        let adler = u32::from_be_bytes(z[z.len() - 4..].try_into().unwrap());
        assert_eq!(adler, adler32(&out), "adler32");
        out
    }

    #[test]
    fn zlib_round_trips() {
        let mut data = Vec::new();
        for i in 0..20_000u32 {
            data.push(((i * 7919) % 251) as u8);
            data.extend(std::iter::repeat((i % 5) as u8).take((i % 300) as usize));
        }
        for case in [&b""[..], b"a", b"abcabcabcabcabcabc", &data] {
            let z = zlib(case);
            assert_eq!(inflate_zlib(&z), case);
        }
        assert!(zlib(&data).len() < data.len() / 4);
    }

    #[test]
    fn checksums_match_known_values() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn png_has_valid_chunks() {
        let img = Indexed {
            width: 3,
            height: 2,
            palette: vec![[0, 0, 0], [255, 255, 255]],
            pixels: vec![0, 1, 0, 1, 0, 1],
        };
        let png = encode(&img);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let mut at = 8;
        let mut kinds = Vec::new();
        let mut idat = Vec::new();
        while at < png.len() {
            let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            let kind = &png[at + 4..at + 8];
            let crc = u32::from_be_bytes(png[at + 8 + len..at + 12 + len].try_into().unwrap());
            assert_eq!(crc, crc32(&png[at + 4..at + 8 + len]));
            if kind == b"IDAT" {
                idat.extend_from_slice(&png[at + 8..at + 8 + len]);
            }
            kinds.push(String::from_utf8(kind.to_vec()).unwrap());
            at += 12 + len;
        }
        assert_eq!(kinds, ["IHDR", "PLTE", "IDAT", "IEND"]);
        assert_eq!(inflate_zlib(&idat), [0, 0, 1, 0, 0, 1, 0, 1]);
    }
}
