//! OGG Vorbis: long music, about 14 times smaller than a WAV
//! (docs/phase3-design.md, section 5).
//!
//! * Writing uses `vorbis_rs`, which builds Xiph's `libvorbis` (with the
//!   aoTuV tuning) from C. Quality is on `oggenc`'s scale, 0 to 10; 4 is
//!   about 105 kbit/s for Duck Echo's music.
//! * Reading uses `lewton`, pure Rust, the decoder Duck Forge uses to
//!   stream music, so Duck Echo checks its OGG files with the same code
//!   that will play them.
//!
//! Two things make an OGG file Duck Echo's own:
//!
//! * The stream's **serial number**, which Ogg players use to tell
//!   streams apart, is normally random. Duck Echo makes it from the
//!   recipe's hash ([`serial_from_hash`]), so the same recipe gives the
//!   same stream.
//! * A Vorbis comment `DUCKECHO_FRAMES=<n>` gives the exact length in
//!   frames. The Ogg pages say where the sound ends too, but `lewton`
//!   only trims to that when the music fills more than one page: for a
//!   file of a second or so it gives a few hundred frames of silence too
//!   many, at the end (the start always lines up). [`decode_ogg`] trims to
//!   the comment, and so must any player that loops the file (Duck Forge),
//!   so a loop joins at the right sample.

use std::io::Cursor;
use std::num::{NonZeroU32, NonZeroU8};
use std::path::Path;

use crate::{AudioBuffer, WavError};

/// The Vorbis comment holding the length in frames.
pub const FRAMES_TAG: &str = "DUCKECHO_FRAMES";

/// The quality Duck Echo uses unless told otherwise (`oggenc -q 4`).
pub const DEFAULT_QUALITY: f32 = 4.0;

/// The lowest and highest quality allowed.
pub const QUALITY_RANGE: std::ops::RangeInclusive<f32> = 0.0..=10.0;

/// How to write an OGG file.
#[derive(Debug, Clone, PartialEq)]
pub struct OggSettings {
    /// Quality, 0 (smallest) to 10 (best), as `oggenc -q`.
    pub quality: f32,
    /// The Ogg stream's serial number (see [`serial_from_hash`]).
    pub serial: i32,
    /// More Vorbis comments, as (name, value), after `DUCKECHO_FRAMES`.
    pub tags: Vec<(String, String)>,
}

/// A serial number from a hash written in hex (such as a `recipe_hash`,
/// with or without `sha256:`): its first eight hex digits.
pub fn serial_from_hash(hash: &str) -> i32 {
    let hex = hash.strip_prefix("sha256:").unwrap_or(hash);
    let digits: String = hex
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(8)
        .collect();
    u32::from_str_radix(&digits, 16).unwrap_or(0) as i32
}

fn format_error(what: impl std::fmt::Display) -> WavError {
    WavError::Format(format!("ogg: {what}"))
}

/// Encode a buffer (mono or stereo) as OGG Vorbis bytes.
pub fn encode_ogg(buf: &AudioBuffer, settings: &OggSettings) -> Result<Vec<u8>, WavError> {
    if !QUALITY_RANGE.contains(&settings.quality) {
        return Err(format_error(format!(
            "quality {} is outside 0..=10",
            settings.quality
        )));
    }
    let rate = NonZeroU32::new(buf.sample_rate).ok_or_else(|| format_error("sample rate is 0"))?;
    let channels = u8::try_from(buf.channels)
        .ok()
        .and_then(NonZeroU8::new)
        .ok_or_else(|| format_error(format!("{} channels", buf.channels)))?;
    let ch = usize::from(channels.get());
    let frames = buf.frames();

    let mut builder = vorbis_rs::VorbisEncoderBuilder::new_with_serial(
        rate,
        channels,
        Vec::new(),
        settings.serial,
    );
    builder.bitrate_management_strategy(vorbis_rs::VorbisBitrateManagementStrategy::QualityVbr {
        // vorbis_rs takes oggenc's quality divided by 10.
        target_quality: settings.quality / 10.0,
    });
    builder
        .comment_tag(FRAMES_TAG, frames.to_string())
        .map_err(format_error)?;
    for (name, value) in &settings.tags {
        builder
            .comment_tag(name.as_str(), value.as_str())
            .map_err(format_error)?;
    }
    let mut encoder = builder.build().map_err(format_error)?;
    // libvorbis wants each channel in a list of its own, a block at a time.
    const BLOCK: usize = 1024;
    let mut start = 0;
    while start < frames {
        let end = (start + BLOCK).min(frames);
        let block: Vec<Vec<f32>> = (0..ch)
            .map(|c| {
                buf.samples[start * ch..end * ch]
                    .iter()
                    .skip(c)
                    .step_by(ch)
                    .copied()
                    .collect()
            })
            .collect();
        encoder.encode_audio_block(&block).map_err(format_error)?;
        start = end;
    }
    encoder.finish().map_err(format_error)
}

