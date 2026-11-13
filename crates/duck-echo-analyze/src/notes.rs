//! Reading the pitch of a note from a stretch of sound.
//!
//! The onset detector ([`crate::timeline`]) finds where notes start, but
//! it reads pitch from only 100 ms and against what came before, which is
//! rough. When the place and the expected pitch of a note are already
//! known (a song says where its notes are), this module reads the note
//! itself: the strongest peak of the spectrum in a range of frequencies,
//! to a fraction of a cent for a steady tone.

use crate::fft::fft;

/// A peak in the spectrum of a stretch of sound.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Peak {
    /// Frequency of the peak, Hz.
    pub hz: f64,
    /// Level of a sine wave that would make this peak, linear (1.0 is
    /// full scale).
    pub amplitude: f64,
}

/// Longest stretch read at once, in samples. Longer input is cut to its
/// middle part.
const MAX_WINDOW: usize = 1 << 15;

/// The FFT is this many times longer than the stretch (zero-padded), so
/// neighbouring bins are close together and the peak is found finely.
const PADDING: usize = 4;

/// The strongest peak between `lo_hz` and `hi_hz` in `mono`.
///
/// The stretch is faded in and out (a Hann window) so its cut ends do not
/// smear the spectrum, and the three bins around the largest one are
/// fitted with a parabola to find the peak between bins. `None` when the
/// stretch is too short for the range (under about three cycles of
/// `lo_hz`) or silent there.
pub fn spectrum_peak(mono: &[f32], sample_rate: u32, lo_hz: f64, hi_hz: f64) -> Option<Peak> {
    let all = spectrum(mono, sample_rate)?;
    all.peak(lo_hz, hi_hz)
}

/// The spectrum of one stretch of sound, kept so several ranges can be
/// searched without doing the FFT again.
#[derive(Debug, Clone)]
pub struct Spectrum {
    /// Magnitude of each bin, already scaled so a full-scale sine reads 1.
    magnitude: Vec<f64>,
    /// Hz between neighbouring bins.
    bin_hz: f64,
    /// Samples in the stretch that was read.
    window: usize,
    sample_rate: u32,
}

/// The spectrum of `mono` (its middle part, if it is very long). `None`
/// for fewer than 64 samples.
pub fn spectrum(mono: &[f32], sample_rate: u32) -> Option<Spectrum> {
    let take = mono.len().min(MAX_WINDOW);
    if take < 64 {
        return None;
    }
    let start = (mono.len() - take) / 2;
    let frame = &mono[start..start + take];
    let n = (take * PADDING).next_power_of_two();
    let mut re = vec![0.0f64; n];
    let mut im = vec![0.0f64; n];
    let mut window_sum = 0.0;
    for (i, s) in frame.iter().enumerate() {
        // Hann window: 0 at both ends, 1 in the middle.
        let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / take as f64).cos();
        window_sum += w;
        re[i] = *s as f64 * w;
    }
    fft(&mut re, &mut im);
    // A sine of amplitude A gives a peak of A * window_sum / 2.
    let scale = 2.0 / window_sum;
    let magnitude = re[..n / 2]
        .iter()
        .zip(&im[..n / 2])
        .map(|(r, i)| (r * r + i * i).sqrt() * scale)
        .collect();
    Some(Spectrum {
        magnitude,
        bin_hz: sample_rate as f64 / n as f64,
        window: take,
        sample_rate,
    })
}

impl Spectrum {
    /// The lowest frequency this stretch can tell apart from its
    /// neighbours: about three cycles must fit in it.
    pub fn lowest_hz(&self) -> f64 {
        3.0 * self.sample_rate as f64 / self.window as f64
    }

    /// The strongest peak between `lo_hz` and `hi_hz`. `None` when the
    /// range is empty, below [`Self::lowest_hz`], or silent.
    pub fn peak(&self, lo_hz: f64, hi_hz: f64) -> Option<Peak> {
        if hi_hz < self.lowest_hz() {
            return None;
        }
        let lo = ((lo_hz / self.bin_hz).ceil() as usize).max(1);
        let hi =
            ((hi_hz / self.bin_hz).floor() as usize).min(self.magnitude.len().saturating_sub(2));
        if lo > hi {
            return None;
        }
        let (bin, &top) = self.magnitude[lo..=hi]
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, m)| (i + lo, m))?;
        if top <= 1e-9 {
            return None;
        }
        // Fit a parabola through the logs of the three bins around the
        // top one; its tip is the true peak.
        let (a, b, c) = (
            self.magnitude[bin - 1].max(1e-12).ln(),
            top.ln(),
            self.magnitude[bin + 1].max(1e-12).ln(),
        );
        let bend = a - 2.0 * b + c;
        let shift = if bend < 0.0 {
            (0.5 * (a - c) / bend).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        Some(Peak {
            hz: (bin as f64 + shift) * self.bin_hz,
            amplitude: (b - 0.25 * (a - c) * shift).exp(),
        })
    }

    /// True when `peak` (found with [`Self::peak`] in `lo_hz..=hi_hz`) is
    /// a real bump inside the range, not just the side of a stronger peak
    /// that sits outside it.
    pub fn is_inside(&self, peak: &Peak, lo_hz: f64, hi_hz: f64) -> bool {
        let edge = 1.5 * self.bin_hz;
        peak.hz > lo_hz + edge && peak.hz < hi_hz - edge
    }
}

/// How far `heard` is from `written`, in cents (100 cents is a semitone).
pub fn cents(heard: f64, written: f64) -> f64 {
    1200.0 * (heard / written).log2()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f64, amplitude: f64, seconds: f64) -> Vec<f32> {
        (0..(seconds * 44_100.0) as usize)
            .map(|i| (amplitude * (std::f64::consts::TAU * hz * i as f64 / 44_100.0).sin()) as f32)
            .collect()
    }

    #[test]
    fn reads_a_steady_tone_to_a_fraction_of_a_cent() {
        for hz in [41.2, 65.41, 261.63, 440.0, 1046.5, 4186.0] {
            let tone = sine(hz, 0.5, 0.5);
            let peak = spectrum_peak(&tone, 44_100, hz * 0.9, hz * 1.1).unwrap();
            assert!(cents(peak.hz, hz).abs() < 0.5, "{hz}: read {}", peak.hz);
            assert!(
                (peak.amplitude - 0.5).abs() < 0.01,
                "{hz}: {}",
                peak.amplitude
            );
        }
    }

    #[test]
    fn picks_the_strongest_of_two_and_knows_silence() {
        let mut two = sine(440.0, 0.2, 0.3);
        for (a, b) in two.iter_mut().zip(sine(660.0, 0.6, 0.3)) {
            *a += b;
        }
        let peak = spectrum_peak(&two, 44_100, 100.0, 2000.0).unwrap();
        assert!(cents(peak.hz, 660.0).abs() < 1.0);
        // Looking only near 440 finds the quieter tone.
        let near = spectrum_peak(&two, 44_100, 420.0, 460.0).unwrap();
        assert!(cents(near.hz, 440.0).abs() < 1.0);
        assert!((near.amplitude - 0.2).abs() < 0.01);

        assert!(spectrum_peak(&vec![0.0; 4410], 44_100, 100.0, 2000.0).is_none());
        // Too short for a low note: 20 ms holds one cycle of 50 Hz.
        assert!(spectrum_peak(&sine(50.0, 0.5, 0.02), 44_100, 45.0, 55.0).is_none());
    }
}
