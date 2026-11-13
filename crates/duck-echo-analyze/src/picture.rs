//! A small picture of a sound, for agents that can read images: level over
//! time on top, a spectrogram below, on one time axis.
//!
//! Numbers describe pitch and loudness well but not texture: grit, a noisy
//! tail, harmonics that shimmer. A spectrogram shows those at a glance. The
//! picture is drawn into a palette image and encoded by [`crate::png`], so
//! it needs no image library. Its axes are fixed (see [`PictureInfo`]) so
//! two pictures can be compared by eye:
//!
//! * **Level panel** (top): for each pixel column, the peak level (light
//!   blue) and RMS level (dark blue) of all channels, from -60 dBFS at the
//!   bottom to 0 dBFS at the top, gridlines at -20 and -40. Red ticks at the
//!   top mark the analysis `onsets`.
//! * **Spectrogram** (bottom): the mean of the channels, 23 ms Hann frames
//!   (the loudest frame per column when a column spans several), on a log
//!   frequency axis from 40 Hz to 20 kHz (or half the sample rate). Colour
//!   runs from black at -90 dB through purple and orange to pale yellow at
//!   0 dB, where 0 dB is a full-scale sine.

use crate::features::fft_size;
use crate::fft::power_spectrum;
use crate::png::{encode, Indexed};
use serde::{Deserialize, Serialize};

/// Picture width in pixels.
pub const WIDTH: u32 = 480;
const LEFT: usize = 34;
const RIGHT: usize = 8;
const TOP: usize = 6;
const LEVEL_H: usize = 64;
const GAP: usize = 8;
const SPEC_H: usize = 160;
const BOTTOM: usize = 16;
const SPEC_TOP: usize = TOP + LEVEL_H + GAP;
/// Picture height in pixels.
pub const HEIGHT: u32 = (SPEC_TOP + SPEC_H + BOTTOM) as u32;
const PLOT_W: usize = WIDTH as usize - LEFT - RIGHT;

/// Level panel range, dBFS.
pub const LEVEL_RANGE_DBFS: [f32; 2] = [-60.0, 0.0];
/// Spectrogram colour range, dB relative to a full-scale sine.
pub const SPECTROGRAM_RANGE_DBFS: [f32; 2] = [-90.0, 0.0];
/// Lowest frequency shown.
pub const MIN_HZ: f32 = 40.0;
/// Highest frequency shown, unless half the sample rate is lower.
pub const MAX_HZ: f32 = 20_000.0;

/// Colours 0..RAMP are the spectrogram ramp; the rest are fixed.
const RAMP: usize = 192;
const BG: u8 = 192;
const GRID: u8 = 193;
const AXIS: u8 = 194;
const TEXT: u8 = 195;
const PEAK: u8 = 196;
const RMS: u8 = 197;
const ONSET: u8 = 198;
const PANEL: u8 = 199;

/// What a picture shows, for reading it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PictureInfo {
    /// Always `image/png`.
    pub mime_type: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The time axis runs from 0 to this many seconds (the whole sound),
    /// left to right, shared by both panels.
    pub seconds: f64,
    /// Level panel range, bottom to top, dBFS.
    pub level_dbfs: [f32; 2],
    /// Spectrogram frequency range, bottom to top, Hz (log scale).
    pub spectrogram_hz: [f32; 2],
    /// Spectrogram colour range, black to pale yellow, dB relative to a
    /// full-scale sine.
    pub spectrogram_dbfs: [f32; 2],
    /// How to read it, in one sentence.
    pub layout: String,
    /// For a song: the time of each bar line drawn, in seconds. Empty for
    /// anything else.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bar_lines: Vec<f64>,
}

/// A picture and its PNG bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    /// What it shows.
    pub info: PictureInfo,
    /// The PNG file.
    pub png: Vec<u8>,
}

/// Draw interleaved `samples`, marking `onsets` (seconds).
pub fn draw(samples: &[f32], sample_rate: u32, channels: u16, onsets: &[f64]) -> Picture {
    draw_with_bars(samples, sample_rate, channels, onsets, &[])
}