/// An OGG file, read.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedOgg {
    /// Its samples, trimmed to `DUCKECHO_FRAMES` when it has that comment.
    pub buffer: AudioBuffer,
    /// What `DUCKECHO_FRAMES` says, if the file has it.
    pub frames_tag: Option<u64>,
    /// How many frames the decoder gave before any trimming.
    pub decoded_frames: usize,
    /// Every Vorbis comment, as (name, value).
    pub tags: Vec<(String, String)>,
}

/// Decode OGG Vorbis bytes into a float buffer.
pub fn decode_ogg(bytes: &[u8]) -> Result<DecodedOgg, WavError> {
    let mut reader =
        lewton::inside_ogg::OggStreamReader::new(Cursor::new(bytes)).map_err(format_error)?;
    let channels = u16::from(reader.ident_hdr.audio_channels);
    let sample_rate = reader.ident_hdr.audio_sample_rate;
    let tags = reader.comment_hdr.comment_list.clone();
    let frames_tag = tags
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(FRAMES_TAG))
        .and_then(|(_, value)| value.trim().parse::<u64>().ok());
    let mut samples = Vec::new();
    while let Some(packet) = reader
        .read_dec_packet_generic::<lewton::samples::InterleavedSamples<f32>>()
        .map_err(format_error)?
    {
        samples.extend_from_slice(&packet.samples);
    }
    let ch = usize::from(channels.max(1));
    let decoded_frames = samples.len() / ch;
    if let Some(n) = frames_tag.and_then(|n| usize::try_from(n).ok()) {
        samples.truncate(n.min(decoded_frames) * ch);
    }
    Ok(DecodedOgg {
        buffer: AudioBuffer {
            sample_rate,
            channels,
            samples,
        },
        frames_tag,
        decoded_frames,
        tags,
    })
}

/// Read an OGG Vorbis file.
pub fn read_ogg(path: &Path) -> Result<DecodedOgg, WavError> {
    let bytes = std::fs::read(path)?;
    decode_ogg(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One second of a stereo chord, left and right different.
    fn chord() -> AudioBuffer {
        let sr = 44_100;
        let samples = (0..sr)
            .flat_map(|i| {
                let t = i as f32 / sr as f32;
                let l = 0.3 * (std::f32::consts::TAU * 220.0 * t).sin();
                let r = 0.3 * (std::f32::consts::TAU * 330.0 * t).sin();
                [l, r]
            })
            .collect();
        AudioBuffer {
            sample_rate: sr as u32,
            channels: 2,
            samples,
        }
    }

    #[test]
    fn a_sound_comes_back_the_same_length_close_and_tagged() {
        let buf = chord();
        let settings = OggSettings {
            quality: DEFAULT_QUALITY,
            serial: serial_from_hash("sha256:0000002a99"),
            tags: vec![("TITLE".into(), "chord".into())],
        };
        let bytes = encode_ogg(&buf, &settings).unwrap();
        // Much smaller than 16-bit PCM (176,400 bytes for this second).
        assert!(bytes.len() < 40_000, "{} bytes", bytes.len());
        assert_eq!(&bytes[..4], b"OggS");
        // The same buffer and settings give the same bytes.
        assert_eq!(bytes, encode_ogg(&buf, &settings).unwrap());

        let back = decode_ogg(&bytes).unwrap();
        // lewton trims the end by the position of the page before the last
        // one; in a file this short all the music is on one page, so it
        // gives a few hundred frames too many, at the end (the start lines
        // up). DUCKECHO_FRAMES trims them.
        assert!(back.decoded_frames >= 44_100, "{}", back.decoded_frames);
        assert_eq!(back.buffer.frames(), 44_100);
        assert_eq!(back.frames_tag, Some(44_100));
        assert_eq!((back.buffer.sample_rate, back.buffer.channels), (44_100, 2));
        assert!(back.tags.contains(&("TITLE".into(), "chord".into())));
        // Lossy, but close: the difference is far below the signal.
        let worst = buf
            .samples
            .iter()
            .zip(&back.buffer.samples)
            .skip(4096)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 0.05, "{worst}");
    }

    #[test]
    fn the_serial_comes_from_the_hash_and_bad_input_is_refused() {
        assert_eq!(serial_from_hash("sha256:0000002a"), 42);
        assert_eq!(serial_from_hash("ffffffff00"), -1);
        let buf = chord();
        let settings = OggSettings {
            quality: 11.0,
            serial: 1,
            tags: Vec::new(),
        };
        assert!(encode_ogg(&buf, &settings).is_err());
        assert!(decode_ogg(b"not an ogg file").is_err());
    }
}
