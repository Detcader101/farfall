//! Bitmap text for on-screen readouts (SPEC §6.5, pass: hud).
//!
//! A 5x7 pixel font rasterised on the CPU into a bit mask that the shader
//! samples. Text layout is fiddly, branchy, and exactly the kind of logic
//! that is miserable to debug on a GPU — so it lives here, in plain Rust,
//! under test. The shader's only job is "how much of this pixel is lit"
//! (it box-filters the bits, so the text is anti-aliased at any scale).
//!
//! No font files, no glyph atlas, no texture: the whole typeface is a
//! table of seven-row strings, which keeps the asset budget at zero (SPEC
//! P2). The letterforms are the classic 5x7 dot-matrix set with the
//! ambiguities resolved: a slashed 0 against O, a serifed 1 against I, a
//! square-cornered 5 against S, a flat-sided B against 8.

/// Font cell, in font pixels.
pub const GLYPH_W: usize = 5;
pub const GLYPH_H: usize = 7;
/// One font pixel of tracking between glyphs.
pub const ADVANCE: usize = GLYPH_W + 1;
/// Line pitch: a glyph and two font pixels of leading. Every block of text
/// lays its lines out on this pitch, and every click on a panel row is
/// divided by it.
pub const LINE: usize = GLYPH_H + 2;

/// Bitmap dimensions, in font pixels. 384 columns is 64 characters; 180
/// rows is twenty text lines at the 9-pixel pitch. (The old 128x96 bitmap
/// clipped the KEYS page's key names and the landing line; a line at row
/// y needs `y + GLYPH_H` rows, and the bitmap drops what does not fit
/// rather than growing.)
pub const COLS: usize = 384;
pub const ROWS: usize = 180;
/// `COLS` bits per row, as twelve `u32`s (three `vec4<u32>` on the GPU).
pub const ROW_WORDS: usize = COLS / 32;

/// The settings menu's card: 48 characters wide.
pub const MENU_COLS: usize = 48;
/// A panel beside a picture (the DRIVE panel, the SHIP bay's card) and
/// the readout: 32 characters wide, so the picture keeps its room.
pub const PANEL_COLS: usize = 32;

/// The width in font pixels of a block `cols` characters wide (the last
/// glyph has no tracking after it).
pub const fn block_width(cols: usize) -> usize {
    cols * ADVANCE - 1
}

/// The height in font pixels of a block `lines` lines tall.
pub const fn block_height(lines: usize) -> usize {
    if lines == 0 {
        0
    } else {
        (lines - 1) * LINE + GLYPH_H
    }
}