/// [`draw`], with a thin vertical line at each time in `bar_lines`
/// (seconds), so a song's picture can be read against its bars. With no
/// bar lines the picture is exactly what [`draw`] makes.
pub fn draw_with_bars(
    samples: &[f32],
    sample_rate: u32,
    channels: u16,
    onsets: &[f64],
    bar_lines: &[f64],
) -> Picture {
    let ch = channels.max(1) as usize;
    let frames = samples.len() / ch;
    let seconds = frames as f64 / sample_rate as f64;
    let max_hz = MAX_HZ.min(sample_rate as f32 / 2.0);
    let mut c = Canvas::new();

    // Level panel.
    c.fill(LEFT, TOP, PLOT_W, LEVEL_H, PANEL);
    for db in [-20.0, -40.0] {
        c.fill(LEFT, level_y(db), PLOT_W, 1, GRID);
    }
    // Bar lines go under the level curves, so the curves stay readable.
    let bar_x = |t: f64| LEFT + ((t / seconds * PLOT_W as f64) as usize).min(PLOT_W - 1);
    let drawn_bars: Vec<f64> = bar_lines
        .iter()
        .copied()
        .filter(|t| seconds > 0.0 && *t >= 0.0 && *t < seconds)
        .collect();
    for &t in &drawn_bars {
        c.fill(bar_x(t), TOP, 1, LEVEL_H, GRID);
    }
    for x in 0..PLOT_W {
        let (a, b) = column(x, frames);
        if a >= frames {
            continue;
        }
        let span = &samples[a * ch..b * ch];
        let peak = span.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let rms = (span.iter().map(|&s| (s as f64) * (s as f64)).sum::<f64>()
            / span.len().max(1) as f64)
            .sqrt() as f32;
        for (level, colour) in [(peak, PEAK), (rms, RMS)] {
            if level > 0.0 {
                let y = level_y(20.0 * level.log10());
                c.fill(LEFT + x, y, 1, TOP + LEVEL_H - y, colour);
            }
        }
    }
    if seconds > 0.0 {
        for &t in onsets {
            let x = ((t / seconds * PLOT_W as f64) as usize).min(PLOT_W - 2);
            c.fill(LEFT + x, TOP, 2, 8, ONSET);
        }
    }

    // Spectrogram.
    let mono: Vec<f32> = if ch == 1 {
        samples.to_vec()
    } else {
        samples
            .chunks_exact(ch)
            .map(|f| f.iter().sum::<f32>() / ch as f32)
            .collect()
    };
    let n = (fft_size(sample_rate) / 2).max(256);
    let bin_hz = sample_rate as f32 / n as f32;
    let full_scale_db = 20.0 * (n as f64 / 4.0).log10();
    let rows: Vec<(usize, usize)> = (0..SPEC_H)
        .map(|y| {
            let hi = freq_at(y as f32, max_hz);
            let lo = freq_at(y as f32 + 1.0, max_hz);
            let a = ((lo / bin_hz).floor() as usize).max(1);
            let b = ((hi / bin_hz).ceil() as usize).clamp(a, n / 2);
            (a, b)
        })
        .collect();
    let mut frame = vec![0.0f32; n];
    for x in 0..PLOT_W {
        let (a, b) = column(x, frames);
        if a >= frames {
            continue;
        }
        let len = b - a;
        let count = len.div_ceil(n / 2).clamp(1, 6);
        let mut power = vec![0.0f64; n / 2 + 1];
        for i in 0..count {
            let centre = a + (2 * i + 1) * len / (2 * count);
            let start = centre as isize - (n / 2) as isize;
            for (j, v) in frame.iter_mut().enumerate() {
                let k = start + j as isize;
                *v = if k >= 0 && (k as usize) < mono.len() {
                    mono[k as usize]
                } else {
                    0.0
                };
            }
            for (m, p) in power.iter_mut().zip(power_spectrum(&frame, n)) {
                *m = m.max(p);
            }
        }
        for (y, &(ka, kb)) in rows.iter().enumerate() {
            let p = power[ka..=kb].iter().copied().fold(0.0, f64::max);
            let db = if p > 0.0 {
                (10.0 * p.log10() - full_scale_db) as f32
            } else {
                f32::NEG_INFINITY
            };
            let [floor, top] = SPECTROGRAM_RANGE_DBFS;
            let t = ((db - floor) / (top - floor)).clamp(0.0, 1.0);
            c.set(
                LEFT + x,
                SPEC_TOP + y,
                (t * (RAMP - 1) as f32).round() as u8,
            );
        }
    }

    // Bar lines over the spectrogram are dotted (2 pixels on, 6 off), so
    // they do not hide what is under them.
    for &t in &drawn_bars {
        for y in (0..SPEC_H).step_by(8) {
            c.fill(bar_x(t), SPEC_TOP + y, 1, 2, GRID);
        }
    }

    // Frames, ticks and labels.
    c.outline(LEFT, TOP, PLOT_W, LEVEL_H, AXIS);
    c.outline(LEFT, SPEC_TOP, PLOT_W, SPEC_H, AXIS);
    for (db, label) in [(0.0, "0dB"), (-20.0, "-20"), (-40.0, "-40")] {
        let y = level_y(db);
        c.text_right(LEFT - 4, y.saturating_sub(3).max(1), label);
    }
    for (hz, label) in [
        (50.0, ""),
        (100.0, "100"),
        (200.0, ""),
        (500.0, ""),
        (1000.0, "1k"),
        (2000.0, ""),
        (5000.0, ""),
        (10_000.0, "10k"),
        (20_000.0, ""),
    ] {
        if hz < MIN_HZ || hz > max_hz {
            continue;
        }
        let y = SPEC_TOP + freq_y(hz, max_hz);
        let long = !label.is_empty();
        c.fill(
            LEFT - if long { 3 } else { 2 },
            y,
            if long { 3 } else { 2 },
            1,
            AXIS,
        );
        if long {
            c.text_right(LEFT - 5, y.saturating_sub(3), label);
        }
    }
    if seconds > 0.0 {
        let step = nice_step(seconds);
        let decimals = (-step.log10().floor()).max(0.0) as usize;
        let mut i = 0;
        loop {
            let t = i as f64 * step;
            if t > seconds + 1e-9 {
                break;
            }
            let x = LEFT + ((t / seconds * PLOT_W as f64).round() as usize).min(PLOT_W - 1);
            let y = SPEC_TOP + SPEC_H;
            c.fill(x, y, 1, 3, AXIS);
            let label = format!("{t:.decimals$}");
            let w = text_width(&label);
            let left = x.saturating_sub(w / 2).clamp(1, WIDTH as usize - w - 1);
            c.text(left, y + 5, &label);
            i += 1;
        }
        c.text(WIDTH as usize - RIGHT + 2, SPEC_TOP + SPEC_H + 5, "s");
    }

    let info = PictureInfo {
        mime_type: "image/png".into(),
        width: WIDTH,
        height: HEIGHT,
        seconds,
        level_dbfs: LEVEL_RANGE_DBFS,
        spectrogram_hz: [MIN_HZ, max_hz],
        spectrogram_dbfs: SPECTROGRAM_RANGE_DBFS,
        layout: format!(
            "Top: level over time, peak light blue and RMS dark blue in dBFS, red ticks at onsets. Bottom: spectrogram, log frequency with low notes at the bottom, black quiet to pale yellow loud. Both panels share the time axis in seconds.{}",
            if drawn_bars.is_empty() {
                ""
            } else {
                " Grey vertical lines (dotted over the spectrogram) mark the start of each bar; `bar_lines` gives their times."
            }
        ),
        bar_lines: drawn_bars,
    };
    Picture {
        png: encode(&c.into_image()),
        info,
    }
}

