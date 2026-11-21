//! Timing and level facts about a rendered sound, for a game engine.
//!
//! Every WAV that Duck Echo renders gets a small JSON file next to it,
//! `<name>.wav.json`, with the same facts that `render --json` returns as
//! `asset`:
//!
//! * its length, loudness and peak, so the engine can set volumes from
//!   loudness;
//! * the silence before it starts, and `hit_seconds`, when its main hit
//!   happens (a thump's loudest moment comes a few milliseconds after its
//!   first sample), so the engine can start it early and land the hit on
//!   the event's frame;
//! * whether it loops;
//! * the recipe's hash, so the engine (or a person) can tell which recipe
//!   made the file and whether it changed since.
//!
//! Made for Duck Forge while making the sound of the film "Pick a Door:
//! Blocks" (roadmap task 13: per-sound volume, the recipe kept with the
//! asset).

use std::path::{Path, PathBuf};

use duck_echo_audio::AudioBuffer;
use serde::Serialize;

use crate::ToolError;

/// What the sidecar file holds.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssetInfo {
    /// Always `duck-echo-asset`, so a reader knows what it has.
    pub kind: &'static str,
    /// The version of this layout: 1.
    pub version: u32,
    /// The WAV file's name (no folder).
    pub file: String,
    /// Length in seconds.
    pub duration_seconds: f64,
    /// Length in frames (samples per channel).
    pub frames: usize,
    /// Samples per second.
    pub sample_rate: u32,
    /// Channels.
    pub channels: u16,
    /// Loudness in LUFS (`null` when silent). See `loudness_method`.
    pub loudness_lufs: Option<f32>,
    /// `bs1770_gated` for 400 ms or more, `k_weighted_ungated` for
    /// shorter sounds.
    pub loudness_method: Option<duck_echo_analyze::LoudnessMethod>,
    /// Largest sample, dBFS (`null` when silent).
    pub peak_dbfs: Option<f32>,
    /// Seconds of silence (below -60 dBFS) before the sound starts.
    pub leading_silence_seconds: f64,
    /// When the main hit happens, in seconds from the start of the file:
    /// the middle of the loudest 5 ms. Start the sound this much before
    /// the moment it should land. `null` when silent.
    pub hit_seconds: Option<f64>,
    /// True for a song made as a seamless loop (`loop = true`).
    pub loops: bool,
    /// `wav` or `ogg`. For an OGG the facts above are measured on the
    /// file decoded again, as a game hears it.
    pub format: &'static str,
    /// For an OGG: its quality, 0 to 10 (as `oggenc -q`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<f32>,
    /// The seed the sound was rendered with.
    pub seed: u64,
    /// `sha256:` and the SHA-256 of the recipe's (or song's) data as JSON,
    /// after defaults are filled in. Comments and layout do not change
    /// it; any value that changes the sound does.
    pub recipe_hash: String,
    /// `sha256:` and the SHA-256 of the audio file itself, as it was
    /// written beside these facts. A file changed by hand no longer
    /// matches it. (An OGG's bytes differ from one computer to another, so
    /// this only says the file is the one these facts describe; whether
    /// the sound changed is `recipe_hash`'s job.)
    pub file_sha256: String,
    /// Where the recipe that made it is kept, when a game keeps it (a
    /// sound sent with `game_send`: `art/step.toml`, from the game's
    /// folder).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipe: Option<String>,
}

/// When the main hit happens: the middle of the loudest 5 ms (all
/// channels), in seconds, rounded to the millisecond. `None` for silence.
pub fn hit_seconds(buffer: &AudioBuffer) -> Option<f64> {
    let ch = usize::from(buffer.channels.max(1));
    let sr = f64::from(buffer.sample_rate);
    let frames = buffer.samples.len() / ch;
    let window = ((sr * 0.005) as usize).max(1);
    if frames == 0 {
        return None;
    }
    // Energy of each frame, then a running sum over the window.
    let energy: Vec<f64> = buffer
        .samples
        .chunks_exact(ch)
        .map(|f| f.iter().map(|x| f64::from(*x) * f64::from(*x)).sum())
        .collect();
    let mut sum: f64 = energy.iter().take(window).sum();
    let (mut best, mut best_at) = (sum, 0usize);
    for start in 1..=frames.saturating_sub(window) {
        sum += energy[start + window - 1] - energy[start - 1];
        if sum > best {
            best = sum;
            best_at = start;
        }
    }
    if best <= 0.0 {
        return None;
    }
    let middle = (best_at + window.min(frames) / 2) as f64 / sr;
    Some((middle * 1000.0).round() / 1000.0)
}

