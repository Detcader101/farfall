//! The text readout on the glass: what the pilot reads at a glance —
//! the frame's numbers in two columns, the altitude and the speed, the
//! flight computer's state, and one status line (the landing, a hail,
//! the hold, the drive's strain, the guns, the haul) wrapped so it never
//! runs off the block.

use farfall_render::text::{wrap, TextBitmap, PANEL_COLS};

/// What the readout shows this frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Readout {
    pub fps: f64,
    pub low_fps: f64,
    pub cpu_ms: f64,
    pub rest_ms: f64,
    pub msaa: u32,
    pub scale_pct: f32,
    pub size: (u32, u32),
    pub altitude_m: f64,
    pub speed_mps: f64,
    pub assist: bool,
    pub bench: bool,
    /// The wind over the hull: speed (m/s) and the arrow it blows along,
    /// relative to the nose. None in vacuum or a calm.
    pub wind: Option<(f32, &'static str)>,
    /// The status line, if there is one.
    pub status: Option<String>,
}

/// The 8-way arrow for a wind blowing toward `angle_rad` off the nose
/// (positive to the pilot's right): the way the air goes, as the pilot
/// sits — `^` up the screen is downwind dead ahead.
pub fn arrow(angle_rad: f32) -> &'static str {
    const ARROWS: [&str; 8] = ["^", "^>", ">", "V>", "V", "<V", "<", "<^"];
    let turn = angle_rad.rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU;
    ARROWS[((turn * 8.0).round() as usize) % 8]
}

/// The readout's width in characters.
pub const COLS: usize = PANEL_COLS;
/// The status line may take this many lines.
pub const STATUS_LINES: usize = 3;

/// The readout's lines, top to bottom.
pub fn lines(r: &Readout) -> Vec<String> {
    let mut out = vec![
        format!("{:.0} FPS   1% LOW {:.0}", r.fps, r.low_fps),
        format!("CPU {:.1}MS   REST {:.1}MS", r.cpu_ms, r.rest_ms),
        format!(
            "{}X MSAA   {:.0}%   {}X{}",
            r.msaa, r.scale_pct, r.size.0, r.size.1
        ),
        format!(
            "ALT {}",
            farfall_render::gauge::length_text(r.altitude_m as f32)
        ),
        format!(
            "VEL {}",
            farfall_render::gauge::speed_text(r.speed_mps as f32)
        ),
    ];
    if let Some((mps, arrow)) = r.wind {
        out.push(format!("WIND {mps:.0} M/S {arrow}"));
    }
    // The flight computer's state lives on the HUD because the log is
    // invisible in fullscreen — X seemed broken when it was merely
    // silent.
    out.push((if r.assist { "FC ON" } else { "FC OFF" }).to_string());
    if r.bench {
        out.push("BENCH SIM FROZEN".to_string());
    }
    if let Some(status) = &r.status {
        let mut lines = wrap(status, COLS);
        lines.truncate(STATUS_LINES);
        out.extend(lines);
    }
    out
}

/// Draw the readout into the bitmap.
pub fn render(text: &mut TextBitmap, r: &Readout) {
    text.clear();
    for (i, line) in lines(r).iter().enumerate() {
        text.draw_line(0, i, line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn readout(status: Option<&str>) -> Readout {
        Readout {
            fps: 360.4,
            low_fps: 335.0,
            cpu_ms: 0.24,
            rest_ms: 2.6,
            msaa: 4,
            scale_pct: 100.0,
            size: (2880, 1800),
            altitude_m: 12_000.0,
            speed_mps: 725.0,
            assist: true,
            bench: true,
            wind: None,
            status: status.map(str::to_string),
        }
    }

    /// The wind line sits between VEL and FC, reads whole metres a
    /// second, and points the way the air goes relative to the nose.
    #[test]
    fn the_wind_line_reads_speed_and_the_way_the_air_goes() {
        let mut r = readout(None);
        r.wind = Some((12.4, arrow(std::f32::consts::FRAC_PI_2)));
        let lines = lines(&r);
        assert_eq!(lines[5], "WIND 12 M/S >");
        assert_eq!(lines[6], "FC ON");
        assert!(lines.iter().all(|l| l.chars().count() <= COLS));
        assert_eq!(arrow(0.0), "^", "downwind dead ahead");
        assert_eq!(arrow(std::f32::consts::PI), "V");
        assert_eq!(arrow(-std::f32::consts::FRAC_PI_2), "<");
        assert_eq!(arrow(std::f32::consts::FRAC_PI_4), "^>");
    }

    /// The landing line and a hail are long: they wrap inside the block
    /// instead of running off its right edge.
    #[test]
    fn long_status_lines_wrap_inside_the_block() {
        let r = readout(Some("LAND HARD IN 50S  DOWN 147  ALONG 723 M/S  VS +12"));
        let lines = lines(&r);
        for line in &lines {
            assert!(line.chars().count() <= COLS, "{line:?}");
        }
        assert!(lines.iter().any(|l| l.starts_with("LAND HARD")));
        assert!(lines.iter().any(|l| l.contains("VS +12")), "{lines:?}");
        let r = readout(Some("HAIL: EASY. NO CLAIM HERE. LUCK TO YOU AND YOURS."));
        let lines = super::lines(&r);
        assert!(lines.len() <= 7 + STATUS_LINES);
        assert!(lines.last().unwrap().contains("YOURS"));
    }

    #[test]
    fn the_numbers_read_in_two_columns() {
        let lines = lines(&readout(None));
        assert_eq!(lines[0], "360 FPS   1% LOW 335");
        assert_eq!(lines[1], "CPU 0.2MS   REST 2.6MS");
        assert_eq!(lines[2], "4X MSAA   100%   2880X1800");
        assert!(lines[3].starts_with("ALT "));
        assert!(lines[4].starts_with("VEL "));
        assert_eq!(lines[5], "FC ON");
        assert_eq!(lines[6], "BENCH SIM FROZEN");
        assert_eq!(lines.len(), 7);
        let mut t = TextBitmap::new();
        render(&mut t, &readout(None));
        assert_eq!(t.used_extent().1, farfall_render::text::block_height(7));
    }
}