/// Frames covered by pixel column `x`: at least one, so short sounds
/// repeat frames across columns.
fn column(x: usize, frames: usize) -> (usize, usize) {
    let a = x * frames / PLOT_W;
    let b = ((x + 1) * frames / PLOT_W)
        .max(a + 1)
        .min(frames.max(a + 1));
    (a, b)
}

fn level_y(db: f32) -> usize {
    let [floor, top] = LEVEL_RANGE_DBFS;
    let t = ((top - db) / (top - floor)).clamp(0.0, 1.0);
    TOP + (t * (LEVEL_H - 1) as f32).round() as usize
}

/// Frequency at row `y` of the spectrogram (0 is the top edge).
fn freq_at(y: f32, max_hz: f32) -> f32 {
    max_hz * (MIN_HZ / max_hz).powf(y / SPEC_H as f32)
}

/// Row of the spectrogram showing `hz`.
fn freq_y(hz: f32, max_hz: f32) -> usize {
    let t = (max_hz / hz).ln() / (max_hz / MIN_HZ).ln();
    ((t * SPEC_H as f32) as usize).min(SPEC_H - 1)
}

/// A step of 1, 2 or 5 times a power of ten giving about five ticks.
fn nice_step(span: f64) -> f64 {
    let raw = span / 5.0;
    let mag = 10f64.powf(raw.log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * mag)
        .find(|&s| raw <= s)
        .unwrap_or(10.0 * mag)
}