/// The facts about `buffer`, written to `file`, made from a document
/// whose data is `data` (a recipe or a song as JSON).
pub fn info(
    buffer: &AudioBuffer,
    analysis: &duck_echo_analyze::Analysis,
    file: &Path,
    data: &serde_json::Value,
    seed: u64,
    loops: bool,
) -> AssetInfo {
    AssetInfo {
        kind: "duck-echo-asset",
        version: 1,
        file: file
            .file_name()
            .map_or(String::new(), |n| n.to_string_lossy().into_owned()),
        duration_seconds: analysis.duration_seconds,
        frames: analysis.frames,
        sample_rate: analysis.sample_rate,
        channels: analysis.channels,
        loudness_lufs: analysis.loudness_lufs,
        loudness_method: analysis.loudness_method,
        peak_dbfs: analysis.peak_dbfs,
        leading_silence_seconds: analysis.leading_silence_seconds,
        hit_seconds: hit_seconds(buffer),
        loops,
        format: "wav",
        quality: None,
        seed,
        recipe_hash: recipe_hash(data),
        file_sha256: String::new(),
        recipe: None,
    }
}

/// `sha256:` and the SHA-256 of a document's data as JSON (see
/// [`recipe_data`] and [`song_data`]).
pub fn recipe_hash(data: &serde_json::Value) -> String {
    format!("sha256:{}", sha256_hex(data.to_string().as_bytes()))
}

/// A recipe's data for [`info`]'s hash: the recipe as JSON, plus its
/// random ranges (which are not fields of the decoded recipe), so changing
/// a range changes the hash.
pub fn recipe_data(recipe: &duck_echo_core::Recipe) -> serde_json::Value {
    let mut data = serde_json::to_value(recipe).expect("a recipe is JSON");
    if !recipe.ranges.is_empty() {
        data["ranges"] = serde_json::to_value(&recipe.ranges).expect("ranges are JSON");
    }
    data
}

/// A song's data for [`info`]'s hash, with its instruments' ranges.
pub fn song_data(song: &duck_echo_core::Song) -> serde_json::Value {
    let mut data = serde_json::to_value(song).expect("a song is JSON");
    for (name, recipe) in &song.instruments {
        if !recipe.ranges.is_empty() {
            data["instruments"][name] = recipe_data(recipe);
        }
    }
    data
}

/// Where the facts of `wav` go: `<wav>.json`, such as `coin.wav.json`
/// (`coin.json` could be a recipe).
pub fn sidecar_path(wav: &Path) -> PathBuf {
    let mut name = wav.as_os_str().to_owned();
    name.push(".json");
    PathBuf::from(name)
}

/// Write the facts next to their audio file, in one step. The audio file
/// must be written first: its SHA-256 is read from disk here and put in
/// `info.file_sha256`, so the facts always describe the file beside them.
pub fn write_sidecar(audio: &Path, info: &mut AssetInfo) -> Result<PathBuf, ToolError> {
    info.file_sha256 = file_sha256(audio)?;
    let path = sidecar_path(audio);
    let text = serde_json::to_string_pretty(info).expect("plain JSON") + "\n";
    crate::save::write_atomic(&path, text.as_bytes())?;
    Ok(path)
}

/// `sha256:` and the SHA-256 of a file's bytes.
pub fn file_sha256(path: &Path) -> Result<String, ToolError> {
    let bytes = std::fs::read(path).map_err(|e| {
        ToolError::failed(
            crate::io_code(&e),
            format!("cannot read {}: {e}", path.display()),
        )
    })?;
    Ok(format!("sha256:{}", sha256_hex(&bytes)))
}