/// Rows top-to-bottom; within a row, `#` is a lit pixel and the first
/// character is the leftmost column.
fn rows(c: char) -> [&'static str; GLYPH_H] {
    match c.to_ascii_uppercase() {
        'A' => [
            ".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
        ],
        'B' => [
            "####.", "#...#", "#...#", "####.", "#...#", "#...#", "####.",
        ],
        'C' => [
            ".###.", "#...#", "#....", "#....", "#....", "#...#", ".###.",
        ],
        'D' => [
            "####.", "#...#", "#...#", "#...#", "#...#", "#...#", "####.",
        ],
        'E' => [
            "#####", "#....", "#....", "####.", "#....", "#....", "#####",
        ],
        'F' => [
            "#####", "#....", "#....", "####.", "#....", "#....", "#....",
        ],
        'G' => [
            ".###.", "#...#", "#....", "#.###", "#...#", "#...#", ".####",
        ],
        'H' => [
            "#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
        ],
        'I' => [
            ".###.", "..#..", "..#..", "..#..", "..#..", "..#..", ".###.",
        ],
        'J' => [
            "..###", "...#.", "...#.", "...#.", "...#.", "#..#.", ".##..",
        ],
        'K' => [
            "#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#",
        ],
        'L' => [
            "#....", "#....", "#....", "#....", "#....", "#....", "#####",
        ],
        'M' => [
            "#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#",
        ],
        'N' => [
            "#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#", "#...#",
        ],
        'O' => [
            ".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
        ],
        'P' => [
            "####.", "#...#", "#...#", "####.", "#....", "#....", "#....",
        ],
        'Q' => [
            ".###.", "#...#", "#...#", "#...#", "#.#.#", "#..#.", ".##.#",
        ],
        'R' => [
            "####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#",
        ],
        'S' => [
            ".####", "#....", "#....", ".###.", "....#", "....#", "####.",
        ],
        'T' => [
            "#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#..",
        ],
        'U' => [
            "#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
        ],
        'V' => [
            "#...#", "#...#", "#...#", "#...#", "#...#", ".#.#.", "..#..",
        ],
        'W' => [
            "#...#", "#...#", "#...#", "#.#.#", "#.#.#", "##.##", "#...#",
        ],
        'X' => [
            "#...#", "#...#", ".#.#.", "..#..", ".#.#.", "#...#", "#...#",
        ],
        'Y' => [
            "#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#..",
        ],
        'Z' => [
            "#####", "....#", "...#.", "..#..", ".#...", "#....", "#####",
        ],
        '0' => [
            ".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###.",
        ],
        '1' => [
            "..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###.",
        ],
        '2' => [
            ".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####",
        ],
        '3' => [
            "#####", "...#.", "..#..", "...#.", "....#", "#...#", ".###.",
        ],
        '4' => [
            "...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#.",
        ],
        '5' => [
            "#####", "#....", "####.", "....#", "....#", "#...#", ".###.",
        ],
        '6' => [
            "..##.", ".#...", "#....", "####.", "#...#", "#...#", ".###.",
        ],
        '7' => [
            "#####", "....#", "...#.", "..#..", ".#...", ".#...", ".#...",
        ],
        '8' => [
            ".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###.",
        ],
        '9' => [
            ".###.", "#...#", "#...#", ".####", "....#", "...#.", ".##..",
        ],
        '.' => [
            ".....", ".....", ".....", ".....", ".....", "..#..", "..#..",
        ],
        ',' => [
            ".....", ".....", ".....", ".....", "..#..", "..#..", ".#...",
        ],
        ':' => [
            ".....", "..#..", "..#..", ".....", "..#..", "..#..", ".....",
        ],
        ';' => [
            ".....", "..#..", "..#..", ".....", "..#..", "..#..", ".#...",
        ],
        '/' => [
            "....#", "....#", "...#.", "..#..", ".#...", "#....", "#....",
        ],
        '\\' => [
            "#....", "#....", ".#...", "..#..", "...#.", "....#", "....#",
        ],
        '-' => [
            ".....", ".....", ".....", "#####", ".....", ".....", ".....",
        ],
        '+' => [
            ".....", "..#..", "..#..", "#####", "..#..", "..#..", ".....",
        ],
        '=' => [
            ".....", ".....", "#####", ".....", "#####", ".....", ".....",
        ],
        '%' => [
            "##...", "##..#", "...#.", "..#..", ".#...", "#..##", "...##",
        ],
        '*' => [
            ".....", "#.#.#", ".###.", "#####", ".###.", "#.#.#", ".....",
        ],
        '<' => [
            "....#", "...#.", "..#..", ".#...", "..#..", "...#.", "....#",
        ],
        '>' => [
            "#....", ".#...", "..#..", "...#.", "..#..", ".#...", "#....",
        ],
        '[' => [
            ".###.", ".#...", ".#...", ".#...", ".#...", ".#...", ".###.",
        ],
        ']' => [
            ".###.", "...#.", "...#.", "...#.", "...#.", "...#.", ".###.",
        ],
        '(' => [
            "...#.", "..#..", ".#...", ".#...", ".#...", "..#..", "...#.",
        ],
        ')' => [
            ".#...", "..#..", "...#.", "...#.", "...#.", "..#..", ".#...",
        ],
        '_' => [
            ".....", ".....", ".....", ".....", ".....", ".....", "#####",
        ],
        '!' => [
            "..#..", "..#..", "..#..", "..#..", "..#..", ".....", "..#..",
        ],
        '?' => [
            ".###.", "#...#", "....#", "...#.", "..#..", ".....", "..#..",
        ],
        '\'' => [
            "..#..", "..#..", ".#...", ".....", ".....", ".....", ".....",
        ],
        '"' => [
            ".#.#.", ".#.#.", ".#.#.", ".....", ".....", ".....", ".....",
        ],
        '^' => [
            "..#..", ".#.#.", "#...#", ".....", ".....", ".....", ".....",
        ],
        '#' => [
            ".#.#.", ".#.#.", "#####", ".#.#.", "#####", ".#.#.", ".#.#.",
        ],
        '&' => [
            ".##..", "#..#.", "#..#.", ".##..", "#.#.#", "#..#.", ".##.#",
        ],
        '|' => [
            "..#..", "..#..", "..#..", "..#..", "..#..", "..#..", "..#..",
        ],
        '~' => [
            ".....", ".....", ".#..#", "#.#.#", "#..#.", ".....", ".....",
        ],
        '@' => [
            ".###.", "#...#", "#.###", "#.#.#", "#.###", "#....", ".###.",
        ],
        // A small filled diamond, the cursor mark: `>` is a chevron and
        // reads as "more"; the mark is a bead.
        '\u{25C6}' => [
            ".....", "..#..", ".###.", "#####", ".###.", "..#..", ".....",
        ],
        // A thin down chevron and up chevron for dropdown marks.
        '\u{2193}' => [
            ".....", ".....", "#...#", ".#.#.", "..#..", ".....", ".....",
        ],
        '\u{2191}' => [
            ".....", ".....", "..#..", ".#.#.", "#...#", ".....", ".....",
        ],
        _ => ["....."; GLYPH_H], // space and anything unmapped
    }
}