/// The spectrogram ramp and the fixed colours, in palette order.
fn palette() -> Vec<[u8; 3]> {
    // Anchors close to the "inferno" colour map: dark to bright, and still
    // ordered when printed in grey.
    const ANCHORS: [(f32, [f32; 3]); 8] = [
        (0.0, [0.0, 0.0, 4.0]),
        (0.15, [31.0, 12.0, 72.0]),
        (0.3, [85.0, 15.0, 109.0]),
        (0.45, [136.0, 34.0, 106.0]),
        (0.6, [186.0, 54.0, 85.0]),
        (0.75, [227.0, 89.0, 51.0]),
        (0.87, [249.0, 140.0, 10.0]),
        (1.0, [252.0, 255.0, 164.0]),
    ];
    let mut out: Vec<[u8; 3]> = (0..RAMP)
        .map(|i| {
            let t = i as f32 / (RAMP - 1) as f32;
            let k = ANCHORS
                .windows(2)
                .position(|w| t <= w[1].0)
                .unwrap_or(ANCHORS.len() - 2);
            let ((t0, c0), (t1, c1)) = (ANCHORS[k], ANCHORS[k + 1]);
            let f = (t - t0) / (t1 - t0);
            [0, 1, 2].map(|j| (c0[j] + (c1[j] - c0[j]) * f).round() as u8)
        })
        .collect();
    out.extend([
        [255, 255, 255], // BG
        [222, 222, 222], // GRID
        [110, 110, 110], // AXIS
        [25, 25, 25],    // TEXT
        [150, 190, 232], // PEAK
        [36, 86, 160],   // RMS
        [228, 50, 36],   // ONSET
        [246, 246, 246], // PANEL
    ]);
    out
}

struct Canvas {
    pixels: Vec<u8>,
}

impl Canvas {
    fn new() -> Self {
        Self {
            pixels: vec![BG; (WIDTH * HEIGHT) as usize],
        }
    }

    fn set(&mut self, x: usize, y: usize, colour: u8) {
        if x < WIDTH as usize && y < HEIGHT as usize {
            self.pixels[y * WIDTH as usize + x] = colour;
        }
    }

    fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, colour: u8) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.set(xx, yy, colour);
            }
        }
    }

    /// A one-pixel frame just outside the area.
    fn outline(&mut self, x: usize, y: usize, w: usize, h: usize, colour: u8) {
        self.fill(x - 1, y - 1, w + 2, 1, colour);
        self.fill(x - 1, y + h, w + 2, 1, colour);
        self.fill(x - 1, y, 1, h, colour);
        self.fill(x + w, y, 1, h, colour);
    }

    fn text(&mut self, x: usize, y: usize, s: &str) {
        for (i, ch) in s.chars().enumerate() {
            let rows = glyph(ch);
            for (r, bits) in rows.iter().enumerate() {
                for col in 0..5 {
                    if bits & (0x10 >> col) != 0 {
                        self.set(x + i * 6 + col, y + r, TEXT);
                    }
                }
            }
        }
    }

    fn text_right(&mut self, right: usize, y: usize, s: &str) {
        let w = text_width(s);
        self.text(right.saturating_sub(w), y, s);
    }

    fn into_image(self) -> Indexed {
        Indexed {
            width: WIDTH,
            height: HEIGHT,
            palette: palette(),
            pixels: self.pixels,
        }
    }
}