/// SHA-256 of `bytes`, as 64 lowercase hex digits (FIPS 180-4). Written
/// here so the tools need no hashing crate.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(bytes);
    hash.finish()
}

/// SHA-256 of the file at `path`, read a piece at a time (a model file is
/// far too big to hold in memory), as 64 lowercase hex digits.
pub fn sha256_of_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut piece = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut piece)?;
        if read == 0 {
            return Ok(hash.finish());
        }
        hash.update(&piece[..read]);
    }
}

/// SHA-256, a piece at a time.
pub struct Sha256 {
    h: [u32; 8],
    /// Bytes that do not fill a 64-byte block yet.
    waiting: Vec<u8>,
    /// How many bytes came in all.
    length: u64,
}

impl Default for Sha256 {
    fn default() -> Sha256 {
        Sha256::new()
    }
}

impl Sha256 {
    /// A new hash, with nothing in it.
    pub fn new() -> Sha256 {
        Sha256 {
            h: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            waiting: Vec::with_capacity(64),
            length: 0,
        }
    }

    /// Add `bytes`.
    pub fn update(&mut self, mut bytes: &[u8]) {
        self.length = self.length.wrapping_add(bytes.len() as u64);
        if !self.waiting.is_empty() {
            let take = (64 - self.waiting.len()).min(bytes.len());
            self.waiting.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.waiting.len() < 64 {
                return;
            }
            let block: [u8; 64] = self.waiting[..].try_into().expect("64 bytes");
            self.block(&block);
            self.waiting.clear();
        }
        let mut blocks = bytes.chunks_exact(64);
        for block in &mut blocks {
            self.block(block.try_into().expect("64 bytes"));
        }
        self.waiting.extend_from_slice(blocks.remainder());
    }

    /// The hash of everything added, as 64 lowercase hex digits.
    pub fn finish(mut self) -> String {
        let bit_len = self.length.wrapping_mul(8);
        let mut end = std::mem::take(&mut self.waiting);
        end.push(0x80);
        while end.len() % 64 != 56 {
            end.push(0);
        }
        end.extend_from_slice(&bit_len.to_be_bytes());
        for block in end.chunks_exact(64) {
            self.block(block.try_into().expect("64 bytes"));
        }
        self.h.iter().map(|x| format!("{x:08x}")).collect()
    }

    fn block(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut w = [0u32; 64];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = self.h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [
                t1.wrapping_add(t2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(t1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for (a, b) in self.h.iter_mut().zip(v) {
            *a = a.wrapping_add(b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_in_pieces_is_the_same_as_in_one_go() {
        let bytes: Vec<u8> = (0..10_000u32).map(|i| (i * 31 % 251) as u8).collect();
        for piece in [1, 7, 63, 64, 65, 1000] {
            let mut hash = Sha256::new();
            for part in bytes.chunks(piece) {
                hash.update(part);
            }
            assert_eq!(hash.finish(), sha256_hex(&bytes), "pieces of {piece}");
        }
        let file = std::env::temp_dir().join(format!("duck-echo-sha-{}", std::process::id()));
        std::fs::write(&file, &bytes).unwrap();
        assert_eq!(sha256_of_file(&file).unwrap(), sha256_hex(&bytes));
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn sha256_matches_the_standard_test_vectors() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn the_hit_is_the_loudest_moment_not_the_first_sample() {
        // 10 ms of a quiet start, then a loud click 30 ms in.
        let sr = 44_100;
        let mut samples = vec![0.0f32; sr / 10];
        for s in samples.iter_mut().take(sr / 100) {
            *s = 0.05;
        }
        let at = sr * 30 / 1000;
        for s in samples.iter_mut().skip(at).take(sr / 1000) {
            *s = 0.9;
        }
        let buffer = AudioBuffer {
            sample_rate: sr as u32,
            channels: 1,
            samples,
        };
        let hit = hit_seconds(&buffer).unwrap();
        assert!((hit - 0.0305).abs() < 0.003, "{hit}");
        assert_eq!(
            sidecar_path(Path::new("sfx/coin.wav")),
            Path::new("sfx/coin.wav.json")
        );
    }
}
