//! A board to lay pictures and words on: the contact sheet of a sound set
//! (every sound's picture with its name) and the picture of a mix (its
//! level over time with a marker and a name for each cue).
//!
//! It uses the palette of [`crate::picture`], so a sound's picture is
//! copied onto the board pixel for pixel, and a small 5×7 font with the
//! capital letters, digits and the signs a file name or a number needs.
//! Small letters are drawn as capitals. The font of the sound pictures is
//! left as it is, so those pictures do not change.

use crate::picture::{palette, BG, GRID, ONSET, PANEL, PEAK, RMS, TEXT};
use crate::png::{encode, Indexed};

/// The colours a board can use, by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    /// White, the background.
    Paper,
    /// Near black, for words.
    Text,
    /// Light grey, for lines that only guide the eye.
    Grid,
    /// Very light grey, for a panel behind a curve.
    Panel,
    /// Light blue, a peak level.
    Peak,
    /// Dark blue, an RMS level or a calm marker.
    Rms,
    /// Red, for a problem.
    Problem,
}

impl Ink {
    fn index(self) -> u8 {
        match self {
            Ink::Paper => BG,
            Ink::Text => TEXT,
            Ink::Grid => GRID,
            Ink::Panel => PANEL,
            Ink::Peak => PEAK,
            Ink::Rms => RMS,
            Ink::Problem => ONSET,
        }
    }
}

/// Width of one character in pixels at size 1 (5 and a gap).
pub const CHAR_W: usize = 6;
/// Height of one line of text in pixels at size 1.
pub const CHAR_H: usize = 7;

