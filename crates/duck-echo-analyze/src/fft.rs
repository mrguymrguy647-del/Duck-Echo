//! A small iterative radix-2 FFT for measurement. Not on the audio path, so
//! it uses `f64` and platform math freely.

use std::f64::consts::TAU;

/// In-place forward FFT of `re` + i·`im`. Both slices must have the same
/// power-of-two length.
pub fn fft(re: &mut [f64], im: &mut [f64]) {
    transform(re, im, -1.0);
}

/// In-place inverse FFT, scaled by `1/n` so `ifft(fft(x)) == x`.
pub fn ifft(re: &mut [f64], im: &mut [f64]) {
    transform(re, im, 1.0);
    let scale = 1.0 / re.len() as f64;
    re.iter_mut().for_each(|v| *v *= scale);
    im.iter_mut().for_each(|v| *v *= scale);
}

fn transform(re: &mut [f64], im: &mut [f64], sign: f64) {
    let n = re.len();
    assert_eq!(n, im.len(), "fft: real and imaginary lengths differ");
    assert!(n.is_power_of_two(), "fft: length {n} is not a power of two");
    if n < 2 {
        return;
    }
    // Bit-reversal permutation.
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if j > i {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let half = len / 2;
        let step = sign * TAU / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..half {
                let (s, c) = (step * k as f64).sin_cos();
                let (a, b) = (start + k, start + k + half);
                let tr = re[b] * c - im[b] * s;
                let ti = re[b] * s + im[b] * c;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len *= 2;
    }
}

/// Power spectrum (`|X[k]|²` for `k` in `0..=n/2`) of `frame`, after a Hann
/// window, zero-padded to `n`.
pub fn power_spectrum(frame: &[f32], n: usize) -> Vec<f64> {
    let mut re = vec![0.0; n];
    let mut im = vec![0.0; n];
    let len = frame.len().min(n);
    for (i, (r, &x)) in re.iter_mut().zip(frame).take(len).enumerate() {
        let w = 0.5 - 0.5 * (TAU * i as f64 / len as f64).cos();
        *r = x as f64 * w;
    }
    fft(&mut re, &mut im);
    (0..=n / 2).map(|k| re[k] * re[k] + im[k] * im[k]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_known_bin() {
        let n = 64;
        let orig: Vec<f64> = (0..n)
            .map(|i| (TAU * 5.0 * i as f64 / n as f64).cos())
            .collect();
        let mut re = orig.clone();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im);
        // A cosine at bin 5 puts n/2 in bins 5 and n-5.
        assert!((re[5] - n as f64 / 2.0).abs() < 1e-9);
        assert!((re[n - 5] - n as f64 / 2.0).abs() < 1e-9);
        assert!(re[7].abs() < 1e-9 && im[7].abs() < 1e-9);
        ifft(&mut re, &mut im);
        for (a, b) in re.iter().zip(&orig) {
            assert!((a - b).abs() < 1e-12);
        }
    }
}