/// A glyph's rows as bit masks: bit 0 is the leftmost column. Public so
/// a caller that has rasterised a glyph some other way (the VR
/// eye-order self-check's own pixel readback, say) can mask-compare
/// against the font's own reference shape instead of re-deriving it.
pub fn glyph(c: char) -> [u8; GLYPH_H] {
    let mut out = [0u8; GLYPH_H];
    for (row, line) in rows(c).iter().enumerate() {
        for (col, ch) in line.bytes().enumerate() {
            if ch == b'#' {
                out[row] |= 1 << col;
            }
        }
    }
    out
}

/// Does the font have a real glyph for this character?
pub fn has_glyph(c: char) -> bool {
    glyph(c) != [0; GLYPH_H]
}

/// Word-wrap `text` to at most `cols` characters a line, breaking at
/// spaces; a word longer than a line is cut. Empty text is no lines.
pub fn wrap(text: &str, cols: usize) -> Vec<String> {
    let cols = cols.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let mut word = word;
        while word.chars().count() > cols {
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            let cut = word.char_indices().nth(cols).map_or(word.len(), |(i, _)| i);
            lines.push(word[..cut].to_string());
            word = &word[cut..];
        }
        let need = if line.is_empty() {
            word.chars().count()
        } else {
            line.chars().count() + 1 + word.chars().count()
        };
        if need > cols && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
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

    /// Draw `text` on text line `line` (the `LINE` pitch) at character
    /// column `col`. Returns the column just past the last glyph.
    pub fn draw_line(&mut self, col: usize, line: usize, text: &str) -> usize {
        self.draw(col * ADVANCE, line * LINE, text) / ADVANCE
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

    /// A line at the bottom of a twenty-line block must keep its glyph
    /// feet: a glyph drawn at y needs rows y..y+GLYPH_H, and the bitmap
    /// silently drops out-of-bounds pixels rather than growing.
    #[test]
    fn the_bottom_line_of_a_full_block_keeps_its_glyph_feet() {
        let mut b = TextBitmap::new();
        let y = 19 * LINE;
        b.draw(0, y, "L");
        // 'L' row 6 is the full foot.
        for x in 0..GLYPH_W {
            assert!(b.get(x, y + GLYPH_H - 1), "foot pixel {x} missing");
        }
        let (_, h) = b.used_extent();
        assert_eq!(h, y + GLYPH_H);
        assert!(h <= ROWS);
        assert_eq!(block_height(20), h);
    }

    /// '1' must actually look like a 1: a stem with a serif and a foot.
    #[test]
    fn glyph_shape_is_rendered_at_the_right_place() {
        let mut b = TextBitmap::new();
        b.draw(0, 0, "1");
        // Row 0 of '1' is "..#.." -> only the middle column.
        assert!(!b.get(1, 0));
        assert!(b.get(2, 0));
        assert!(!b.get(3, 0));
        // Row 1 is ".##.." -> the serif.
        assert!(b.get(1, 1) && b.get(2, 1));
        // Row 6 is ".###." -> the foot.
        assert!(b.get(1, 6) && b.get(2, 6) && b.get(3, 6));
    }

    /// The look-alikes a pilot must never confuse are drawn differently.
    #[test]
    fn look_alike_glyphs_are_distinct() {
        for (a, b) in [
            ('0', 'O'),
            ('1', 'I'),
            ('1', 'L'),
            ('5', 'S'),
            ('8', 'B'),
            ('2', 'Z'),
        ] {
            assert_ne!(glyph(a), glyph(b), "{a} and {b} look the same");
        }
        // The zero is slashed: its middle row has a diagonal pixel.
        assert_eq!(rows('0')[3], "#.#.#");
        assert_eq!(rows('O')[3], "#...#");
    }

    /// Every glyph is exactly five columns by seven rows, and no glyph
    /// leaks into the tracking column.
    #[test]
    fn every_glyph_fits_its_cell() {
        for c in 32u8..127u8 {
            for (i, row) in rows(c as char).iter().enumerate() {
                assert_eq!(row.len(), GLYPH_W, "{:?} row {i}: {row:?}", c as char);
                assert!(
                    row.bytes().all(|b| b == b'#' || b == b'.'),
                    "{:?} row {i}: {row:?}",
                    c as char
                );
            }
        }
    }

    #[test]
    fn draw_advances_by_glyph_pitch() {
        let mut b = TextBitmap::new();
        let end = b.draw(0, 0, "88");
        assert_eq!(end, 2 * ADVANCE);
        // Second glyph starts one advance over ('8' row 0 is ".###.").
        assert!(b.get(ADVANCE + 1, 0));
        // The tracking column between glyphs stays clear.
        for y in 0..GLYPH_H {
            assert!(!b.get(GLYPH_W, y));
        }
    }

    #[test]
    fn draw_line_lays_lines_out_on_the_pitch() {
        let mut b = TextBitmap::new();
        let end = b.draw_line(2, 3, "AB");
        assert_eq!(end, 4);
        assert!(b.get(2 * ADVANCE + 1, 3 * LINE), "'A' row 0 is .###.");
        assert_eq!(b.used_extent().1, 3 * LINE + GLYPH_H);
    }

    #[test]
    fn space_and_unknown_chars_are_blank_but_still_advance() {
        let mut b = TextBitmap::new();
        let end = b.draw(0, 0, " \u{00e9}");
        assert_eq!(end, 2 * ADVANCE);
        for y in 0..ROWS {
            for x in 0..COLS {
                assert!(!b.get(x, y), "blank glyph drew a pixel at {x},{y}");
            }
        }
        assert!(!has_glyph(' '));
        assert!(has_glyph('A') && has_glyph('7') && has_glyph('%'));
    }

    /// A readout that overruns the buffer must clip silently — never panic in
    /// the middle of a frame.
    #[test]
    fn drawing_off_the_edge_clips_without_panicking() {
        let mut b = TextBitmap::new();
        b.draw(COLS - 2, ROWS - 1, "888888");
        b.draw(COLS + 50, 0, "8");
        b.draw(0, ROWS + 5, "8");
        // '8' row 0 is ".###.": column 1 of the glyph, at x = COLS - 1.
        assert!(b.get(COLS - 1, ROWS - 1));
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
        b.draw(0, 0, "1");
        assert_eq!(b.used_extent(), (GLYPH_W - 1, GLYPH_H), "'1' is four wide");
        b.draw(0, LINE, "1M");
        // Widest row wins; height reaches the last occupied row.
        assert_eq!(b.used_extent(), (ADVANCE + GLYPH_W, LINE + GLYPH_H));
    }

    #[test]
    fn block_sizes_match_what_draw_occupies() {
        let mut b = TextBitmap::new();
        for line in 0..4 {
            b.draw_line(0, line, "MMMMMMMM");
        }
        assert_eq!(b.used_extent(), (block_width(8), block_height(4)));
        assert_eq!(block_height(0), 0);
    }

    #[test]
    fn used_extent_spans_word_boundaries() {
        let mut b = TextBitmap::new();
        b.set(40, 2);
        assert_eq!(b.used_extent(), (41, 3));
        b.set(COLS - 1, 5);
        assert_eq!(b.used_extent(), (COLS, 6));
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

    /// Every character the game prints must have a real glyph, or the HUD
    /// silently renders blanks: the readout's units, the menu's marks,
    /// the mimics' hails (with their punctuation), the key names.
    #[test]
    fn the_games_charset_is_covered() {
        for c in "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789%./:-+=<>[]()*_!?,'\"^#&|~@\\\u{25C6}\u{2191}\u{2193}"
            .chars()
        {
            assert!(has_glyph(c), "no glyph for {c:?}");
        }
    }

    #[test]
    fn wrap_breaks_at_spaces_and_cuts_long_words() {
        assert_eq!(
            wrap("LAND HARD IN 50S  DOWN 147  ALONG 723 M/S  VS +12", 24),
            vec!["LAND HARD IN 50S DOWN", "147 ALONG 723 M/S VS +12"]
        );
        assert_eq!(wrap("", 10), Vec::<String>::new());
        assert_eq!(wrap("ABCDEFGHIJ", 4), vec!["ABCD", "EFGH", "IJ"]);
        assert_eq!(wrap("A B", 1), vec!["A", "B"]);
        for line in wrap("the quick brown fox jumps over the lazy dog", 11) {
            assert!(line.chars().count() <= 11, "{line:?}");
        }
    }
}
