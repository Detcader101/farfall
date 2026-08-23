//! Scene tests: the game itself, run headless through the bench knobs, and
//! its captures read back and measured. Tests for the picture, because a
//! feature that compiles and passes its unit tests can still draw nothing
//! (a uniform lane in the wrong place once hid Uranus entirely).
//!
//! They need a GPU and a window server, so they run only with
//! FARFALL_SCENE_TESTS=1 (the pre-commit gate sets it on a machine that
//! can); elsewhere they pass trivially.

use std::path::PathBuf;
use std::process::Command;

struct Frame {
    w: usize,
    h: usize,
    px: Vec<u8>,
}

impl Frame {
    fn at(&self, x: usize, y: usize) -> [f32; 3] {
        let i = (y * self.w + x) * 4;
        [
            self.px[i] as f32 / 255.0,
            self.px[i + 1] as f32 / 255.0,
            self.px[i + 2] as f32 / 255.0,
        ]
    }

    /// Mean colour over a box given as fractions of the frame.
    fn mean(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> [f32; 3] {
        let mut sum = [0.0f64; 3];
        let mut n = 0.0;
        for y in ((y0 * self.h as f32) as usize)..((y1 * self.h as f32) as usize) {
            for x in ((x0 * self.w as f32) as usize)..((x1 * self.w as f32) as usize) {
                let c = self.at(x, y);
                for k in 0..3 {
                    sum[k] += c[k] as f64;
                }
                n += 1.0;
            }
        }
        [
            (sum[0] / n) as f32,
            (sum[1] / n) as f32,
            (sum[2] / n) as f32,
        ]
    }

    /// How many pixels in the box satisfy the predicate, as a fraction.
    fn share(&self, x0: f32, y0: f32, x1: f32, y1: f32, f: impl Fn([f32; 3]) -> bool) -> f32 {
        let mut hit = 0.0;
        let mut n = 0.0;
        for y in ((y0 * self.h as f32) as usize)..((y1 * self.h as f32) as usize) {
            for x in ((x0 * self.w as f32) as usize)..((x1 * self.w as f32) as usize) {
                if f(self.at(x, y)) {
                    hit += 1.0;
                }
                n += 1.0;
            }
        }
        hit / n
    }
}

fn enabled() -> bool {
    std::env::var("FARFALL_SCENE_TESTS").is_ok_and(|v| v == "1")
}

/// Run one bench capture with these extra environment variables and read
/// the frame back.
fn capture(name: &str, env: &[(&str, &str)]) -> Frame {
    let dir = std::env::temp_dir().join(format!("farfall-scene-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let home = dir.join("home");
    std::fs::create_dir_all(home.join(".farfall")).unwrap();
    std::fs::write(
        home.join(".farfall/settings.cfg"),
        "ui.gauges = stay\ncockpit.frame = on\n",
    )
    .unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_farfall"));
    cmd.env("TMPDIR", &dir)
        .env("HOME", &home)
        .env("FARFALL_MSAA", "1")
        .env("FARFALL_BENCH", "1")
        .env("FARFALL_BENCH_SECONDS", "2")
        .env("FARFALL_CAPTURE", "final");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("the game runs");
    assert!(
        out.status.success(),
        "{name}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let png: PathBuf = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.extension().is_some_and(|x| x == "png"))
        .unwrap_or_else(|| panic!("{name}: no capture in {}", dir.display()));
    let decoder = png::Decoder::new(std::fs::File::open(&png).unwrap());
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    let _ = std::fs::remove_dir_all(&dir);
    Frame {
        w: info.width as usize,
        h: info.height as usize,
        px: buf,
    }
}

fn lum(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

// Bench positions, in the planet's frame.
const RING_POS: &str = "16706348420.5,3498037764.1,-24798000172.3";
const RING_LOOK_ALONG: &str = "0.1371,-0.9902,0.0283";
const RING_LOOK_AT_URANUS: &str = "0.2019,0,-0.9794";
const SUN_NEAR_POS: &str = "911880426.7,617725450.3,-970711421.9";
const SUN_LOOK: &str = "0.6211,0.4208,-0.6612";

#[test]
fn uranus_fills_the_sky_from_its_ring_with_rocks_in_front() {
    if !enabled() {
        return;
    }
    let f = capture(
        "uranus",
        &[
            ("FARFALL_BENCH_POS", RING_POS),
            ("FARFALL_BENCH_LOOK", RING_LOOK_AT_URANUS),
        ],
    );
    // The ice giant: the upper middle of the view is pale and cyan-white.
    let sky = f.mean(0.35, 0.05, 0.65, 0.25);
    assert!(lum(sky) > 0.55, "Uranus is bright: {sky:?}");
    assert!(sky[2] >= sky[0], "and blue-ish: {sky:?}");
    // Rocks: dark, grey, sizeable blobs against it.
    let rocks = f.share(0.1, 0.05, 0.9, 0.6, |c| {
        lum(c) < 0.35 && (c[0] - c[2]).abs() < 0.12
    });
    assert!(rocks > 0.01, "rocks in front of Uranus: {rocks}");
}

#[test]
fn the_belt_has_rocks_and_the_ring_is_a_haze_not_a_wall() {
    if !enabled() {
        return;
    }
    let f = capture(
        "belt",
        &[
            ("FARFALL_BENCH_POS", RING_POS),
            ("FARFALL_BENCH_LOOK", RING_LOOK_ALONG),
        ],
    );
    let rocks = f.share(0.2, 0.3, 0.8, 0.6, |c| {
        lum(c) > 0.25 && lum(c) < 0.8 && (c[0] - c[2]).abs() < 0.1 && (c[0] - c[1]).abs() < 0.1
    });
    assert!(rocks > 0.01, "rocks along the ring: {rocks}");
    // The sheet above: a haze, well short of grey.
    let haze = f.mean(0.3, 0.02, 0.7, 0.2);
    assert!(lum(haze) < 0.45, "the ring from inside is a haze: {haze:?}");
}

#[test]
fn the_sun_up_close_has_a_surface_with_spots() {
    if !enabled() {
        return;
    }
    let f = capture(
        "sun",
        &[
            ("FARFALL_BENCH_POS", SUN_NEAR_POS),
            ("FARFALL_BENCH_LOOK", SUN_LOOK),
        ],
    );
    let disc = f.mean(0.42, 0.38, 0.58, 0.62);
    assert!(lum(disc) > 0.6, "the disc is bright: {disc:?}");
    assert!(disc[0] >= disc[2], "and warm: {disc:?}");
    // Sunspots: dark pixels inside the disc.
    let spots = f.share(0.4, 0.35, 0.6, 0.65, |c| lum(c) < 0.45);
    assert!(spots > 0.003, "sunspots on the disc: {spots}");
}

#[test]
fn the_ground_has_a_blue_sky_over_it_and_no_murk() {
    if !enabled() {
        return;
    }
    let f = capture(
        "ground",
        &[
            ("FARFALL_BENCH_POS", "0,63709.98,0"),
            ("FARFALL_BENCH_LOOK", "1,0,0"),
        ],
    );
    let sky = f.mean(0.3, 0.05, 0.7, 0.3);
    assert!(sky[2] > sky[0] && sky[2] > 0.4, "blue sky: {sky:?}");
    let below = f.mean(0.3, 0.55, 0.7, 0.62);
    assert!(lum(below) > 0.3, "the ground is lit, not murk: {below:?}");
}

#[test]
fn the_shield_ripples_on_strikes_and_the_after_image_shows() {
    if !enabled() {
        return;
    }
    let f = capture(
        "shield",
        &[
            ("FARFALL_BENCH_POS", "0,0,-400000"),
            ("FARFALL_BENCH_LOOK", "0,0.2,-1"),
            ("FARFALL_BENCH_STRIKES", "5"),
        ],
    );
    let blue = f.share(0.2, 0.0, 0.8, 0.5, |c| c[2] > 0.35 && c[2] > c[0] + 0.12);
    assert!(blue > 0.02, "ripples of blue on the shell: {blue}");
    let g = capture(
        "ghost",
        &[
            ("FARFALL_BENCH_POS", "0,0,-400000"),
            ("FARFALL_BENCH_LOOK", "0,0,-1"),
            ("FARFALL_BENCH_GHOST", "0.35"),
        ],
    );
    let image = g.share(0.4, 0.3, 0.75, 0.6, |c| c[2] > 0.3 && c[2] > c[0] + 0.08);
    assert!(image > 0.002, "the after-image ahead: {image}");
}

#[test]
fn the_menu_and_the_dials_draw() {
    if !enabled() {
        return;
    }
    let f = capture("menu", &[("FARFALL_BENCH_MENU", "3")]);
    // Cyan text on a dark card, top left.
    let text = f.share(0.13, 0.18, 0.47, 0.4, |c| {
        c[1] > 0.5 && c[2] > 0.5 && c[0] < 0.6
    });
    assert!(text > 0.03, "menu text: {text}");
    let d = capture(
        "dials",
        &[
            ("FARFALL_BENCH_HEAD", "0,-26"),
            ("FARFALL_BENCH_G", "-2.5,6.5,-1.0"),
        ],
    );
    let lit = d.share(0.0, 0.4, 1.0, 1.0, |c| c[1] > 0.45 && c[2] > 0.45);
    assert!(lit > 0.01, "dials on the dash: {lit}");
}