fn text_width(s: &str) -> usize {
    (s.chars().count() * 6).saturating_sub(1)
}

/// 5×7 glyphs for the labels, one byte per row, high bit on the left.
fn glyph(c: char) -> [u8; 7] {
    match c {
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        'k' => [0x10, 0x10, 0x12, 0x14, 0x18, 0x14, 0x12],
        's' => [0x00, 0x00, 0x0E, 0x10, 0x0E, 0x01, 0x1E],
        'd' => [0x01, 0x01, 0x0D, 0x13, 0x11, 0x11, 0x0F],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        _ => [0; 7],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::png::tests::inflate_zlib;

    /// Decode a PNG written by [`encode`] back to palette indices.
    fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut at = 8;
        let (mut w, mut h, mut idat) = (0, 0, Vec::new());
        while at < png.len() {
            let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            let data = &png[at + 8..at + 8 + len];
            match &png[at + 4..at + 8] {
                b"IHDR" => {
                    w = u32::from_be_bytes(data[0..4].try_into().unwrap());
                    h = u32::from_be_bytes(data[4..8].try_into().unwrap());
                }
                b"IDAT" => idat.extend_from_slice(data),
                _ => {}
            }
            at += 12 + len;
        }
        let raw = inflate_zlib(&idat);
        let pixels = raw
            .chunks(w as usize + 1)
            .flat_map(|row| row[1..].to_vec())
            .collect();
        (w, h, pixels)
    }

    #[test]
    fn a_tone_lights_its_row_and_the_level_panel() {
        let sr = 44_100;
        // 0.5 s of silence, then 0.5 s of a 1 kHz sine at -6 dBFS.
        let mut s = vec![0.0f32; sr / 2];
        s.extend(
            (0..sr / 2)
                .map(|i| 0.5 * (std::f32::consts::TAU * 1000.0 * i as f32 / sr as f32).sin()),
        );
        let pic = draw(&s, sr as u32, 1, &[0.5]);
        assert_eq!(pic.info.width, WIDTH);
        assert_eq!(pic.info.spectrogram_hz, [MIN_HZ, MAX_HZ]);
        let (w, h, px) = decode(&pic.png);
        assert_eq!((w, h), (WIDTH, HEIGHT));
        let at = |x: usize, y: usize| px[y * WIDTH as usize + x];
        let late = LEFT + PLOT_W * 3 / 4;
        let early = LEFT + PLOT_W / 4;
        // The 1 kHz row is bright late and dark early.
        let row = SPEC_TOP + freq_y(1000.0, MAX_HZ);
        assert!((at(late, row) as usize) > RAMP * 3 / 4, "{}", at(late, row));
        assert_eq!(at(early, row), 0);
        // A row far from 1 kHz stays dark.
        assert!((at(late, SPEC_TOP + freq_y(100.0, MAX_HZ)) as usize) < RAMP / 3);
        // Level panel: -6 dB peak is near the top late, nothing early.
        assert_eq!(at(late, level_y(-7.0)), PEAK);
        assert_eq!(at(late, level_y(-10.0)), RMS);
        assert_eq!(at(early, level_y(-50.0)), PANEL);
        // The onset tick sits in the middle.
        assert_eq!(at(LEFT + PLOT_W / 2 + 1, TOP + 2), ONSET);
    }

    #[test]
    fn short_and_silent_sounds_draw() {
        for s in [vec![], vec![0.0; 10], vec![0.3; 5]] {
            let pic = draw(&s, 8000, 1, &[]);
            let (w, h, px) = decode(&pic.png);
            assert_eq!((w, h, px.len()), (WIDTH, HEIGHT, (WIDTH * HEIGHT) as usize));
            assert_eq!(pic.info.spectrogram_hz[1], 4000.0);
        }
    }

    #[test]
    fn nice_steps() {
        for (span, step) in [(1.0, 0.2), (0.12, 0.05), (60.0, 20.0), (0.3, 0.1)] {
            assert!((nice_step(span) - step).abs() < 1e-12, "{span}");
        }
    }
}
