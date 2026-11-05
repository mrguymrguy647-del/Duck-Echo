//! Audio buffer type and WAV I/O for Duck Echo.
//!
//! Samples are `f32` in -1..=1, interleaved when there is more than one
//! channel. Recipes render mono or stereo; the channel count travels with
//! the buffer.

use std::fmt;

/// A block of audio samples.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioBuffer {
    /// Samples per second.
    pub sample_rate: u32,
    /// Number of interleaved channels (1 = mono).
    pub channels: u16,
    /// Interleaved samples in -1..=1.
    pub samples: Vec<f32>,
}

impl AudioBuffer {
    /// An empty mono buffer.
    pub fn mono(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            channels: 1,
            samples: Vec::new(),
        }
    }

    /// Number of frames (samples per channel).
    pub fn frames(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.samples.len() / self.channels as usize
        }
    }

    /// Length in seconds.
    pub fn duration_seconds(&self) -> f64 {
        self.frames() as f64 / self.sample_rate as f64
    }

    /// Mono view: the buffer itself when mono, else the mean of channels.
    pub fn to_mono(&self) -> Vec<f32> {
        if self.channels <= 1 {
            return self.samples.clone();
        }
        let ch = self.channels as usize;
        self.samples
            .chunks_exact(ch)
            .map(|frame| frame.iter().sum::<f32>() / ch as f32)
            .collect()
    }
}

/// Errors from WAV I/O.
#[derive(Debug)]
pub enum WavError {
    /// The file could not be read or written.
    Io(std::io::Error),
    /// The file is not a WAV we can decode.
    Format(String),
}

impl fmt::Display for WavError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WavError::Io(e) => write!(f, "io error: {e}"),
            WavError::Format(s) => write!(f, "wav format error: {s}"),
        }
    }
}

impl std::error::Error for WavError {}

impl From<std::io::Error> for WavError {
    fn from(e: std::io::Error) -> Self {
        WavError::Io(e)
    }
}

/// Output sample format for WAV files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WavFormat {
    /// 16-bit signed PCM. The default; what game engines expect.
    #[default]
    Pcm16,
    /// 24-bit signed PCM.
    Pcm24,
    /// 32-bit IEEE float.
    Float32,
}

#[cfg(feature = "wav")]
mod wav {
    use super::*;
    use std::path::Path;

    impl From<hound::Error> for WavError {
        fn from(e: hound::Error) -> Self {
            match e {
                hound::Error::IoError(io) => WavError::Io(io),
                other => WavError::Format(other.to_string()),
            }
        }
    }

    /// Encode a buffer as WAV bytes. Deterministic: same buffer, same bytes.
    pub fn encode_wav(buf: &AudioBuffer, format: WavFormat) -> Result<Vec<u8>, WavError> {
        let (bits, sample_format) = match format {
            WavFormat::Pcm16 => (16, hound::SampleFormat::Int),
            WavFormat::Pcm24 => (24, hound::SampleFormat::Int),
            WavFormat::Float32 => (32, hound::SampleFormat::Float),
        };
        let spec = hound::WavSpec {
            channels: buf.channels,
            sample_rate: buf.sample_rate,
            bits_per_sample: bits,
            sample_format,
        };
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut w = hound::WavWriter::new(&mut cursor, spec)?;
            match format {
                WavFormat::Pcm16 => {
                    for &s in &buf.samples {
                        w.write_sample(to_int(s, 16) as i16)?;
                    }
                }
                WavFormat::Pcm24 => {
                    for &s in &buf.samples {
                        w.write_sample(to_int(s, 24))?;
                    }
                }
                WavFormat::Float32 => {
                    for &s in &buf.samples {
                        w.write_sample(s.clamp(-1.0, 1.0))?;
                    }
                }
            }
            w.finalize()?;
        }
        Ok(cursor.into_inner())
    }

    /// Write a buffer to a WAV file.
    pub fn write_wav(path: &Path, buf: &AudioBuffer, format: WavFormat) -> Result<(), WavError> {
        let bytes = encode_wav(buf, format)?;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    /// Decode WAV bytes into a float buffer.
    pub fn decode_wav(bytes: &[u8]) -> Result<AudioBuffer, WavError> {
        let mut r = hound::WavReader::new(std::io::Cursor::new(bytes))?;
        let spec = r.spec();
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => r.samples::<f32>().collect::<Result<_, _>>()?,
            hound::SampleFormat::Int => {
                let scale = 1.0 / (1u32 << (spec.bits_per_sample - 1)) as f32;
                r.samples::<i32>()
                    .map(|s| s.map(|v| v as f32 * scale))
                    .collect::<Result<_, _>>()?
            }
        };
        Ok(AudioBuffer {
            sample_rate: spec.sample_rate,
            channels: spec.channels,
            samples,
        })
    }

    /// Read a WAV file into a float buffer.
    pub fn read_wav(path: &Path) -> Result<AudioBuffer, WavError> {
        let bytes = std::fs::read(path)?;
        decode_wav(&bytes)
    }

    fn to_int(s: f32, bits: u32) -> i32 {
        let max = ((1i64 << (bits - 1)) - 1) as f32;
        (s.clamp(-1.0, 1.0) * max).round() as i32
    }
}

#[cfg(feature = "wav")]
pub use wav::{decode_wav, encode_wav, read_wav, write_wav};
