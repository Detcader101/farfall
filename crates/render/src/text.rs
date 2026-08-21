//! Bitmap text for on-screen readouts (SPEC §6.5, pass: hud).
//!
//! A 3x5 pixel font rasterised on the CPU into a bit mask that the shader
//! samples. Text layout is fiddly, branchy, and exactly the kind of logic that
//! is miserable to debug on a GPU — so it lives here, in plain Rust, under
//! test. The shader's only job is "is this bit set".
//!
//! No font files, no glyph atlas, no texture: the whole typeface is 17 `u8`
//! arrays, which keeps the asset budget at zero (SPEC P2).

/// Font cell, in font pixels.
pub const GLYPH_W: usize = 3;
pub const GLYPH_H: usize = 5;
/// One font pixel of tracking between glyphs.
pub const ADVANCE: usize = GLYPH_W + 1;

/// Bitmap dimensions, in font pixels. 128 columns is ~32 characters; 64 rows
/// is ten text lines at the standard 6-pixel pitch. (At 16 rows, every HUD
/// line below the third was silently clipped — including the altitude and
/// speed readouts, which simply never appeared.)
pub const COLS: usize = 128;
pub const ROWS: usize = 64;
/// `COLS` bits per row, as four `u32`s (a `vec4<u32>` on the GPU).
pub const ROW_WORDS: usize = COLS / 32;

/// Rows top-to-bottom; within a row, bit 0 is the leftmost column.
fn glyph(c: char) -> [u8; GLYPH_H] {
    match c.to_ascii_uppercase() {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b011, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '3' => [0b111, 0b100, 0b111, 0b100, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b100, 0b100],
        '5' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '6' => [0b111, 0b001, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b100, 0b100, 0b100, 0b100],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b100, 0b111],
        'F' => [0b111, 0b001, 0b111, 0b001, 0b001],
        'P' => [0b111, 0b101, 0b111, 0b001, 0b001],
        'S' => [0b111, 0b001, 0b111, 0b100, 0b111],
        'L' => [0b001, 0b001, 0b001, 0b001, 0b111],
        'O' => [0b111, 0b101, 0b101, 0b101, 0b111],
        'W' => [0b101, 0b101, 0b101, 0b111, 0b101],
        'M' => [0b101, 0b111, 0b111, 0b101, 0b101],
        'A' => [0b111, 0b101, 0b111, 0b101, 0b101],
        'X' => [0b101, 0b101, 0b010, 0b101, 0b101],
        'K' => [0b101, 0b011, 0b001, 0b011, 0b101],
        'V' => [0b101, 0b101, 0b101, 0b101, 0b010],
        'N' => [0b101, 0b011, 0b111, 0b101, 0b101],
        'Y' => [0b101, 0b101, 0b010, 0b010, 0b010],
        'C' => [0b111, 0b001, 0b001, 0b001, 0b111],
        'T' => [0b111, 0b010, 0b010, 0b010, 0b010],
        'I' => [0b111, 0b010, 0b010, 0b010, 0b111],
        'D' => [0b011, 0b101, 0b101, 0b101, 0b011],
        'E' => [0b111, 0b001, 0b111, 0b001, 0b111],
        'R' => [0b111, 0b101, 0b011, 0b101, 0b101],
        'U' => [0b101, 0b101, 0b101, 0b101, 0b111],
        'G' => [0b111, 0b001, 0b101, 0b101, 0b111],
        'H' => [0b101, 0b101, 0b111, 0b101, 0b101],
        'B' => [0b011, 0b101, 0b011, 0b101, 0b011],
        '%' => [0b101, 0b100, 0b010, 0b001, 0b101],
        '.' => [0b000, 0b000, 0b000, 0b000, 0b010],
        ':' => [0b000, 0b010, 0b000, 0b010, 0b000],
        '/' => [0b100, 0b100, 0b010, 0b001, 0b001],
        '-' => [0b000, 0b000, 0b111, 0b000, 0b000],
        'J' => [0b100, 0b100, 0b100, 0b101, 0b111],
        'Q' => [0b111, 0b101, 0b101, 0b111, 0b100],
        'Z' => [0b111, 0b100, 0b010, 0b001, 0b111],
        '<' => [0b100, 0b010, 0b001, 0b010, 0b100],
        '>' => [0b001, 0b010, 0b100, 0b010, 0b001],
        '[' => [0b011, 0b001, 0b001, 0b001, 0b011],
        ']' => [0b110, 0b100, 0b100, 0b100, 0b110],
        '+' => [0b000, 0b010, 0b111, 0b010, 0b000],
        '=' => [0b000, 0b111, 0b000, 0b111, 0b000],
        '*' => [0b101, 0b010, 0b111, 0b010, 0b101],
        '_' => [0b000, 0b000, 0b000, 0b000, 0b111],
        _ => [0; GLYPH_H], // space and anything unmapped
    }
}

/// A monochrome text buffer, packed for direct upload as `vec4<u32>` rows.
#[derive(Clone, Copy)]
pub struct TextBitmap {
    pub rows: [[u32; ROW_WORDS]; ROWS],
}

impl Default for TextBitmap {
    fn default() -> Self {
        Self::new()
    }
}

impl TextBitmap {
    pub const fn new() -> Self {
        Self {
            rows: [[0; ROW_WORDS]; ROWS],
        }
    }

    pub fn clear(&mut self) {
        self.rows = [[0; ROW_WORDS]; ROWS];
    }

    /// Set one pixel. Out-of-bounds writes are dropped rather than panicking:
    /// a readout that runs off the edge should clip, not take the frame down.
    fn set(&mut self, x: usize, y: usize) {
        if x < COLS && y < ROWS {
            self.rows[y][x / 32] |= 1u32 << (x % 32);
        }
    }