/// A picture being drawn.
#[derive(Debug, Clone)]
pub struct Board {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

impl Board {
    /// A white board.
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![BG; width * height],
        }
    }

    /// Width in pixels.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> usize {
        self.height
    }

    /// The colour index at a pixel (for tests).
    pub fn at(&self, x: usize, y: usize) -> u8 {
        self.pixels[y * self.width + x]
    }

    /// The palette index of an ink (for tests).
    pub fn ink(ink: Ink) -> u8 {
        ink.index()
    }

    fn set(&mut self, x: usize, y: usize, colour: u8) {
        if x < self.width && y < self.height {
            self.pixels[y * self.width + x] = colour;
        }
    }

    /// Fill a rectangle; the parts off the board are left out.
    pub fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, ink: Ink) {
        let colour = ink.index();
        for yy in y..(y + h).min(self.height) {
            for xx in x..(x + w).min(self.width) {
                self.pixels[yy * self.width + xx] = colour;
            }
        }
    }

    /// A one-pixel frame around a rectangle.
    pub fn frame(&mut self, x: usize, y: usize, w: usize, h: usize, ink: Ink) {
        self.fill(x, y, w, 1, ink);
        self.fill(x, y + h.saturating_sub(1), w, 1, ink);
        self.fill(x, y, 1, h, ink);
        self.fill(x + w.saturating_sub(1), y, 1, h, ink);
    }

    /// Copy a picture made with the same palette (a sound's picture) with
    /// its top left corner at `(x, y)`.
    pub fn place(&mut self, image: &Indexed, x: usize, y: usize) {
        let w = image.width as usize;
        for (row, line) in image.pixels.chunks_exact(w.max(1)).enumerate() {
            for (col, &p) in line.iter().enumerate() {
                self.set(x + col, y + row, p);
            }
        }
    }

    /// Write `text` with its top left corner at `(x, y)`, each pixel of
    /// the font `size` pixels wide. Characters past `max_chars` are cut
    /// and the last one shown becomes `~`.
    pub fn text(
        &mut self,
        x: usize,
        y: usize,
        text: &str,
        size: usize,
        ink: Ink,
        max_chars: usize,
    ) {
        let colour = ink.index();
        let chars: Vec<char> = text.chars().collect();
        let cut = chars.len() > max_chars;
        for (i, &c) in chars.iter().take(max_chars).enumerate() {
            let c = if cut && i + 1 == max_chars { '~' } else { c };
            for (r, bits) in glyph(c).iter().enumerate() {
                for col in 0..5 {
                    if bits & (0x10 >> col) != 0 {
                        for dy in 0..size {
                            for dx in 0..size {
                                self.set(
                                    x + (i * CHAR_W + col) * size + dx,
                                    y + r * size + dy,
                                    colour,
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// The board as a palette image.
    pub fn into_image(self) -> Indexed {
        Indexed {
            width: self.width as u32,
            height: self.height as u32,
            palette: palette(),
            pixels: self.pixels,
        }
    }

    /// The board as PNG bytes.
    pub fn into_png(self) -> Vec<u8> {
        encode(&self.into_image())
    }
}

/// How many pixels wide `text` is at `size`.
pub fn text_width(text: &str, size: usize) -> usize {
    (text.chars().count() * CHAR_W).saturating_sub(1) * size
}

/// 5×7 glyphs, one byte per row, high bit (of five) on the left. Small
/// letters are drawn as capitals; a character with no glyph is a box.
fn glyph(c: char) -> [u8; 7] {
    match c.to_ascii_uppercase() {
        ' ' => [0; 7],
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x11, 0x0A, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
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
        ',' => [0x00, 0x00, 0x00, 0x00, 0x0C, 0x04, 0x08],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        '+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        '_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1F],
        ':' => [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00],
        '/' => [0x00, 0x01, 0x02, 0x04, 0x08, 0x10, 0x00],
        '%' => [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03],
        '~' => [0x00, 0x00, 0x08, 0x15, 0x02, 0x00, 0x00],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        '!' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04],
        '?' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
        '=' => [0x00, 0x00, 0x1F, 0x00, 0x1F, 0x00, 0x00],
        '<' => [0x02, 0x04, 0x08, 0x10, 0x08, 0x04, 0x02],
        '>' => [0x08, 0x04, 0x02, 0x01, 0x02, 0x04, 0x08],
        '#' => [0x0A, 0x0A, 0x1F, 0x0A, 0x1F, 0x0A, 0x0A],
        '\'' => [0x04, 0x04, 0x08, 0x00, 0x00, 0x00, 0x00],
        ';' => [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x04, 0x08],
        '"' => [0x0A, 0x0A, 0x0A, 0x00, 0x00, 0x00, 0x00],
        '[' => [0x0E, 0x08, 0x08, 0x08, 0x08, 0x08, 0x0E],
        ']' => [0x0E, 0x02, 0x02, 0x02, 0x02, 0x02, 0x0E],
        '*' => [0x00, 0x04, 0x15, 0x0E, 0x15, 0x04, 0x00],
        '&' => [0x0C, 0x12, 0x14, 0x08, 0x15, 0x12, 0x0D],
        _ => [0x1F, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1F],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_and_pictures_land_where_asked() {
        let mut b = Board::new(100, 40);
        b.text(2, 2, "Hi", 2, Ink::Text, 10);
        // The H's left stroke, two pixels wide at size 2.
        assert_eq!(b.at(2, 2), Board::ink(Ink::Text));
        assert_eq!(b.at(3, 15), Board::ink(Ink::Text));
        assert_eq!(b.at(4, 2), Board::ink(Ink::Paper));
        let pic = crate::picture::draw_image(&[0.0; 100], 8000, 1, &[], &[]).1;
        let mut big = Board::new(pic.width as usize + 10, pic.height as usize + 10);
        big.place(&pic, 10, 10);
        assert_eq!(big.at(10, 10), pic.pixels[0]);
        assert_eq!(big.at(0, 0), Board::ink(Ink::Paper));
        // Too long a name is cut with a ~.
        let mut c = Board::new(60, 10);
        c.text(0, 0, "abcdefghij", 1, Ink::Problem, 4);
        assert_eq!(c.at(4 * CHAR_W, 3), Board::ink(Ink::Paper));
        assert_eq!(text_width("abc", 2), 34);
        assert!(!b.into_png().is_empty());
    }
}
