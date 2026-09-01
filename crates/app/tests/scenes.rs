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

    /// Mean absolute difference from another frame of the same size, 0..1.
    fn diff(&self, other: &Frame) -> f32 {
        assert_eq!((self.w, self.h), (other.w, other.h));
        let mut sum = 0.0f64;
        for y in 0..self.h {
            for x in 0..self.w {
                let a = self.at(x, y);
                let b = other.at(x, y);
                sum += ((a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs()) as f64;
            }
        }
        (sum / (3.0 * self.w as f64 * self.h as f64)) as f32
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
    // TMPDIR is the Unix name; Windows' temp_dir() reads TMP / TEMP.
    cmd.env("TMPDIR", &dir)
        .env("TMP", &dir)
        .env("TEMP", &dir)
        .env("HOME", &home)
        .env("FARFALL_MSAA", "1")
        .env("FARFALL_BENCH", "1")
        .env("FARFALL_BENCH_SECONDS", "2")
        .env("FARFALL_CAPTURE", "final");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let find_png = || -> Option<PathBuf> {
        std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.extension().is_some_and(|x| x == "png"))
    };
    let mut png: Option<PathBuf> = None;
    // The scenes run in parallel on one GPU: a starved run can quit before
    // its final frame is read back. One more go before it counts.
    for attempt in 0..2 {
        let out = cmd.output().expect("the game runs");
        assert!(
            out.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        png = find_png();
        if png.is_some() {
            break;
        }
        eprintln!("{name}: no capture on attempt {attempt}, again");
    }
    let png = png.unwrap_or_else(|| panic!("{name}: no capture in {}", dir.display()));
    let decoder = png::Decoder::new(std::fs::File::open(&png).unwrap());
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    // FARFALL_SCENE_KEEP=1 leaves the captures on disk, to be looked at.
    if std::env::var("FARFALL_SCENE_KEEP").is_err() {
        let _ = std::fs::remove_dir_all(&dir);
    } else {
        eprintln!("{name}: kept {}", dir.display());
    }
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
const RING_ABOVE_POS: &str = "16706357148.3,3498039023.8,-24797998372.8";
const RING_ABOVE_LOOK: &str = "0.0206,-0.9998,0.0042";
const SUN_NEAR_POS: &str = "911880426.7,617725450.3,-970711421.9";
const SUN_LOOK: &str = "0.6211,0.4208,-0.6612";
// Deep space, away from the planet, the sun off to the right.
const NEBULA_LOOK: &str = "0,0.2,-1";

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
fn the_far_ring_resolves_into_rocks_and_hides_the_stars() {
    if !enabled() {
        return;
    }
    // Nine kilometres above the ring plane, looking along it: the sheet
    // fills the lower half of the view.
    let f = capture(
        "ringsheet",
        &[
            ("FARFALL_BENCH_POS", RING_ABOVE_POS),
            ("FARFALL_BENCH_LOOK", RING_ABOVE_LOOK),
        ],
    );
    // Rocks on the sheet: specks of lit grey, well short of star-white.
    let specks = f.share(0.1, 0.5, 0.9, 0.6, |c| {
        lum(c) > 0.3 && lum(c) < 0.85 && (c[0] - c[2]).abs() < 0.12
    });
    assert!(specks > 0.003, "rocks resolve on the far sheet: {specks}");
    // And no star-white pinpricks through it: the belt hides the sky
    // (the box sits under the horizon and clear of the reticle).
    let stars = f.share(0.1, 0.52, 0.45, 0.6, |c| lum(c) > 0.9);
    assert!(stars < 0.001, "no stars through the ring: {stars}");
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
fn the_arms_light_the_belt() {
    if !enabled() {
        return;
    }
    let f = capture(
        "arms",
        &[
            ("FARFALL_BENCH_POS", RING_POS),
            ("FARFALL_BENCH_LOOK", RING_LOOK_ALONG),
            ("FARFALL_BENCH_ARMS", "1"),
        ],
    );
    // Tracers and bursts: hot, warm pixels in the middle of the view.
    let fire = f.share(0.2, 0.3, 0.8, 0.7, |c| {
        c[0] > 0.75 && c[1] > 0.45 && c[0] > c[2] + 0.15
    });
    assert!(fire > 0.002, "tracers and sparks ahead: {fire}");
    // The rail's violet wake, somewhere left of the nose.
    let wake = f.share(0.2, 0.3, 0.7, 0.7, |c| c[2] > 0.3 && c[2] > c[1] + 0.08);
    assert!(wake > 0.0002, "the rail's wake: {wake}");
}

#[test]
fn a_mimic_drops_its_shroud_and_a_hostile_one_opens_fire() {
    if !enabled() {
        return;
    }
    // Mid-reveal: the ship glowing cyan as a hologram over its hardening
    // hull, left of the nose.
    let f = capture(
        "mimic-reveal",
        &[
            ("FARFALL_BENCH_POS", RING_POS),
            ("FARFALL_BENCH_LOOK", RING_LOOK_ALONG),
            ("FARFALL_BENCH_MIMIC", "reveal"),
        ],
    );
    let holo = f.share(0.2, 0.2, 0.55, 0.65, |c| {
        c[2] > 0.5 && c[1] > 0.4 && c[2] > c[0] + 0.15
    });
    assert!(holo > 0.0015, "the hologram out of the rock: {holo}");
    // Attacking: a solid hull with amber engines, red tracers coming at
    // us, a ripple on the shield.
    let g = capture(
        "mimic-attack",
        &[
            ("FARFALL_BENCH_POS", RING_POS),
            ("FARFALL_BENCH_LOOK", RING_LOOK_ALONG),
            ("FARFALL_BENCH_MIMIC", "attack"),
        ],
    );
    let hull = g.share(0.2, 0.2, 0.55, 0.65, |c| {
        let m = (c[0] + c[1] + c[2]) / 3.0;
        m > 0.12 && (c[0] - c[2]).abs() < 0.12 && (c[1] - m).abs() < 0.06
    });
    assert!(hull > 0.001, "a grey hull in the sun: {hull}");
    // Its fire: hot heads and warm tails between it and us.
    let fire = g.share(0.1, 0.2, 0.9, 0.9, |c| c[0] > 0.7 && c[0] > c[2] + 0.2);
    assert!(fire > 0.0002, "its fire in the air: {fire}");
    assert!(g.diff(&f) > 0.002, "the two scenes differ");
}

#[test]
fn the_debris_tumbles_ahead() {
    if !enabled() {
        return;
    }
    let f = capture(
        "debris",
        &[
            ("FARFALL_BENCH_POS", RING_POS),
            ("FARFALL_BENCH_LOOK", RING_LOOK_ALONG),
            ("FARFALL_BENCH_ARMS", "debris"),
        ],
    );
    // Fresh shards glow orange-white in the middle of the view.
    let ember = f.share(0.3, 0.3, 0.7, 0.7, |c| c[0] > 0.6 && c[0] > c[2] + 0.25);
    assert!(ember > 0.0003, "embers among the shards: {ember}");
}

#[test]
fn the_scars_glow_on_the_rock_ahead() {
    if !enabled() {
        return;
    }
    let f = capture(
        "scars",
        &[
            ("FARFALL_BENCH_POS", RING_POS),
            ("FARFALL_BENCH_LOOK", RING_LOOK_ALONG),
            ("FARFALL_BENCH_ARMS", "scars"),
        ],
    );
    // Hot orange-white craters somewhere on the glass.
    let hot = f.share(0.0, 0.0, 1.0, 1.0, |c| {
        c[0] > 0.8 && c[1] > 0.35 && c[0] > c[2] + 0.12
    });
    assert!(hot > 0.0002, "craters glowing: {hot}");
}

#[test]
fn the_gun_sight_holds_on_the_gimbal_ring() {
    if !enabled() {
        return;
    }
    // The head turned well past the gimbal: the sight stops on the ring,
    // amber, with a leader back to the gaze — against the same frame
    // with the sight off, measured where the reticle sits (35 degrees
    // from a gaze 55 off: a fifth of the way in from the left).
    let amber = |c: [f32; 3]| c[0] > 0.45 && c[1] > 0.25 && c[0] > c[2] + 0.2 && c[0] >= c[1];
    let with = capture(
        "sight",
        &[
            ("FARFALL_BENCH_ARMS", "sight"),
            ("FARFALL_BENCH_HEAD", "55,4"),
        ],
    )
    .share(0.2, 0.4, 0.45, 0.6, amber);
    let without = capture(
        "nosight",
        &[
            ("FARFALL_BENCH_ARMS", "nosight"),
            ("FARFALL_BENCH_HEAD", "55,4"),
        ],
    )
    .share(0.2, 0.4, 0.45, 0.6, amber);
    assert!(
        with > without + 0.0001,
        "the sight held on the ring: {with} vs {without} without"
    );
    // Straight ahead: a cyan sight at the centre of the glass.
    let g = capture("sight-ahead", &[("FARFALL_BENCH_ARMS", "sight")]);
    let cyan = g.share(0.42, 0.42, 0.58, 0.58, |c| {
        c[1] > 0.6 && c[2] > 0.6 && c[0] < 0.6
    });
    assert!(cyan > 0.0005, "the sight ahead: {cyan}");
}

#[test]
fn the_helmet_camera_moves_the_whole_view_together() {
    if !enabled() {
        return;
    }
    // Parked at a deflection the cabin, glass and world all shift as one:
    // the frame differs from the still one, but the dials are still on
    // their dash (the same cyan share, moved).
    let still = capture("shake-still", &[("FARFALL_BENCH_HEAD", "0,-26")]);
    let shook = capture(
        "shake",
        &[
            ("FARFALL_BENCH_HEAD", "0,-26"),
            ("FARFALL_BENCH_SHAKE", "4,3,6"),
        ],
    );
    let d = still.diff(&shook);
    assert!(d > 0.01, "the view moved: {d}");
    let cyan = |c: [f32; 3]| c[1] > 0.45 && c[2] > 0.45 && c[0] < 0.5;
    let a = still.share(0.0, 0.3, 1.0, 1.0, cyan);
    let b = shook.share(0.0, 0.3, 1.0, 1.0, cyan);
    assert!(
        (a - b).abs() < a * 0.35 + 0.002,
        "the dials came along: {a} vs {b}"
    );
}

#[test]
fn the_chase_view_shows_the_ship_and_no_cockpit() {
    if !enabled() {
        return;
    }
    let f = capture("chase", &[("FARFALL_BENCH_CHASE", "1")]);
    // The fighter, a few lengths ahead of the eye: a solid block of hull
    // in the middle of the frame — low saturation, neither sky nor void.
    let hull = f.share(0.40, 0.42, 0.60, 0.65, |c| {
        lum(c) > 0.25 && (c[0] - c[2]).abs() < 0.16 && (c[1] - c[2]).abs() < 0.16
    });
    assert!(
        hull > 0.05,
        "the ship fills the chase view's middle: {hull}"
    );
    // And no instrument cluster: the dash's cyan dials are gone from the
    // bottom of the frame.
    let dials = f.share(0.0, 0.75, 1.0, 1.0, |c| {
        c[1] > 0.5 && c[2] > 0.55 && c[0] < 0.35
    });
    assert!(dials < 0.004, "no cockpit in third person: {dials}");
}

#[test]
fn the_holo3pp_stands_as_a_3d_hologram_over_the_dash() {
    if !enabled() {
        return;
    }
    let f = capture("holo", &[("FARFALL_BENCH_HOLO", "1")]);
    // In the glass at the upper right, under the mini map and clear of the
    // dials: the little ship and its emitter's ring in the hologram's
    // cyan...
    let cyan = f.share(0.80, 0.20, 1.0, 0.50, |c| {
        c[1] > 0.45 && c[2] > 0.5 && c[0] < 0.45 && c[2] > c[0] + 0.2
    });
    assert!(
        cyan > 0.002,
        "the hologram's ship and emitter are lit: {cyan}"
    );
    // ...and the nearest body (Uranus, 200 km under the keel here) as an
    // amber wire globe at its true size beside it.
    let amber = f.share(0.80, 0.20, 1.0, 0.50, |c| {
        c[0] > 0.45 && c[0] > c[2] + 0.15 && c[1] > 0.25
    });
    assert!(amber > 0.001, "the body's wire globe is lit: {amber}");
    // And nothing of it over the forward view: the hologram is out of the
    // way.
    let over_view = f.share(0.30, 0.10, 0.70, 0.45, |c| {
        c[0] > 0.45 && c[0] > c[2] + 0.15 && c[1] > 0.25
    });
    assert!(
        over_view < 0.0005,
        "no hologram in the forward view: {over_view}"
    );
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

/// The SHIP bay (B): the whole screen, the fighter's hologram big on the
/// left over a deep backdrop, leader lines to its card on the right, and
/// the pointer over it.
#[test]
fn the_ship_bay_fills_the_screen_with_the_hologram_its_callouts_and_a_pointer() {
    if !enabled() {
        return;
    }
    let f = capture("ship", &[("FARFALL_BENCH_SHIP", "1")]);
    // The hologram, left of centre: cyan over the backdrop.
    let cyan = f.share(0.15, 0.35, 0.65, 0.75, |c| {
        c[1] > 0.45 && c[2] > 0.5 && c[0] < 0.5 && c[2] > c[0] + 0.15
    });
    assert!(cyan > 0.03, "the hologram is lit: {cyan}");
    // The backdrop is deep everywhere the hologram is not: the cockpit
    // behind it is gone.
    let dark = f.share(0.02, 0.02, 0.98, 0.2, |c| lum(c) < 0.25);
    assert!(dark > 0.85, "the bay's backdrop covers the screen: {dark}");
    // The card, top right, has its cyan text.
    let card = f.share(0.80, 0.03, 0.99, 0.16, |c| {
        c[1] > 0.45 && c[2] > 0.5 && c[0] < 0.5
    });
    assert!(card > 0.01, "the card is up: {card}");
    // The pointer: a bright arrow with a dark edge at 62%, 36%.
    let pointer = f.share(0.60, 0.34, 0.66, 0.42, |c| lum(c) > 0.6);
    assert!(pointer > 0.001, "the pointer is drawn: {pointer}");
}

/// The WARTHOG style (FARFALL_BENCH_STYLE=warthog): every dial set into
/// the dash as an A-10 steam gauge — white markings and needles, no
/// hologram cyan on the cluster.
#[test]
fn the_warthog_dials_are_white_steam_gauges_in_the_dash() {
    if !enabled() {
        return;
    }
    let f = capture("warthog", &[("FARFALL_BENCH_STYLE", "warthog")]);
    // The cluster band across the lower dash: white markings...
    let white = f.share(0.2, 0.6, 0.8, 0.92, |c| {
        lum(c) > 0.6 && (c[0] - c[2]).abs() < 0.12 && (c[1] - c[2]).abs() < 0.12
    });
    assert!(white > 0.004, "white markings on the faces: {white}");
    // ...and next to no hologram cyan there.
    let cyan = f.share(0.2, 0.6, 0.8, 0.92, |c| {
        c[2] > 0.6 && c[1] > 0.5 && c[0] < 0.35
    });
    assert!(
        cyan < white * 0.5,
        "the cluster is not cyan: {cyan} vs {white}"
    );
}

#[test]
fn the_nebula_colours_the_sky_and_goes_away_when_off() {
    if !enabled() {
        return;
    }
    // Deep space, looking away from the planet: sky wall to wall.
    let on = capture(
        "nebula",
        &[
            ("FARFALL_BENCH_POS", "0,0,-400000"),
            ("FARFALL_BENCH_LOOK", NEBULA_LOOK),
            ("FARFALL_BENCH_NEBULA", "1"),
        ],
    );
    let off = capture(
        "nebula-off",
        &[
            ("FARFALL_BENCH_POS", "0,0,-400000"),
            ("FARFALL_BENCH_LOOK", NEBULA_LOOK),
            ("FARFALL_BENCH_NEBULA", "off"),
        ],
    );
    // Coloured gas over the glass, clear of the sun's flare on the right:
    // lit, clearly not grey, and blue-heavy like both stock hues. "Lit" is
    // measured above the HDR picture's floor: AgX leaves empty space near
    // sRGB 0.15 with a cool cast, which the old 0.08 counted as gas.
    let gas = |f: &Frame| {
        f.share(0.15, 0.0, 0.7, 0.45, |c| {
            let mx = c[0].max(c[1]).max(c[2]);
            let mn = c[0].min(c[1]).min(c[2]);
            mx > 0.22 && mx - mn > 0.04 && c[2] > c[1] + 0.02
        })
    };
    let on_gas = gas(&on);
    let off_gas = gas(&off);
    assert!(on_gas > 0.05, "the nebula's gas across the sky: {on_gas}");
    assert!(
        on_gas > off_gas * 3.0 + 0.03,
        "off is black sky again: on {on_gas} vs off {off_gas}"
    );
    let d = on.diff(&off);
    assert!(d > 0.004, "the knob changes the picture: {d}");
}

/// FARFALL_FOV=deg (a graphics knob like FARFALL_SCALE, over the settings
/// file's graphics.fov): 50 degrees zooms into the sight, 100 pulls the
/// whole cabin into frame — two different pictures, not two perf runs of
/// the same one.
#[test]
fn the_fov_knob_reframes_the_scene() {
    if !enabled() {
        return;
    }
    let narrow = capture("fov50", &[("FARFALL_FOV", "50")]);
    let wide = capture("fov100", &[("FARFALL_FOV", "100")]);
    let d = narrow.diff(&wide);
    assert!(d > 0.02, "50 and 100 degrees are different pictures: {d}");
}