    pub fn get(&self, x: usize, y: usize) -> bool {
        x < COLS && y < ROWS && self.rows[y][x / 32] & (1u32 << (x % 32)) != 0
    }

    /// Draw `text` with its top-left corner at font-pixel (`x`, `y`).
    /// Returns the x position just past the last glyph.
    pub fn draw(&mut self, x: usize, y: usize, text: &str) -> usize {
        let mut pen = x;
        for c in text.chars() {
            let g = glyph(c);
            for (row, bits) in g.iter().enumerate() {
                for col in 0..GLYPH_W {
                    if bits & (1 << col) != 0 {
                        self.set(pen + col, y + row);
                    }
                }
            }
            pen += ADVANCE;
        }
        pen
    }

    /// Row `y` as GPU-ready words.
    pub fn row_words(&self, y: usize) -> [u32; ROW_WORDS] {
        self.rows[y]
    }

    /// Width and height actually occupied, in font pixels, so the backdrop can
    /// hug the text instead of spanning the whole buffer.
    pub fn used_extent(&self) -> (usize, usize) {
        let (mut w, mut h) = (0, 0);
        for (y, row) in self.rows.iter().enumerate() {
            for (word_idx, word) in row.iter().enumerate() {
                if *word != 0 {
                    let highest = 31 - word.leading_zeros() as usize;
                    w = w.max(word_idx * 32 + highest + 1);
                    h = h.max(y + 1);
                }
            }
        }
        (w, h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_bitmap_has_no_set_pixels() {
        let b = TextBitmap::new();
        for y in 0..ROWS {
            for x in 0..COLS {
                assert!(!b.get(x, y));
            }
        }
    }

    /// '1' must actually look like a 1: a stem with a serif foot.
    #[test]
    fn glyph_shape_is_rendered_at_the_right_place() {
        let mut b = TextBitmap::new();
        b.draw(0, 0, "1");
        // Row 0 of '1' is 0b010 -> only the middle column.
        assert!(!b.get(0, 0));
        assert!(b.get(1, 0));
        assert!(!b.get(2, 0));
        // Row 4 is 0b111 -> the full foot.
        assert!(b.get(0, 4) && b.get(1, 4) && b.get(2, 4));
    }

    #[test]
    fn draw_advances_by_glyph_pitch() {
        let mut b = TextBitmap::new();
        let end = b.draw(0, 0, "88");
        assert_eq!(end, 2 * ADVANCE);
        // Second glyph starts one advance over.
        assert!(b.get(ADVANCE, 0));
        // The tracking column between glyphs stays clear.
        assert!(!b.get(GLYPH_W, 0));
    }

    #[test]
    fn space_and_unknown_chars_are_blank_but_still_advance() {
        let mut b = TextBitmap::new();
        let end = b.draw(0, 0, " ~");
        assert_eq!(end, 2 * ADVANCE);
        for y in 0..ROWS {
            for x in 0..COLS {
                assert!(!b.get(x, y), "blank glyph drew a pixel at {x},{y}");
            }
        }
    }

    /// A readout that overruns the buffer must clip silently — never panic in
    /// the middle of a frame.
    #[test]
    fn drawing_off_the_edge_clips_without_panicking() {
        let mut b = TextBitmap::new();
        b.draw(COLS - 2, ROWS - 1, "888888");
        b.draw(COLS + 50, 0, "8");
        b.draw(0, ROWS + 5, "8");
        assert!(b.get(COLS - 2, ROWS - 1));
    }

    #[test]
    fn lowercase_matches_uppercase() {
        let (mut a, mut b) = (TextBitmap::new(), TextBitmap::new());
        a.draw(0, 0, "fps");
        b.draw(0, 0, "FPS");
        assert_eq!(a.rows, b.rows);
    }

    #[test]
    fn used_extent_hugs_the_text() {
        let mut b = TextBitmap::new();
        assert_eq!(b.used_extent(), (0, 0), "empty bitmap occupies nothing");
        b.draw(0, 0, "8");
        assert_eq!(b.used_extent(), (GLYPH_W, GLYPH_H));
        b.draw(0, 6, "88");
        // Widest row wins; height reaches the last occupied row.
        assert_eq!(b.used_extent(), (ADVANCE + GLYPH_W, 6 + GLYPH_H));
    }

    #[test]
    fn used_extent_spans_word_boundaries() {
        let mut b = TextBitmap::new();
        b.set(40, 2);
        assert_eq!(b.used_extent(), (41, 3));
    }

    #[test]
    fn clear_resets_everything() {
        let mut b = TextBitmap::new();
        b.draw(0, 0, "123");
        b.clear();
        assert_eq!(b.rows, TextBitmap::new().rows);
    }

    /// Packing must put x=32 in the second word, not overflow the first.
    #[test]
    fn bit_packing_crosses_word_boundaries() {
        let mut b = TextBitmap::new();
        b.set(31, 0);
        b.set(32, 0);
        assert_eq!(b.rows[0][0], 1 << 31);
        assert_eq!(b.rows[0][1], 1);
        assert!(b.get(31, 0) && b.get(32, 0));
    }

    /// Every character used by the live readout must have a real glyph, or the
    /// HUD silently renders blanks.
    #[test]
    fn readout_charset_is_covered() {
        for c in "0123456789FPSLOWMSAAVNCXKY%./:-".chars() {
            assert_ne!(glyph(c), [0; GLYPH_H], "no glyph for {c:?}");
        }
    }
}
