//! farfall-ap — native shell (SPEC §5.1).
//!
//! Owns the window, the fixed-timestep accumulator (SPEC §7.2), and the
//! sim → render translation. The sim is authoritative (SPEC §5.2): this loop
//! feeds it inputs and *reads* state; nothing here mutates the world directly.
//!
//! M1 scope: the ship is hand-flown. Keys map to sim [`Controls`] (see
//! [`input`]), the camera rides the hull looking down the nose, and rotational
//! flight assist is toggleable. Planet, HUD, and sun arrive in M1 tasks 4-6.

mod arms;
mod bay;
mod belt;
mod capture;
mod card;
mod cockpit;
mod eva;
mod heli;
mod hold;
mod hud_file;
mod input;
mod landing;
mod look;
mod map;
mod menu;
mod mimic;
mod miner;
mod panel;
mod readout;
mod reforger;
mod save;
mod settings;
mod shake;
mod stick;
mod warp;

use cockpit::Instrument;
use look::Look;
use menu::{Change, Menu, MenuEvent};
use settings::Settings;
use warp::Warp;
mod telemetry;
#[cfg(target_arch = "wasm32")]
pub mod web;
#[cfg(not(target_arch = "wasm32"))]
mod xr;

use glam::{DQuat, DVec3, Quat, Vec3};
use std::sync::Arc;
use web_time::{Duration, Instant};

use capture::Capture;
use farfall_audio::Audio;
use farfall_render::debris::{debris_pass, DebrisPass, DebrisScene, DebrisUniforms, ShardView};
use farfall_render::dust::{DustPass, DustScene, DustUniforms};
use farfall_render::heli::{heli_pass, HeliPass, HeliUniforms, HeliView};
use farfall_render::mimic::{mimic_pass, MimicPass, MimicUniforms, MimicView};
use farfall_render::scar::{scar_heat, scar_pass, ScarPass, ScarScene, ScarUniforms, ScarView};
use farfall_render::sight::{sight_pass, SightPass, SightScene, SightUniforms};
use farfall_render::tracer::{
    tracer_pass, BurstView, Occluder, SlugView, TracerPass, TracerScene, TracerUniforms,
};
use farfall_render::wind::{WindPass, WindScene, WindUniforms};
use farfall_render::{
    attitude::{
        guide_pass, gyro_pass, horizon_pass, Attitude, GuideUniforms, GyroUniforms, HorizonFade,
        HorizonUniforms,
    },
    bake::BakedMaps,
    belt::{belt_pass, BeltPass, BeltUniforms, RockView},
    blit::BlitPass,
    bodies::{BodiesPass, BodiesUniforms},
    gauge::{
        gauge_pass, gvec_pass, AltitudeFade, GForceFade, GaugeFade, GaugePass, GaugeUniforms,
        HoloSway, MachAlert,
    },
    ghost::{ghost_pass, GhostPass, GhostUniforms, GHOST_LIFE_S},
    holo::{holo_centre, holo_pass, HoloPass, HoloScene, HoloUniforms, HOLO_RADIUS_M},
    hologram::{
        hologram_pass, Callout, HologramCamera, HologramPass, HologramScene, HologramUniforms,
    },
    hud::{HudBlock, HudPass},
    instrument::InstrumentPass,
    jet::{jet_pass, JetPass, JetUniforms},
    nebula::{NebulaBake, NebulaKnobs, NebulaParams},
    planet::{PlanetAppearance, PlanetPass, PlanetUniforms},
    pointer::{pointer_pass, PointerPass, PointerUniforms},
    post::{PostPass, PostUniforms},
    shield::{shield_pass, Impact, ShieldPass, ShieldUniforms},
    starfield::StarfieldPass,
    text::{TextBitmap, LINE, MENU_COLS, PANEL_COLS},
    thermal::{PlasmaPass, PlasmaUniforms, ThermalInputs, ThermalPass},
    trajectory::{TrajectoryPass, TrajectoryUniforms, TrajectoryWorld, MARK_SPACING_M},
    CameraFrame, FrameUniforms, SceneTarget,
};
use farfall_sim as sim;
use input::{InputState, Named};
use telemetry::FrameStats;
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{KeyCode, PhysicalKey},
    window::Window,
};

const STAR_DENSITY: f64 = 1.0;
/// World-space direction to the sun. Fixed for now: a moving sun is a sim
/// concern (planet rotation, orbit) and does not belong in the renderer.
/// Chosen so the terminator crosses the visible face at spawn.
/// How often the frame-time window is summarised to the log.
/// A row of the SHIP bay's card.
#[derive(Debug, Clone, Copy, PartialEq)]
enum BayRow {
    Header,
    /// The airframe row: FIGHTER or HELICOPTER (SPEC §6.5c).
    Craft,
    Slot(usize),
    Option(usize, bay::Mount),
    Footer,
}

/// The nebula's bake, from its knobs.
fn nebula_params(s: &Settings) -> NebulaParams {
    NebulaParams::new(NebulaKnobs {
        enabled: s.nebula > 0.0,
        seed: s.nebula_seed,
        scale: s.nebula_scale,
        density: s.nebula_density,
        clouds: s.nebula_clouds,
        intensity: s.nebula,
        hue_a_deg: s.nebula_hue * 360.0,
        hue_b_deg: s.nebula_hue2 * 360.0,
        softness: s.nebula_spread,
    })
}

/// What a panel's answer does: settings saved and applied, the drive
/// fired, or the game left.
fn apply_menu_event(
    game: &mut Game,
    gpu: &mut Gpu,
    event_loop: &winit::event_loop::ActiveEventLoop,
    ev: MenuEvent,
) {
    match ev {
        MenuEvent::Changed(change) => {
            game.settings.save();
            match change {
                Change::Bindings => {
                    game.input.set_bindings(game.settings.bindings);
                    game.look.sensitivity = game.settings.look_sensitivity;
                }
                Change::Graphics => gpu.apply_graphics(&game.settings),
                Change::Layout => game.arms.mounts = game.settings.mounts,
            }
            // A no-op unless a nebula knob moved.
            gpu.nebula
                .bake(&gpu.device, &gpu.queue, nebula_params(&game.settings));
        }
        MenuEvent::SaveHud => match hud_file::save(&game.settings, game.hud_loaded) {
            Some((n, path)) => {
                game.hud_loaded = Some(n);
                log::info!("hud: saved {}", path.display());
            }
            None => log::warn!("hud: nowhere to save (no home directory)"),
        },
        MenuEvent::LoadHud(pick) => {
            if pick == 0 {
                game.settings = hud_file::apply(&game.settings, "");
                game.hud_loaded = None;
                log::info!("hud: the stock cockpit");
            } else if let Some((n, s)) = hud_file::load(&game.settings, pick) {
                game.settings = s;
                game.hud_loaded = Some(n);
                log::info!("hud: wearing hud-{n}");
            }
            game.settings.save();
        }
        MenuEvent::Quit => {
            game.log_exit("menu quit");
            event_loop.exit();
        }
        MenuEvent::Engage => {
            game.settings.save();
            game.engage_warp();
        }
        // The wizard sits over the open menu: same pause, same panel.
        MenuEvent::StickWizard => game.wizard = Some(stick::Wizard::new()),
        MenuEvent::NewGame => {
            save::forget();
            game.reset_world();
            game.mimics.line = Some(("NEW GAME".to_string(), game.state.time_s + 3.0));
            log::info!("world: new game, forgot the save");
        }
        MenuEvent::ExportReforger => match reforger::save() {
            Some(path) => log::info!("reforger: wrote {}", path.display()),
            None => log::warn!("reforger: nowhere to write (no home directory)"),
        },
        MenuEvent::Closed | MenuEvent::Nothing => {}
    }
}

/// The target-pixel rectangle a dial can reach: its drawing radius
/// (0.155 canopy units at size 1, on the reference glass) with room for
/// the hologram's sway, its socket, needles and text; None when hidden.
/// Canopy x runs over the aspect, so the patch is a rect, not a square.
fn dial_scissor(
    anchor: [f32; 2],
    on: f32,
    size: f32,
    aspect: f32,
    target: (u32, u32),
) -> Option<[u32; 4]> {
    if on <= 0.001 {
        return None;
    }
    let r = 0.155 * size * 1.6 + 0.12;
    let (w, h) = (target.0 as f32, target.1 as f32);
    let px = |ndc: f32, span: f32| ((ndc * 0.5 + 0.5) * span).clamp(0.0, span);
    let x0 = px(anchor[0] - r / aspect, w);
    let x1 = px(anchor[0] + r / aspect, w);
    let y0 = px(-(anchor[1] + r), h);
    let y1 = px(-(anchor[1] - r), h);
    let (x0, y0) = (x0.floor() as u32, y0.floor() as u32);
    let (x1, y1) = (x1.ceil() as u32, y1.ceil() as u32);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some([x0, y0, x1 - x0, y1 - y0])
}

/// The bay camera's field of view: tan of half of it.
const BAY_TAN_HALF_FOV: f32 = 0.42;
const PERF_LOG_EVERY: Duration = Duration::from_secs(5);
/// AUTO SCALE's floor on the world's scale: past this the picture is
/// mush, and a machine that cannot hold the floor here is told so.
const AUTO_SCALE_MIN: f32 = 0.35;
/// The least time between the governor's moves: the rate must settle.
const AUTO_SCALE_STEP_S: f32 = 0.75;
/// How long the rate must sit on the floor before the scale is raised.
const AUTO_SCALE_RAISE_S: f32 = 3.0;
/// The assumed distance, metres, of the virtual plane every VR glass
/// overlay is painted on (SPEC §5.3) — the readout, glass-style dials,
/// the mini map, the eye-order label. Not yet a menu setting
/// (`vr.hud-distance` is the eventual key); a reasonable dash-distance
/// estimate a pilot's own pass can retune. Flat flight never reads this
/// — `on_glass`'s eye offset is always zero there.
const VR_HUD_DISTANCE_M: f32 = 1.0;

/// FARFALL_VR_LABEL=1's own L/R corner mark's target angular height,
/// degrees — a fixed *angle*, not a fixed canopy-NDC size, so it reads
/// the same on a narrow-fov headset and a wide one (SPEC §5.3): a
/// synth capture (e80e9af, before the race-condition fix below) showed
/// the old fixed px_canopy=0.09 filling ~35% of the eye's own width, a
/// close obscuring plane by itself. [`label_px_canopy`] converts this
/// into the per-font-pixel canopy size from the live eye's own vertical
/// tangent.
#[cfg(not(target_arch = "wasm32"))]
const LABEL_TARGET_DEG: f32 = 4.0;
/// The mark's own top-left anchor, canopy NDC, before the per-eye
/// parallax shift: comfortably inside the frame's outer top corner, not
/// jammed against the literal rim — `canopy_glass`'s own dimming grows
/// sharply for `length(ndc*aspect, ndc.y) > 0.75`, and the old anchor's
/// magnitude sat well past that, which — thin ink, no backdrop, and
/// dimmed toward invisible by the glass itself — is the likeliest
/// reason nothing showed on a9869ff's own capture. Eye 1's own anchor
/// mirrors this so the mark's *right* edge sits the same distance from
/// ITS outer edge that eye 0's *left* edge sits from its own.
#[cfg(not(target_arch = "wasm32"))]
const LABEL_CORNER_LEFT: [f32; 2] = [-0.78, 0.80];

/// The per-font-pixel canopy size that makes a `GLYPH_H`-tall mark
/// subtend [`LABEL_TARGET_DEG`] of the eye's own vertical field,
/// computed from that eye's own tangent `ty` (its symmetric vertical
/// half-fov's tan) — not a fixed NDC constant tuned for one particular
/// headset's fov and wrong for any other. Linear in tangent space
/// (canopy NDC 0..1 spans `atan(ty)` radians near centre), the same
/// convention the capture-review measurements in this branch already
/// use.
#[cfg(not(target_arch = "wasm32"))]
fn label_px_canopy(ty: f32) -> f32 {
    let rad_per_ndc = ty.max(1e-4).atan();
    let glyph_height_ndc = LABEL_TARGET_DEG.to_radians() / rad_per_ndc;
    glyph_height_ndc / farfall_render::text::GLYPH_H as f32
}

/// The label's own top-left anchor (canopy NDC, after the per-eye
/// parallax shift) and per-font-pixel size for `eye` of `eyes` — the
/// one place both `xr_composite`'s draw and `eye_order_self_check`'s
/// readback compute this from, so a future change to one cannot
/// silently stop matching the other the way the old hand-duplicated
/// constants could.
#[cfg(not(target_arch = "wasm32"))]
fn label_geometry(eye: usize, eyes: &[VrEye; 2]) -> ([f32; 2], f32) {
    let e = &eyes[eye];
    let tx = (e.tan[0] + e.tan[1]).max(1e-3) * 0.5;
    let ty = (e.tan[2] + e.tan[3]).max(1e-3) * 0.5;
    let d = VR_HUD_DISTANCE_M;
    let shift = [-e.pos.x / (d * tx), -e.pos.y / (d * ty)];
    let px = label_px_canopy(ty);
    let width_ndc = farfall_render::text::ADVANCE as f32 * px;
    let corner = if eye == 0 {
        LABEL_CORNER_LEFT
    } else {
        [-LABEL_CORNER_LEFT[0] - width_ndc, LABEL_CORNER_LEFT[1]]
    };
    ([corner[0] + shift[0], corner[1] + shift[1]], px)
}

/// A glass anchor as the turned head sees it: the glass is a sphere about
/// the pilot, so every element is re-projected, not slid (look.rs). The
/// anchors are laid out in a REFERENCE projection — the pilot's base field
/// of view, head centred — and shown through the live one, so a throttle
/// flare or a warp does not slide them over the ship. `head` is whichever
/// rotation this glass is pinned against — the identity for a flat HUD
/// deliberately kept screen-fixed, the pilot's own look in DESIGN mode, or
/// a headset's real orientation so the glass stays cockpit-fixed in VR
/// instead of following the pilot's face (see [`Game::glass_head`]).
/// `eye_pos` is that same headset's per-eye seat (zero everywhere but
/// VR): without it every glass element sits at optical infinity while
/// the cabin around it has real depth, which reads as a close plane
/// obscuring the view (SPEC §5.3) — reproject_with_eye's own parallax
/// shift is what puts it on the glass instead.
fn on_glass(head: Quat, eye_pos: Vec3, cam: &CameraFrame, ref_tan: f32, a: [f32; 2]) -> [f32; 2] {
    look::reproject_with_eye(
        head,
        eye_pos,
        VR_HUD_DISTANCE_M,
        a,
        ref_tan,
        (cam.fov_y * 0.5).tan(),
        cam.aspect,
    )
}

/// Where a dial sits and whether it shows: a hidden instrument keeps any
/// anchor and a visibility of zero. The anchors themselves are the slots
/// in `cockpit.rs` (or wherever the pilot dragged it); the menu assigns
/// them.
fn slot_of(
    layout: &cockpit::Layout,
    head: Quat,
    eye_pos: Vec3,
    cam: &CameraFrame,
    ref_tan: f32,
    i: Instrument,
) -> ([f32; 2], f32) {
    match layout.anchor(i) {
        Some(a) => (on_glass(head, eye_pos, cam, ref_tan, a), 1.0),
        None => ([0.0, 0.0], 0.0),
    }
}
/// The Chaos Drive's limits: seconds of full running before the drive
/// slips (the slip point itself is drawn between 70% and 100% of this —
/// the pilot never knows exactly), and how long the entropy takes to
/// ease off once the field is down.
const HYPER_STRAIN_S: f32 = 40.0;
const HYPER_EASE_S: f32 = 90.0;

/// How often the world is autosaved, in sim seconds (never wall-clock: a
/// paused/frozen session should not burn through them).
const AUTOSAVE_INTERVAL_S: f64 = 30.0;

/// Whether this run may touch the world file at all — shared by the load
/// in `App::init_gpu`, the autosave in `tick`, and the store in
/// `log_exit`. Pure, so it is trivial to test exhaustively: `frozen` or a
/// bench spawn override (`FARFALL_BENCH_POS`/`ALT`/`VEL`/`LOOK`/`ROLL`)
/// always refuses, `env_resume` of "0"/"off"/"false" always refuses, and
/// otherwise it is exactly the RESUME setting. There is no environment
/// value that forces it on over a pilot's RESUME OFF.
fn resume_allowed(
    settings_resume: bool,
    frozen: bool,
    env_resume: Option<&str>,
    bench_env_present: bool,
) -> bool {
    if frozen || bench_env_present {
        return false;
    }
    if matches!(env_resume, Some("0" | "off" | "false")) {
        return false;
    }
    settings_resume
}

/// The benchmark-only spawn overrides that make a run's start unlike a
/// real one — `FARFALL_BENCH` itself is `Game::frozen`, tracked
/// separately.
fn bench_spawn_env_present() -> bool {
    [
        "FARFALL_BENCH_POS",
        "FARFALL_BENCH_ALT",
        "FARFALL_BENCH_VEL",
        "FARFALL_BENCH_LOOK",
        "FARFALL_BENCH_ROLL",
    ]
    .iter()
    .any(|k| std::env::var_os(k).is_some())
}

/// `FARFALL_RESUME`, read fresh each time so a profiler's override always
/// wins without a restart.
fn env_resume() -> Option<String> {
    std::env::var("FARFALL_RESUME").ok()
}

/// Whether `FARFALL_BENCH_SAVE`/`FARFALL_BENCH_RESUME` may act at all:
/// only ever during a bench, and only when a path was actually given.
/// Pure, tested exhaustively the same way as [`resume_allowed`] — the
/// real read of the env var happens at the (untested) call site, this is
/// just the decision.
fn bench_path_action_allowed(bench: bool, path_env: Option<&std::ffi::OsStr>) -> bool {
    bench && path_env.is_some()
}

/// `FARFALL_BENCH_SAVE=<path>`: bench-only, for e2e verification — write
/// the world file to this EXACT path (never `~/.farfall`) at the bench's
/// own exit, so a scripted run produces a real sealed save of a real
/// parked world with no interactive window. Called from both of
/// `redraw`'s bench-exit sites (the headless/occluded capture path and
/// the normal end-of-frame path).
fn bench_save_world(game: &Game, bench: bool) {
    let path_env = std::env::var_os("FARFALL_BENCH_SAVE");
    if !bench_path_action_allowed(bench, path_env.as_deref()) {
        return;
    }
    game.snapshot()
        .store_to(std::path::Path::new(&path_env.unwrap()));
}

/// `T+HH:MM:SS` from a sim time in seconds, for the RESUMED readout.
fn format_hms(total_s: f64) -> String {
    let total = total_s.max(0.0) as u64;
    format!(
        "{:02}:{:02}:{:02}",
        total / 3600,
        (total % 3600) / 60,
        total % 60
    )
}

/// How unstable the flight is at this much of the way to the slip: calm
/// to halfway, then shaking harder and harder.
fn chaos_level(entropy: f32, slip_at: f32) -> f32 {
    let x = entropy / slip_at.max(1e-3);
    ((x - 0.5) / 0.5).clamp(0.0, 1.0).powi(2)
}

/// A unit vector from two uniform draws, evenly over the sphere.
fn random_unit(u1: f32, u2: f32) -> DVec3 {
    let z = 1.0 - 2.0 * u1.clamp(0.0, 1.0) as f64;
    let a = std::f64::consts::TAU * u2.clamp(0.0, 1.0) as f64;
    let s = (1.0 - z * z).max(0.0).sqrt();
    DVec3::new(s * a.cos(), s * a.sin(), z)
}

/// How hard the hull is being driven, 0..1: the speed against the
/// relativity wall, where the frame has the most to say.
fn hull_stress(speed_mps: f64) -> f32 {
    ((speed_mps / sim::RELATIVITY_FROM_MPS).clamp(0.0, 1.0)) as f32
}

/// Strikes per second: nothing at rest — a still ship meets nothing —
/// one every couple of seconds at a kilometre a second, rising steeply
/// with the speed (the ship sweeps a longer column and meets what is in
/// it), capped at a patter; in air, nothing — dust does not fly.
fn strike_rate_hz(speed_mps: f32, air: f32) -> f32 {
    let v = speed_mps.max(0.0) / 1_000.0;
    ((0.5 * v.powf(1.5)).min(6.0)) * (1.0 - air.clamp(0.0, 1.0))
}

/// A strike's size from one uniform draw: mostly grains, a few pebbles.
fn strike_size_from(u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    if u < 0.85 {
        0.05 + 0.25 * (u / 0.85)
    } else if u < 0.97 {
        0.3 + 0.4 * ((u - 0.85) / 0.12)
    } else {
        0.7 + 0.3 * ((u - 0.97) / 0.03)
    }
}

/// How far ahead the path predictor looks, seconds. A little over one
/// orbit at the spawn altitude (~8.5 min), so a closed orbit draws closed.
const TRAJECTORY_HORIZON_S: f32 = 560.0;

/// The field of view the glass is laid out in, as tan(fov/2): fixed, so
/// the FOV setting changes the view and never the cockpit.
const LAYOUT_TAN: f32 = 0.700_207_5; // 70 degrees

/// A dial's settings once its own are laid over the cockpit's.
#[derive(Debug, Clone, Copy, PartialEq)]
struct DialEffective {
    size: f32,
    style: settings::GaugeStyle,
    stay: bool,
    /// Leaned toward the pilot, radians.
    tilt: f32,
    /// Leaned sideways about its own upright, radians.
    lean: f32,
    /// The face turned in its own plane, radians.
    rotate: f32,
}

/// The ship as it was at a WARP STOP: where the after-image comes from.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Ghost {
    orient: DQuat,
    dir_world: DVec3,
    at_s: f32,
}

/// Something on the glass the gaze can pick up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dragged {
    Dial(Instrument),
    /// The settings menu's text block.
    MenuPanel,
    /// The text readout.
    Readout,
    /// The map pane with its DRIVE panel.
    MapPanel,
    /// The mini map, a gauge on the glass.
    MiniMap,
    /// The holo3PP panel.
    HoloPanel,
    /// The SHIP bay's hologram pane with its card.
    BayPanel,
}

impl Dragged {
    fn name(self) -> &'static str {
        match self {
            Dragged::Dial(i) => i.name(),
            Dragged::MenuPanel => "SETTINGS PANEL",
            Dragged::Readout => "READOUT",
            Dragged::MapPanel => "MAP",
            Dragged::MiniMap => "MINI MAP",
            Dragged::HoloPanel => "HOLO3PP",
            Dragged::BayPanel => "SHIP BAY",
        }
    }
}

/// Something DESIGN mode can select: a dial, or one of the other glass
/// elements — each with its own card and keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DesignEl {
    Dial(Instrument),
    /// The holo3PP: the ship's hologram over the dash.
    Holo,
    /// The mini map pane.
    MiniMap,
    /// The text readout's block.
    Readout,
}

/// What the text readout shows this frame.
/// The gun mounts' gimbal: how far off the nose the gaze can take them.
const GIMBAL_RAD: f64 = 35.0 * std::f64::consts::PI / 180.0;
/// How much of a slug's flight the tracer shows behind its head, s.
const TRACER_TRAIL_S: f64 = 0.06;

struct Readout {
    altitude_m: f64,
    speed_mps: f64,
    assist: bool,
    show: bool,
    /// The wind over the hull: speed (m/s) and its nose-relative arrow.
    wind: Option<(f32, &'static str)>,
    /// The collective's position 0..1 while a helicopter is flown.
    collective: Option<f64>,
    /// The LANDING lines (the approach in landing mode; DOWN or LANDED
    /// whenever the ship is on the ground), newline-separated — or the
    /// next system's line when the ground has nothing to say.
    landing: Option<String>,
}

/// How close (aspect-corrected NDC) the gaze must be to a dial's anchor to
/// pick it up with the left button.
const DRAG_REACH: f32 = 0.30;
/// Speed of sound for the mach instrument and the sonic boom, m/s. The
/// gauge shader hard-codes the same number for its barrier tick (shaders
/// cannot import Rust consts) — change one, change both.
const MACH1_MPS: f64 = 340.0;

/// Runtime knobs, read from the environment so a perf A/B needs no rebuild:
///   FARFALL_MSAA=1|2|4|8   (default 4)
///   FARFALL_VSYNC=on|off   (default on; off uncaps the frame rate so the
///                           renderer's real headroom is measurable rather
///                           than hidden behind the display's refresh rate)
///   FARFALL_GPU_SYNC=1     (profiling only: block until the GPU finishes each
///                           frame before timing it)
///   FARFALL_WINDOWED=1     (start windowed instead of borderless fullscreen)
///   FARFALL_BENCH=1        (freeze the sim at spawn so every run renders the
///                           identical frame — without this the ship drifts and
///                           two perf runs are not comparable. Forces a window
///                           rather than fullscreen and exits by itself, so a
///                           benchmark can never be left sitting on screen
///                           being mistaken for a game with broken controls.
///                           HERMETIC: starts from stock settings — the pilot's
///                           settings.cfg is neither read nor ever written —
///                           and never polls a real stick, so a plugged HOTAS
///                           or a lived-in cockpit cannot colour a capture;
///                           FARFALL_HUD and the knobs below stage looks and
///                           demands deliberately.)
///   FARFALL_BENCH_SECONDS  (how long a benchmark runs before quitting; 20)
///   FARFALL_BENCH_ALT      (frozen altitude in metres; low values are the
///                           worst case, a screen filled edge to edge with
///                           ground, which is where this renderer hurts.
///                           Under 8 km the ship flies at 250 m/s over a
///                           coast instead of orbiting — orbital speed in
///                           thick air lights the entry plasma and fogs the
///                           glass; FARFALL_BENCH_ALT_AT=lat,lon picks the
///                           spot in degrees)
///   FARFALL_BENCH_POS=x,y,z (benchmark only: park the ship at this world
///                           position, nose on the planet — e.g. behind the
///                           Moon, to check what hides what); with
///                           FARFALL_BENCH_VEL=x,y,z for the velocity
///                           (else at rest) and FARFALL_BENCH_LOOK=x,y,z
///   FARFALL_BENCH_ROLL=rad (benchmark only: rolled about the look axis)
///                           for where the nose points (else the planet)
///   FARFALL_BENCH_CRAFT=heli (benchmark only: the pilot's own craft is the
///                           FARFALL helicopter, as the SHIP page's CRAFT
///                           row would choose — SPEC §6.5c)
///   FARFALL_BENCH_SHIP=1   (benchmark only: open the SHIP bay for a capture)
///   FARFALL_BENCH_FIT=n,l,r,b (benchmark only: the four hardpoints' mounts by
///                           key — empty, cannon, rail — in hardpoint order
///                           nose, wing L, wing R, belly; the cockpit, the
///                           bay and the chase view all show it)
///   FARFALL_BENCH_STYLE=k  (benchmark only: the cockpit's gauge style by key — tron, jet, dial, warthog)
///   FARFALL_BENCH_MAP=1    (benchmark only: open the MAP page at once)
///   FARFALL_BENCH_HEAD=y,p (benchmark only: turn the head yaw,pitch degrees)
///   FARFALL_BENCH_LAND=1   (benchmark only: LANDING mode on)
///   FARFALL_BENCH_LANDED=1 (benchmark only: parked on the ground, LANDED,
///                           on its gear with the Sun up the sky)
///   FARFALL_BENCH_HELI=1   (benchmark only: parked LANDED beside the coast
///                           pad's helicopter, the boarding offer up;
///                           =fly: boarded instead, hovering over the pad)
///   FARFALL_BENCH_DISEMBARK=1 (benchmark only: DISEMBARK pressed at once,
///                           for its answer on the readout)
///   FARFALL_BENCH_EVA=1    (benchmark only: parked LANDED and already on
///                           foot — walked out a dozen metres, looking back
///                           at the ship, the suit's readout up)
///   FARFALL_BENCH_DESIGN=1 (benchmark only: DESIGN mode on)
///   FARFALL_BENCH_MENU=n   (benchmark only: the settings menu open, paged n times,
///                           0..8; 2 is the STICK page)
///   FARFALL_BENCH_STICK=n  (benchmark only: the stick wizard open at step n
///                           (0-based), with a stand-in detection on it)
///   FARFALL_BENCH_PROFILE=reforger (benchmark only: the stick wears the
///                           Reforger helicopter-pilot profile for the run,
///                           for the PROFILE row's worn value)
///   FARFALL_BENCH_DEMAND=p,r,y,t (benchmark only: a parked pitch/roll/yaw/
///                           throttle demand, for the console stick's mirror)
///   FARFALL_BENCH_CARD=1   (benchmark only: the CONTROLS card up, as on the first run)
///   FARFALL_BENCH_HYPER=1  (benchmark only: the hyper drive's field fully up)
///   FARFALL_BENCH_NEBULA=1|off (benchmark only: a full sky of nebula at
///                           twice the stock glow, or off for a baseline)
///   FARFALL_BENCH_DUST=k   (benchmark only: the DUST setting for this run,
///                           0..2 — the motes and space dust on their own)
///   FARFALL_BENCH_GHOST=age (benchmark only: a WARP STOP after-image this
///                           many seconds old AT THE CAPTURE — halfway
///                           through the bench — ahead and a little banked;
///                           0.5 is a good look, it lives 1.8 s)
///   FARFALL_BENCH_ARMS=nosight (benchmark only: the sight off, for a baseline)
///   FARFALL_BENCH_ARMS=sight (benchmark only: the cannon hot, as if firing:
///                           pair with FARFALL_BENCH_HEAD past the gimbal to
///                           see the sight held on the ring)
///   FARFALL_BENCH_ARMS=scars (benchmark only: three craters, hot to cold, on
///                           the nearest rock ahead)
///   FARFALL_BENCH_ARMS=debris (benchmark only: a rock's shards ahead, fresh
///                           ones glowing, with the break's burst)
///   FARFALL_BENCH_ARMS=1   (benchmark only: tracers from both guns, a muzzle
///                           flash and every kind of burst ahead)
///   FARFALL_BENCH_STRIKES=n (benchmark only: n strikes on the shield at
///                           staggered ages at the capture, for its ripples)
///   FARFALL_BENCH_CLOUDS=k (benchmark only: the CLOUDS setting for this run,
///                           0 clears the deck — to see the air and the
///                           ground on their own)
///   FARFALL_BENCH_WIND=mps[,deg] (benchmark only: force the wind the
///                           ribbons and the WIND readout show to this
///                           speed, blowing FROM deg degrees off the nose
///                           — 0 a headwind, 90 from the right; 90 if
///                           omitted. Visuals only: a bench's sim is
///                           frozen and never feels it)
///   FARFALL_BENCH_MIMIC=reveal|hail|attack|wreck (benchmark only: a ship
///                           out of a rock ahead-left in that state)
///   FARFALL_BENCH_MINERS=tier[,mine|fight] (benchmark only: a miner of
///                           tier 0..3 ahead-left, its beam on a planted
///                           rock, or come about with its fire in the air;
///                           a second one far off as a speck)
///   FARFALL_BENCH_SHAKE=y,p,r (benchmark only: the helmet camera parked at
///                           this yaw, pitch, roll in degrees)
///   FARFALL_BENCH_G=x,y,z  (benchmark only: a felt load in g — right, up,
///                           forward — for the G instruments)
///   FARFALL_BENCH_THRUST=m,p,y,r (benchmark only: force main thrust 0..1 and
///                           pitch/yaw/roll demands -1..1, for the plumes)
///   FARFALL_BENCH_FULL=1   (benchmark only: borderless fullscreen, the real
///                           pixel count)
///   FARFALL_BENCH_SIZE=w,h (benchmark only: a window of exactly this many
///                           pixels, bigger than the display if need be — the
///                           2880x1800 floor measured on a 1080p desk)
///   FARFALL_BENCH_SPIN=n   (benchmark only: the head turns a full circle over
///                           the bench and n frames are captured on the way —
///                           a look round the whole cabin)
///   FARFALL_CAPTURE=final  (screenshots take the presented frame, with the
///                           post pass, the map and the text, instead of the
///                           scene target)
///   FARFALL_SCALE=0.25..1  (scene render scale; the HUD stays native)
///   FARFALL_FOV=50..110    (vertical field of view in degrees for this run,
///                           over the settings file's graphics.fov)
///   FARFALL_HUD=path       (wear a saved HUD layout file (.fhud) for this
///                           run — see crates/app/src/hud_file.rs)
///   FARFALL_MUTE=1         (no audio stream at all)
///   FARFALL_BENCH_WARP=s   (benchmark only: engage the wormhole drive s
///                           seconds in, so the sequence can be captured)
///   FARFALL_BENCH_SAVE=path (benchmark only: write the world file to this
///                           EXACT path, never ~/.farfall, at the bench's
///                           own exit — a real sealed save of a real
///                           parked world, for scripted e2e verification)
///   FARFALL_BENCH_RESUME=path (benchmark only: load and restore a world
///                           file from this EXACT path, never ~/.farfall,
///                           through the same seal check as a real
///                           resume; a tampered file is refused and
///                           logged, and the bench stays at the stock
///                           orbit, same as any other refusal. The bench
///                           stays frozen either way.)
///   FARFALL_RESUME=0|off|false (turn RESUME off for this run only, whatever
///                           the setting says: the world file is neither
///                           read nor written. There is no value that
///                           forces it ON over the setting. Unrelated to
///                           FARFALL_BENCH_SAVE/RESUME above, which never
///                           touch ~/.farfall regardless of this.)
///   FARFALL_SKIP=a,b       (profiling only: leave out passes by name —
///                           starfield, bodies, planet, plasma, trajectory, cockpit, gauge,
///                           post (the picture: one plain fetch instead), bloom (the chain),
///                           hud, blit, dust, wind —
///                           so each one's cost can be measured by its absence)
struct Config {
    msaa: u32,
    vsync: bool,
    gpu_sync: bool,
    windowed: bool,
    bench: bool,
    bench_seconds: f64,
    scale: f32,
    skip: Vec<String>,
    bench_warp_at: Option<f64>,
    /// FARFALL_BENCH_SIZE=w,h: the bench window's size in pixels.
    bench_size: Option<(u32, u32)>,
    /// FARFALL_BENCH_SPIN=n: the head turns a full circle over the bench
    /// and n frames are captured on the way round.
    bench_spin: u32,
    /// Born into a headset's Vulkan device this run, instead of the flat
    /// one: VR HEADSET in the settings, overridden by `FARFALL_VR`. Read
    /// only by the native `init_gpu` path — native VR has no web
    /// equivalent (WebXR drives `Game::vr` from the page instead, see
    /// `web.rs::xr_frame`), so this is dead weight on that build.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    vr: bool,
    /// A factor on the OpenXR runtime's recommended per-eye render size.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    vr_scale: f32,
    /// FARFALL_VR=synth: a synthetic Valve-Index-shaped headset needing
    /// no OpenXR runtime (SPEC §5.3) — `vr` is also true whenever this
    /// is, so every other `vr`-gated path (device setup, instrument
    /// placement, `xr_composite`) runs unchanged; only `xr::init` itself
    /// is skipped in favour of `xr::init_synth`.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    vr_synth: bool,
    /// FARFALL_VR_SCRIPT: the synthetic headset's deterministic head
    /// motion. `xr::HeadScript` only exists off wasm32 (`mod xr` is
    /// native-only, WebXR drives `Game::vr` from the page instead), so
    /// this field is too — unused off a synthetic headset either way.
    #[cfg(not(target_arch = "wasm32"))]
    vr_script: xr::HeadScript,
    /// FARFALL_VR_MIRROR=pair: the desktop mirror shows both cropped
    /// eyes side by side — exactly what the headset sees, provable
    /// before anyone wears it — instead of the default single letterboxed
    /// eye.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    vr_mirror_pair: bool,
    /// FARFALL_VR_LABEL=1: stamp a big "L"/"R" into each eye's own
    /// swapchain image, so the headset side (not only the mirror) proves
    /// which eye is which.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    vr_label: bool,
    /// Render (and, under FARFALL_CAPTURE, save) a VR frame even when
    /// the runtime says `should_render` is false — a bench run or a
    /// desk-side capture needs a picture whether or not the compositor
    /// currently wants one submitted; the frame is still rendered either
    /// way; only the submission to the OpenXR swapchain is skipped when
    /// the runtime didn't ask for one.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    vr_force_render: bool,
}

impl Config {
    fn draws(&self, pass: &str) -> bool {
        !self.skip.iter().any(|s| s == pass)
    }

    /// Environment knobs over the settings file: the file is the pilot's
    /// choice, the variables are the profiler's, and the profiler wins.
    fn from_env(settings: &Settings) -> Self {
        let msaa = std::env::var("FARFALL_MSAA")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|n| matches!(n, 1 | 2 | 4 | 8))
            .unwrap_or(settings.msaa);
        let vsync = match std::env::var("FARFALL_VSYNC").as_deref() {
            Ok("off" | "0" | "false") => false,
            Ok("on" | "1" | "true") => true,
            _ => settings.vsync,
        };
        // Without this, CPU-side frame timing measures how fast we can *submit*
        // work, not how long the GPU takes: submission is asynchronous, so the
        // CPU runs ahead and reports sub-millisecond "frames" while the GPU is
        // still busy. Blocking on completion makes the wall clock mean
        // something. It costs pipelining, so it is a profiling mode, not a
        // default.
        let gpu_sync = matches!(
            std::env::var("FARFALL_GPU_SYNC").as_deref(),
            Ok("1" | "on" | "true")
        );
        let mut windowed = matches!(
            std::env::var("FARFALL_WINDOWED").as_deref(),
            Ok("1" | "on" | "true")
        );
        // Screen coverage is what this renderer costs, and coverage depends on
        // altitude — so a moving ship makes every measurement a different
        // scene. Freezing it is the difference between profiling and guessing.
        let bench = matches!(
            std::env::var("FARFALL_BENCH").as_deref(),
            Ok("1" | "on" | "true")
        );
        if bench && std::env::var("FARFALL_BENCH_FULL").is_err() {
            // Not fullscreen: a benchmark must be visibly not the game —
            // unless asked, because the display's own size is the number
            // that matters.
            windowed = true;
        }
        // A window of a given size, which may be bigger than the display:
        // the 2880×1800 floor measured on a 1080p desk.
        let bench_size = std::env::var("FARFALL_BENCH_SIZE")
            .ok()
            .filter(|_| bench)
            .and_then(|v| {
                let (w, h) = v.split_once(',')?;
                Some((w.trim().parse::<u32>().ok()?, h.trim().parse::<u32>().ok()?))
            })
            .filter(|&(w, h)| w >= 64 && h >= 64);
        if bench_size.is_some() {
            windowed = true;
        }
        let bench_seconds = std::env::var("FARFALL_BENCH_SECONDS")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(20.0);
        let scale = std::env::var("FARFALL_SCALE")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(settings.scale)
            .clamp(0.25, 1.0);
        // The profiler's own knob wins over the pilot's: `FARFALL_VR=0`
        // overrides VR HEADSET off without touching the file, and a
        // plain bench (no FARFALL_VR set at all) defaults to flat, since
        // most benches never want a headset — but `FARFALL_BENCH=1
        // FARFALL_VR=1` is the VR bench (SPEC §5.3): a deaf run inside
        // the OpenXR session, explicit VR always wins over a bare bench.
        // FARFALL_VR=synth: a synthetic headset, needing no OpenXR
        // runtime — for benching the whole VR pipeline (comfort/depth/
        // eye-order self-checks included) on any machine, any time
        // (SPEC §5.3). `vr` is also true here, so every other `vr`-gated
        // path runs exactly as it would for a real headset.
        let vr_synth = std::env::var("FARFALL_VR").as_deref() == Ok("synth");
        let vr = vr_synth
            || match std::env::var("FARFALL_VR").as_deref() {
                Ok("1" | "on" | "true") => true,
                Ok("0" | "off" | "false") => false,
                _ => settings.vr_headset && !bench,
            };
        #[cfg(not(target_arch = "wasm32"))]
        let vr_script = xr::HeadScript::parse(
            std::env::var("FARFALL_VR_SCRIPT")
                .ok()
                .as_deref()
                .unwrap_or(""),
        );
        let vr_scale = std::env::var("FARFALL_VR_SCALE")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|f| f.is_finite())
            .unwrap_or(settings.vr_scale)
            .clamp(settings::VR_SCALE_MIN, settings::VR_SCALE_MAX);
        // A VR bench always wants the labelled pair, not a single
        // letterboxed eye — its captures are that content specifically
        // (see below).
        let vr_mirror_pair = bench || std::env::var("FARFALL_VR_MIRROR").as_deref() == Ok("pair");
        let vr_label = matches!(
            std::env::var("FARFALL_VR_LABEL").as_deref(),
            Ok("1" | "on" | "true")
        );
        // A desk-side capture needs a picture with nobody wearing the
        // headset: the runtime may leave `should_render` false while the
        // session sits VISIBLE-but-unworn, and a bench (queued
        // separately) needs one every frame regardless.
        let vr_force_render = bench || capture_final();
        Self {
            msaa,
            vsync,
            gpu_sync,
            windowed,
            bench,
            bench_seconds,
            vr,
            vr_scale,
            vr_synth,
            #[cfg(not(target_arch = "wasm32"))]
            vr_script,
            vr_mirror_pair,
            vr_label,
            vr_force_render,
            bench_warp_at: std::env::var("FARFALL_BENCH_WARP")
                .ok()
                .and_then(|v| v.parse::<f64>().ok()),
            bench_spin: std::env::var("FARFALL_BENCH_SPIN")
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(0)
                .min(64),
            skip: std::env::var("FARFALL_SKIP")
                .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_default(),
            scale,
            bench_size,
        }
    }
}

/// Presentation-side timing. Separate from the sim clock: the sim advances in
/// fixed steps regardless of how long a frame took, and conflating the two
/// would make the perf numbers a function of the physics rate.
struct Perf {
    stats: FrameStats,
    /// Time spent on the CPU building and encoding the frame, measured
    /// separately from the wall-clock frame time. The difference between the
    /// two is the GPU (plus any vsync wait), and keeping them apart is the only
    /// way to know which half is actually the bottleneck.
    cpu: FrameStats,
    /// Time blocked acquiring a swapchain image — GPU and vsync, not CPU.
    wait: FrameStats,
    /// VR only: CPU encode plus real GPU frame time (a forced gpu_sync
    /// poll's own duration, added in) — the bench stamp's `render_ms`.
    render: FrameStats,
    last_frame: Instant,
    last_log: Instant,
    last_title: Instant,
}

impl Perf {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            stats: FrameStats::default(),
            cpu: FrameStats::default(),
            wait: FrameStats::default(),
            render: FrameStats::default(),
            last_frame: now,
            last_log: now,
            last_title: now,
        }
    }
}

/// Every pass that renders into the scene target — all of whose pipelines
/// depend on the MSAA count, so the menu rebuilds them together.
struct Passes {
    starfield: StarfieldPass,
    bodies: BodiesPass,
    planet: PlanetPass,
    gauge: GaugePass,
    alt_gauge: GaugePass,
    g_gauge: GaugePass,
    gvec: GaugePass,
    gyro: InstrumentPass,
    horizon: InstrumentPass,
    /// The design guide overlay.
    guide: InstrumentPass,
    /// The wireframe cabin around the head.
    cabin: farfall_render::cabin::CabinPass,
    /// The hull heat field, simulated on the GPU, and the sheath it lights.
    thermal: ThermalPass,
    plasma: PlasmaPass,
    /// The predicted path, integrated on the GPU.
    trajectory: TrajectoryPass,
    shield: ShieldPass,
    ghost: GhostPass,
    belt: BeltPass,
    /// The ship from outside, for the chase view and the holo3PP.
    jet: JetPass,
    /// The holo3PP: the volumetric hologram over the dash.
    holo: HoloPass,
    /// The arms' light: tracers, flashes, bursts.
    tracer: TracerPass,
    /// The arms' debris: shards off the rocks.
    debris: DebrisPass,
    /// Space dust and the cabin's motes.
    dust: DustPass,
    /// The planet's wind made visible: ribbons of moving air.
    wind: WindPass,
    /// The arms' scars: craters glowing on the rocks.
    scar: ScarPass,
    /// The gun sight on the glass.
    sight: SightPass,
    /// The mimics: ships out of the rocks.
    mimic: MimicPass,
    /// The helicopters on their pads down on the planet.
    heli: HeliPass,
}

impl Passes {
    fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        msaa: u32,
        baked: &BakedMaps,
        nebula: &wgpu::TextureView,
        cabin_res: f32,
    ) -> Self {
        // Two targets, one sample count: everything outside the glass
        // writes radiance into the HDR world; the cabin, the dials and the
        // holo3PP draw after the post pass into the 8-bit ship target.
        let world = SceneTarget::WORLD_FORMAT;
        let thermal = ThermalPass::new(device);
        let plasma = PlasmaPass::new(device, world, msaa, &thermal, baked);
        Self {
            starfield: StarfieldPass::new(device, world, msaa, STAR_DENSITY, baked, nebula),
            bodies: BodiesPass::new(device, world, msaa),
            planet: PlanetPass::new(device, world, msaa, baked),
            gauge: gauge_pass(device, format, msaa),
            alt_gauge: gauge_pass(device, format, msaa),
            g_gauge: gauge_pass(device, format, msaa),
            gvec: gvec_pass(device, format, msaa),
            gyro: gyro_pass(device, format, msaa),
            horizon: horizon_pass(device, format, msaa),
            guide: guide_pass(device, format, msaa),
            cabin: farfall_render::cabin::CabinPass::new(device, format, msaa, cabin_res),
            thermal,
            plasma,
            trajectory: TrajectoryPass::new(device, world, msaa),
            shield: shield_pass(device, world, msaa),
            ghost: ghost_pass(device, world, msaa),
            belt: belt_pass(device, world, msaa),
            jet: jet_pass(device, world, msaa),
            holo: holo_pass(device, format, msaa),
            tracer: tracer_pass(device, world, msaa),
            debris: debris_pass(device, world, msaa),
            dust: DustPass::new(device, world, format, msaa),
            wind: WindPass::new(device, world, msaa),
            scar: scar_pass(device, world, msaa),
            sight: sight_pass(device, format, msaa),
            mimic: mimic_pass(device, world, msaa),
            heli: heli_pass(device, world, msaa),
        }
    }
}

impl Gpu {
    /// A native VR session's own eye size, decoupled from the mirror
    /// window's — `None` on every build without one (always, on the
    /// web build, which has no `xr` field at all). Kept as a method
    /// rather than a bare field read so `redraw`'s VR branches need no
    /// `#[cfg]` of their own.
    #[cfg(not(target_arch = "wasm32"))]
    fn xr_eye_size(&self) -> Option<(u32, u32)> {
        self.xr.as_ref().map(xr::XrSession::eye_size)
    }
    #[cfg(target_arch = "wasm32")]
    fn xr_eye_size(&self) -> Option<(u32, u32)> {
        None
    }

    /// The offscreen stereo pair's view, when native VR is drawing one.
    #[cfg(not(target_arch = "wasm32"))]
    fn vr_pair_view(&self) -> Option<&wgpu::TextureView> {
        self.vr_pair.as_ref().map(|p| &p.view)
    }
    #[cfg(target_arch = "wasm32")]
    fn vr_pair_view(&self) -> Option<&wgpu::TextureView> {
        None
    }

    /// This frame's per-eye render size (SPEC §5.3: the runtime's own
    /// recommended size, inflated by the hull-vs-true ratio from the
    /// real tangents — `xr::eye_render_size`), resizing the pair to
    /// match if it isn't already there. `None` on every build without a
    /// native session.
    #[cfg(not(target_arch = "wasm32"))]
    fn ensure_vr_render_size(&mut self, tans: [[f32; 4]; 2]) -> Option<(u32, u32)> {
        let recommended = self.xr.as_ref()?.recommended_size();
        let render_size = xr::eye_render_size(recommended, tans, self.cfg.vr_scale);
        if self.vr_pair.as_mut()?.ensure(&self.device, render_size) {
            // The factors themselves, not just the pixel count they
            // produced: "is this headset's own lens asymmetry really
            // this many percent" is a question the raw tangents answer
            // directly, so log them rather than making a reviewer back
            // them out of before/after sizes.
            let (fx, fy) = xr::hull_over_true_factors(tans);
            log::info!(
                "VR: render resized to {}x{} per eye (hull/true factor x={fx:.3} y={fy:.3}, \
                 eye 0 tan L/R/U/D {:.3}/{:.3}/{:.3}/{:.3}, eye 1 {:.3}/{:.3}/{:.3}/{:.3})",
                render_size.0,
                render_size.1,
                tans[0][0],
                tans[0][1],
                tans[0][2],
                tans[0][3],
                tans[1][0],
                tans[1][1],
                tans[1][2],
                tans[1][3],
            );
        }
        Some(render_size)
    }
    #[cfg(target_arch = "wasm32")]
    fn ensure_vr_render_size(&mut self, _tans: [[f32; 4]; 2]) -> Option<(u32, u32)> {
        None
    }

    /// The runtime's own current display rate, Hz — the VR bench
    /// stamp's `hz=`. `None` off any build without a native session.
    #[cfg(not(target_arch = "wasm32"))]
    fn xr_display_refresh_hz(&self) -> Option<f32> {
        self.xr.as_ref().and_then(xr::XrSession::display_refresh_hz)
    }
    #[cfg(target_arch = "wasm32")]
    fn xr_display_refresh_hz(&self) -> Option<f32> {
        None
    }

    /// Wall-clock ms the most recent VR frame spent inside
    /// `FrameWaiter::wait` — the VR bench stamp's `xr_wait_ms=`.
    #[cfg(not(target_arch = "wasm32"))]
    fn xr_last_wait_ms(&self) -> f32 {
        self.xr
            .as_ref()
            .map(xr::XrSession::last_wait_ms)
            .unwrap_or(0.0)
    }
    #[cfg(target_arch = "wasm32")]
    fn xr_last_wait_ms(&self) -> f32 {
        0.0
    }

    /// Whether the active native session is the synthetic bench headset
    /// (`FARFALL_VR=synth`), not a real runtime — the VR bench stamp's
    /// `synth=`. `false` off any build without a native session.
    #[cfg(not(target_arch = "wasm32"))]
    fn xr_is_synth(&self) -> bool {
        self.xr.as_ref().is_some_and(xr::XrSession::is_synth)
    }
    #[cfg(target_arch = "wasm32")]
    fn xr_is_synth(&self) -> bool {
        false
    }

    /// The scene textures were recreated: point the post pass at the new
    /// world and the blit at the new ship target, or they sample a
    /// destroyed view.
    fn rebind_scene(&mut self) {
        self.post.rebind(&self.device, &self.scene);
        if let Some(view) = self.scene.colour_view() {
            self.blit.rebind(&self.device, view);
        }
    }

    /// The post pass's uniforms for this frame: the drive's look on the
    /// world, the picture settings, the exposure's drift.
    fn update_post(&self, game: &mut Game, aspect: f32, time_s: f32) {
        let l = game.warp_look();
        let s = &game.settings;
        let look = farfall_render::post::Look {
            bloom: s.bloom,
            exposure: s.exposure,
            tonemap: s.tonemap,
            fringe: s.fringe,
        };
        self.post.update(
            &self.queue,
            &PostUniforms::new(l.fisheye, l.invert, l.particles, l.charge, aspect, time_s)
                .with_speed(game.speed_look())
                .with_drive(l.stretch, l.pull, l.reform)
                .with_look(&look)
                .with_adapt_blend(self.post.adapt_blend(game.frame_dt))
                .with_bypass(!self.cfg.draws("post")),
        );
    }
    /// The holo3PP's frame: the miniature's scene in the ship's frame,
    /// seen from the pilot's head.
    fn update_holo(&self, game: &Game, aspect: f32) {
        let pose = game.pose(aspect);
        self.passes
            .holo
            .update(&self.queue, &game.holo_uniforms(&pose));
    }
}

struct Gpu {
    /// The native VR session, when one is up: declared first so Rust's
    /// field-order drop tears it down (and the swapchains/session bound
    /// to `device` with it) before `device`/`queue` below are dropped.
    #[cfg(not(target_arch = "wasm32"))]
    xr: Option<xr::XrSession>,
    window: Arc<Window>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    scene: SceneTarget,
    /// The stereo pair, offscreen, when native VR is active: each eye
    /// drawn at its own symmetric frustum into its half, then cropped
    /// into the headset's own swapchain images and mirrored into the
    /// window (SPEC §5.3).
    #[cfg(not(target_arch = "wasm32"))]
    vr_pair: Option<VrPair>,
    /// Set once the eye-order self-check has run (`xr_composite`) — a
    /// GPU readback, so it runs exactly once per session, on the first
    /// labelled composite, not every frame.
    #[cfg(not(target_arch = "wasm32"))]
    eye_order_checked: bool,
    /// Set once the overlay-depth self-check has logged its measured
    /// per-eye tangent shift (`update_instruments`, which only takes
    /// `&self`) — a pure measurement, but only worth a log line once a
    /// session.
    #[cfg(not(target_arch = "wasm32"))]
    overlay_depth_logged: std::cell::Cell<bool>,
    /// The picture: bloom, exposure, tonemap and the drive's distortion,
    /// done to the world before the ship is drawn over it.
    post: PostPass,
    blit: BlitPass,
    passes: Passes,
    /// Owns the baked textures the passes sample.
    baked: BakedMaps,
    /// The nebula's bake: re-rendered when its knobs change, sampled by the
    /// starfield.
    nebula: NebulaBake,
    hud: HudPass,
    /// The system map pane, native resolution like the text.
    map: InstrumentPass,
    /// The SHIP bay's hologram pane.
    hologram: HologramPass,
    /// Where each dial pass may draw this frame (target pixels): speed,
    /// altitude, g-meter, g-vector, gyro. None: hidden, not drawn.
    dial_rects: std::cell::Cell<[Option<[u32; 4]>; 5]>,
    /// The panels' mouse pointer.
    pointer: PointerPass,
    text: TextBitmap,
    cfg: Config,
    perf: Perf,
    /// AUTO SCALE's factor on the world's scale, 0.35..1: lowered when the
    /// frame misses the floor, raised back when there is room.
    auto_scale: f32,
    /// When the governor last moved.
    auto_scale_at: Instant,
    /// Set by a key press; consumed by the next frame.
    capture_requested: bool,
    bench_captured: bool,
    /// Spin frames taken so far.
    bench_spin_taken: u32,
}

/// Native VR's offscreen stereo pair and the two cut-out blits that read
/// it: one per eye's OpenXR swapchain image (the true asymmetric crop),
/// and one for the mirror window (the left eye's half, uncropped).
#[cfg(not(target_arch = "wasm32"))]
struct VrPair {
    view: wgpu::TextureView,
    /// (2 * one eye's width, one eye's height).
    size: (u32, u32),
    surface_format: wgpu::TextureFormat,
    to_swapchain: farfall_render::blit_xr::XrBlitPass,
    to_window: farfall_render::blit_xr::XrBlitPass,
    /// FARFALL_VR_MIRROR=pair: a second window-format blit, rebound each
    /// frame to whichever swapchain image was just cropped (unlike
    /// `to_window`, which is bound once to the pair itself) — the
    /// mirror then shows exactly what the headset does, labels
    /// included, instead of re-deriving its own crop.
    mirror_swap: farfall_render::blit_xr::XrBlitPass,
    /// FARFALL_VR_LABEL=1: a big "L"/"R" stamped into each eye's own
    /// swapchain image, so the headset itself (not just the mirror)
    /// proves which eye is which before anyone puts it on.
    label_bitmap: farfall_render::text::TextBitmap,
    /// One `HudPass` per eye, not one shared: both eyes' label passes
    /// are recorded into the SAME encoder before the single
    /// `queue.submit()` at the end of `xr_composite`, and
    /// `HudPass::update` writes its uniform buffer via
    /// `queue.write_buffer` — a queued write, not one interleaved with
    /// the encoder's own recorded commands. Two `update()` calls to one
    /// shared buffer before one submit both land before either render
    /// pass actually runs on the GPU, so the *second* write (eye 1's
    /// "R", eye 1's own anchor) is what BOTH passes read — exactly the
    /// bug a synth capture caught: a giant "R" in both eyes, positioned
    /// by eye 1's own shift even inside eye 0's image. A distinct
    /// buffer per eye makes the two writes independent regardless of
    /// submission timing.
    label_hud: [farfall_render::hud::HudPass; 2],
}

#[cfg(not(target_arch = "wasm32"))]
impl VrPair {
    fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        xr_format: wgpu::TextureFormat,
        eye_size: (u32, u32),
    ) -> Self {
        let size = (eye_size.0 * 2, eye_size.1);
        let view = Self::make_texture(device, surface_format, size);
        let mut to_swapchain = farfall_render::blit_xr::XrBlitPass::new(device, xr_format);
        to_swapchain.rebind(device, &view);
        let mut to_window = farfall_render::blit_xr::XrBlitPass::new(device, surface_format);
        to_window.rebind(device, &view);
        // Bound to the pair for now; MIRROR=pair rebinds this to each
        // swapchain image in turn every frame it is used.
        let mut mirror_swap = farfall_render::blit_xr::XrBlitPass::new(device, surface_format);
        mirror_swap.rebind(device, &view);
        Self {
            view,
            size,
            surface_format,
            to_swapchain,
            to_window,
            mirror_swap,
            label_bitmap: farfall_render::text::TextBitmap::new(),
            label_hud: [
                farfall_render::hud::HudPass::new(device, xr_format, 1),
                farfall_render::hud::HudPass::new(device, xr_format, 1),
            ],
        }
    }

    fn make_texture(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: (u32, u32),
    ) -> wgpu::TextureView {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vr pair"),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// Resize the pair to `2 * eye_render_size` if it isn't already
    /// there — Index-matching (SPEC §5.3): the render itself has to be
    /// bigger than the runtime's own recommended size, by the
    /// hull-vs-true ratio (`eye_render_size`), and that ratio is only
    /// known once the first real frame's tangents arrive, so this runs
    /// every frame and only actually reallocates the one time the size
    /// changes (normally once, ever, for a fixed headset). Returns
    /// whether it reallocated, matching `SceneTarget::ensure`'s own
    /// convention.
    fn ensure(&mut self, device: &wgpu::Device, render_size: (u32, u32)) -> bool {
        let size = (render_size.0 * 2, render_size.1);
        if size == self.size {
            return false;
        }
        self.view = Self::make_texture(device, self.surface_format, size);
        self.size = size;
        self.to_swapchain.rebind(device, &self.view);
        self.to_window.rebind(device, &self.view);
        true
    }
}

impl Gpu {
    /// Every dial and overlay, from the cockpit layout: an instrument whose
    /// slot is Off gets visibility zero and draws nothing at all.
    /// `eye_ship`: the pose's own eye seat in the ship's frame
    /// (`pose.eye_ship`) — the same one the cabin's ray marcher already
    /// draws from (`CabinUniforms::with_eye`) — so a DIAL's face plane
    /// ray-casts from the same seat as the ray-marched bezel around it,
    /// not a shared, disparity-free ship origin (SPEC §5.3).
    fn update_instruments(
        &self,
        game: &Game,
        cam: &CameraFrame,
        aspect: f32,
        altitude_m: f32,
        eye_ship: Vec3,
    ) {
        let layout = &game.settings.layout;
        let h = self.scene.size().1 as f32;
        let sway = game.holo_sway.sway();
        let look = &game.look;
        let glass_head = game.glass_head();
        let glass_eye_pos = game.glass_eye_pos();
        let ref_tan = game.ref_tan();
        // Overlay-depth self-check: once a session, measure (not just
        // assert in a unit test) that the two eyes' own on_glass shift
        // actually differ — proof the live per-frame eye positions are
        // really reaching reproject_with_eye, not just the pure maths
        // in isolation (SPEC §5.3).
        #[cfg(not(target_arch = "wasm32"))]
        if !self.overlay_depth_logged.get() {
            if let Some(vr) = &game.vr {
                self.overlay_depth_logged.set(true);
                let t = (cam.fov_y * 0.5).tan();
                let shift = |eye_pos: Vec3| on_glass(glass_head, eye_pos, cam, ref_tan, [0.0, 0.0]);
                let s0 = shift(vr.eyes[0].pos);
                let s1 = shift(vr.eyes[1].pos);
                let diff = ((s0[0] - s1[0]).powi(2) + (s0[1] - s1[1]).powi(2)).sqrt();
                let ipd = (vr.eyes[1].pos - vr.eyes[0].pos).length();
                let expected = ipd / (VR_HUD_DISTANCE_M * t.max(1e-4) * cam.aspect.max(1e-4));
                log::info!(
                    "VR: overlay-depth self-check: eyes' on_glass shift differs by \
                     {diff:.5} NDC units at hud-distance {VR_HUD_DISTANCE_M}m (IPD \
                     {ipd:.4}m, ~{expected:.5} expected)"
                );
            }
        }
        // Each dial's own style, size and fade, over the cockpit's.
        let tweak = |i: Instrument| game.dial_tweak(i);
        let fade = |i: Instrument, level: f32| if tweak(i).stay { 1.0 } else { level };
        let jet = |i: Instrument| tweak(i).style == settings::GaugeStyle::Jet;
        let warthog = |i: Instrument| tweak(i).style == settings::GaugeStyle::Warthog;
        // Placement: in the dash for a DIAL (under its hologram's
        // direction, at its size), else on the glass at its size — which
        // also shrinks with a wider live field of view, as a fixed object
        // would.
        let t = (cam.fov_y * 0.5).tan();
        let head = game.head();
        let fov_scale = ref_tan / t.max(1e-4);
        let placed = |i: Instrument| -> Option<farfall_render::cabin::Placement> {
            let tw = tweak(i);
            if matches!(
                tw.style,
                settings::GaugeStyle::Dial | settings::GaugeStyle::Warthog
            ) {
                let a = layout.anchor(i)?;
                let dir = farfall_render::cabin::anchor_direction(a, ref_tan, cam.aspect);
                if let Some(p) = farfall_render::cabin::Placement::in_dash(
                    head, t, dir, tw.size, tw.tilt, tw.lean, eye_ship,
                ) {
                    return Some(p);
                }
            }
            Some(farfall_render::cabin::Placement::glass_sized(tw.size * fov_scale).tilted(tw.tilt))
        };
        let (speed_anchor, speed_on) = slot_of(
            layout,
            glass_head,
            glass_eye_pos,
            cam,
            ref_tan,
            Instrument::Speed,
        );
        self.passes.gauge.update(
            &self.queue,
            &GaugeUniforms::speed(
                game.state.ship.vel_mps.length() as f32,
                fade(Instrument::Speed, game.gauge_fade.level()) * speed_on,
                cam.time_s,
                aspect,
                h,
                speed_anchor,
                sway,
                game.mach(),
                game.mach_alert.level() * speed_on,
            )
            .jet(jet(Instrument::Speed))
            .warthog(warthog(Instrument::Speed))
            .placed(placed(Instrument::Speed))
            .oriented(
                tweak(Instrument::Speed).lean,
                tweak(Instrument::Speed).rotate,
            ),
        );
        let (alt_anchor, alt_on) = slot_of(
            layout,
            glass_head,
            glass_eye_pos,
            cam,
            ref_tan,
            Instrument::Altitude,
        );
        self.passes.alt_gauge.update(
            &self.queue,
            &GaugeUniforms::altitude(
                altitude_m,
                fade(Instrument::Altitude, game.alt_fade.level()) * alt_on,
                cam.time_s,
                aspect,
                h,
                alt_anchor,
                sway,
            )
            .jet(jet(Instrument::Altitude))
            .warthog(warthog(Instrument::Altitude))
            .placed(placed(Instrument::Altitude))
            .oriented(
                tweak(Instrument::Altitude).lean,
                tweak(Instrument::Altitude).rotate,
            ),
        );
        let (g_anchor, g_on) = slot_of(
            layout,
            glass_head,
            glass_eye_pos,
            cam,
            ref_tan,
            Instrument::GForce,
        );
        self.passes.g_gauge.update(
            &self.queue,
            &GaugeUniforms::g_force(
                game.felt_g,
                fade(Instrument::GForce, game.g_fade.level()) * g_on,
                cam.time_s,
                aspect,
                h,
                g_anchor,
                sway,
            )
            .jet(jet(Instrument::GForce))
            .warthog(warthog(Instrument::GForce))
            .placed(placed(Instrument::GForce))
            .oriented(
                tweak(Instrument::GForce).lean,
                tweak(Instrument::GForce).rotate,
            ),
        );
        let (gv_anchor, gv_on) = slot_of(
            layout,
            glass_head,
            glass_eye_pos,
            cam,
            ref_tan,
            Instrument::GVector,
        );
        self.passes.gvec.update(
            &self.queue,
            &GaugeUniforms::g_vector(
                game.felt_g_body,
                fade(Instrument::GVector, game.g_fade.level()) * gv_on,
                cam.time_s,
                aspect,
                h,
                gv_anchor,
                sway,
            )
            .jet(jet(Instrument::GVector))
            .warthog(warthog(Instrument::GVector))
            .placed(placed(Instrument::GVector))
            .oriented(
                tweak(Instrument::GVector).lean,
                tweak(Instrument::GVector).rotate,
            ),
        );
        let (gyro_anchor, gyro_on) = slot_of(
            layout,
            glass_head,
            glass_eye_pos,
            cam,
            ref_tan,
            Instrument::Gyro,
        );
        // Each dial's patch of the target: a full-screen pass that discards
        // is not free, so the dials draw only where they are.
        let size = self.scene.size();
        let rect = |a: [f32; 2], on: f32, i: Instrument| {
            dial_scissor(a, on, tweak(i).size * fov_scale, aspect, size)
        };
        self.dial_rects.set([
            rect(speed_anchor, speed_on, Instrument::Speed),
            rect(alt_anchor, alt_on, Instrument::Altitude),
            rect(g_anchor, g_on, Instrument::GForce),
            rect(gv_anchor, gv_on, Instrument::GVector),
            rect(gyro_anchor, gyro_on, Instrument::Gyro),
        ]);
        self.passes.gyro.update(
            &self.queue,
            &GyroUniforms::new(
                game.attitude(),
                gyro_on,
                aspect,
                h,
                gyro_anchor,
                sway,
                cam.time_s,
            )
            .jet(jet(Instrument::Gyro))
            .warthog(warthog(Instrument::Gyro))
            .placed(placed(Instrument::Gyro))
            .oriented(tweak(Instrument::Gyro).lean, tweak(Instrument::Gyro).rotate)
            .ball_if(game.gyro_ball(cam, tweak(Instrument::Gyro), eye_ship)),
        );
        // The design guide: the glass ruled, every shown dial's anchor and
        // reach, the gaze.
        {
            let gaze = if game.design {
                game.cursor_on_glass(cam)
                    .unwrap_or_else(|| look.gaze(ref_tan, cam.aspect))
            } else {
                look.gaze(ref_tan, cam.aspect)
            };
            let mut anchors: Vec<[f32; 2]> = Instrument::ALL
                .iter()
                .copied()
                .filter(|i| i.slotted())
                .filter_map(|i| layout.anchor(i))
                .collect();
            // The other glass elements DESIGN mode can take are marked
            // on the guide too: holo3PP, mini map, readout.
            if game.holo_active() {
                anchors.push(game.settings.holo_anchor);
            }
            if game.mini_map_shown() {
                anchors.push(layout.inset(game.mini_map_anchor()));
            }
            if layout.shown(Instrument::Readout) {
                anchors.push(game.settings.readout_anchor);
            }
            let anchors: Vec<[f32; 2]> = anchors
                .into_iter()
                .map(|a| on_glass(look.rotation(), Vec3::ZERO, cam, ref_tan, a))
                .take(8)
                .collect();
            self.passes.guide.update(
                &self.queue,
                &GuideUniforms::new(
                    aspect,
                    game.settings.guide || game.design,
                    layout.safe_edge,
                    on_glass(look.rotation(), Vec3::ZERO, cam, ref_tan, gaze),
                    DRAG_REACH,
                    look.engaged() || game.design,
                    &anchors,
                ),
            );
        }
        let horizon_on = if layout.shown(Instrument::Horizon) {
            1.0
        } else {
            0.0
        };
        self.passes.horizon.update(
            &self.queue,
            &HorizonUniforms::new(
                cam,
                game.up_world().as_vec3(),
                (game.state.ship.orient * DVec3::NEG_Z).as_vec3(),
                game.horizon_fade.level() * horizon_on,
                h,
                layout.shown(Instrument::Ladder),
            ),
        );
    }

    /// Whether the benchmark wants a capture now: one at the halfway mark,
    /// or — spinning — one every 1/n of the way round.
    fn bench_capture_due(&mut self, t: f64) -> bool {
        if self.cfg.bench_spin > 0 {
            let n = self.cfg.bench_spin;
            if self.bench_spin_taken >= n {
                return false;
            }
            let slot = (self.bench_spin_taken as f64 + 0.5) / n as f64;
            if t >= slot * self.cfg.bench_seconds {
                self.bench_spin_taken += 1;
                return true;
            }
            return false;
        }
        if t > self.cfg.bench_seconds * 0.5 && !self.bench_captured {
            self.bench_captured = true;
            return true;
        }
        false
    }

    /// While looking, the cursor is hidden and locked in place so the mouse
    /// measures head movement rather than walking off the window.
    fn set_look_cursor(&self, looking: bool) {
        use winit::window::CursorGrabMode;
        // A benchmark never takes the mouse: it is a measurement window on
        // someone's second screen, not a game.
        if self.cfg.bench {
            return;
        }
        self.window.set_cursor_visible(!looking);
        let mode = if looking {
            CursorGrabMode::Locked
        } else {
            CursorGrabMode::None
        };
        if self.window.set_cursor_grab(mode).is_err() && looking {
            let _ = self.window.set_cursor_grab(CursorGrabMode::Confined);
        }
    }

    /// Graphics settings, live. Scale and vsync are cheap; MSAA rebuilds
    /// every scene pipeline (and the heat field with them — the hull cools
    /// for a frame).
    fn apply_graphics(&mut self, settings: &Settings) {
        if (self.passes.cabin.fraction() - settings.cockpit_res).abs() > 1e-4 {
            self.passes.cabin.set_fraction(settings.cockpit_res);
            log::info!("cabin at {:.0}% of the scene", settings.cockpit_res * 100.0);
        }
        let target = self.scale_target(settings);
        if (self.scene.scale() - target).abs() > 1e-4 {
            self.scene.set_scale(target);
            log::info!("render scale {:.0}%", self.scene.scale() * 100.0);
        }
        if self.cfg.vsync != settings.vsync {
            self.cfg.vsync = settings.vsync;
            self.config.present_mode = if settings.vsync {
                wgpu::PresentMode::AutoVsync
            } else {
                wgpu::PresentMode::AutoNoVsync
            };
            self.surface.configure(&self.device, &self.config);
            self.perf.stats.skip_next_frame();
        }
        if self.cfg.msaa != settings.msaa {
            self.cfg.msaa = settings.msaa;
            self.scene = SceneTarget::new(settings.msaa, self.config.format, self.scene.scale());
            self.post = PostPass::new(&self.device, self.config.format, settings.msaa);
            self.passes = Passes::new(
                &self.device,
                self.config.format,
                settings.msaa,
                &self.baked,
                &self.nebula.view,
                settings.cockpit_res,
            );
            self.perf.stats.skip_next_frame();
            log::info!("MSAA {}x", settings.msaa);
        }
    }

    /// Close out the frame: record its duration, refresh the live readout in
    /// the title bar, and periodically summarise the window to the log.
    /// The world's scale this frame: the setting, or under AUTO SCALE
    /// the setting as a ceiling on the governor's factor.
    fn scale_target(&self, settings: &Settings) -> f32 {
        if self.vr_auto_scale_on(settings) {
            (settings.scale * self.auto_scale).clamp(AUTO_SCALE_MIN, 1.0)
        } else {
            settings.scale
        }
    }

    /// AUTO SCALE's effective on/off: the pilot's own setting, or forced
    /// on in VR (SPEC §5.3) — a native session's own compositor has a
    /// real deadline every frame, unlike a flat monitor's vsync, so the
    /// world's own resolution holding that pace is not optional the way
    /// it is at the desk. HUD/text/dials are unaffected either way —
    /// this governs `self.scene`'s own scale alone, drawn under them.
    fn vr_auto_scale_on(&self, settings: &Settings) -> bool {
        settings.auto_scale || self.cfg.vr
    }

    /// The FPS floor AUTO SCALE governs to: the pilot's own FPS FLOOR
    /// setting, or the runtime's own live display rate in VR (SPEC
    /// §5.3) — the compositor's actual pace always wins over a flat-
    /// play choice nobody tuned with a headset in mind. Falls back to
    /// the pilot's own setting when the runtime never reported a rate.
    fn vr_fps_floor(&self, settings_floor: f32) -> f32 {
        if self.cfg.vr {
            self.xr_display_refresh_hz().unwrap_or(settings_floor)
        } else {
            settings_floor
        }
    }

    /// AUTO SCALE: hold the FPS floor with the world's resolution — the
    /// one cost that scales with every pass at once — leaving the HUD,
    /// the dials and the text at native size. A miss drops the scale a
    /// step at a time; room above the floor, held for a while, brings
    /// it back. Vsync pins the rate at the floor, so "room" is the rate
    /// sitting on the floor with no slow frames under it.
    fn govern_scale(&mut self, settings: &Settings, fps_floor: f32) {
        if !self.vr_auto_scale_on(settings) || fps_floor <= 0.0 {
            return;
        }
        let now = Instant::now();
        let since = now.duration_since(self.auto_scale_at).as_secs_f32();
        if since < AUTO_SCALE_STEP_S {
            return;
        }
        let fps = self.perf.stats.smoothed_fps() as f32;
        let low = self.perf.stats.recent_low_1pct_fps() as f32;
        if fps <= 0.0 {
            return;
        }
        let before = self.auto_scale;
        if fps < fps_floor - 3.0 {
            // How far short, in pixels: the cost is the area.
            let ratio = (fps / fps_floor).clamp(0.5, 1.0).sqrt();
            self.auto_scale = (self.auto_scale * ratio.max(0.85)).max(AUTO_SCALE_MIN);
        } else if since >= AUTO_SCALE_RAISE_S && fps >= fps_floor - 1.0 && low >= fps_floor * 0.8 {
            self.auto_scale = (self.auto_scale * 1.08).min(1.0);
        } else {
            return;
        }
        if (self.auto_scale - before).abs() > 1e-4 {
            self.auto_scale_at = now;
            let target = self.scale_target(settings);
            if (self.scene.scale() - target).abs() > 1e-4 {
                self.scene.set_scale(target);
                log::info!(
                    "auto scale: {:.1} fps (1% low {:.1}) against a floor of {:.0}: render scale {:.0}%",
                    fps,
                    low,
                    fps_floor,
                    target * 100.0
                );
            }
        }
    }

    fn frame_timing(
        &mut self,
        cpu_seconds: f64,
        wait_seconds: f64,
        render_seconds: Option<f64>,
        fps_floor: f32,
        readout: &Readout,
    ) {
        let Readout {
            altitude_m,
            speed_mps,
            assist,
            show: show_readout,
            wind,
            collective,
            landing,
        } = readout;
        let (altitude_m, speed_mps, assist, show_readout) =
            (*altitude_m, *speed_mps, *assist, *show_readout);
        self.perf.cpu.record(cpu_seconds);
        self.perf.wait.record(wait_seconds);
        if let Some(r) = render_seconds {
            self.perf.render.record(r);
        }
        let now = Instant::now();
        let dt = now.duration_since(self.perf.last_frame).as_secs_f64();
        self.perf.last_frame = now;
        self.perf.stats.record(dt);
        // The floor: the cabin's moving detail answers for a slow frame
        // spent re-marching it.
        self.passes
            .cabin
            .govern(&self.device, (dt * 1000.0) as f32, fps_floor);

        // 4 Hz is fast enough to feel live and slow enough to stay readable.
        if now.duration_since(self.perf.last_title) >= Duration::from_millis(250) {
            self.perf.last_title = now;
            let fps = self.perf.stats.smoothed_fps();
            let low = self.perf.stats.recent_low_1pct_fps();
            self.text.clear();
            if !show_readout {
                return;
            }
            // CPU against total, side by side, because "the CPU feels busy" is
            // a hypothesis and this is the measurement that settles it.
            let frame_ms = if fps > 0.0 { 1000.0 / fps } else { 0.0 };
            let cpu_fps = self.perf.cpu.smoothed_fps();
            let cpu_ms = if cpu_fps > 0.0 { 1000.0 / cpu_fps } else { 0.0 };
            let (sw, sh) = self.scene.size();
            readout::render(
                &mut self.text,
                &readout::Readout {
                    fps,
                    low_fps: low,
                    cpu_ms,
                    rest_ms: (frame_ms - cpu_ms).max(0.0),
                    msaa: self.cfg.msaa,
                    scale_pct: self.scene.scale() * 100.0,
                    size: (sw, sh),
                    altitude_m,
                    speed_mps,
                    assist,
                    bench: self.cfg.bench,
                    wind: *wind,
                    collective: collective.map(|c| c as f32),
                    status: landing.clone(),
                },
            );
        }

        if now.duration_since(self.perf.last_log) >= PERF_LOG_EVERY {
            self.perf.last_log = now;
            if let Some(s) = self.perf.stats.take_summary() {
                // The VR bench stamp: key=value tokens the harness parses
                // off this exact line (never the "perf split" line
                // below). vr=<eyeW>x<eyeH>x2, scale, the runtime's own
                // display rate (never assumed — "unknown" if the runtime
                // never offered fb_display_refresh_rate), render_ms/
                // render_ms_1pct (CPU encode + real GPU time, forced
                // gpu_sync), xr_wait_ms (time inside wait_frame, the
                // runtime's own pacing) and the headroom against the
                // runtime's own rate.
                let vr_stamp = self.cfg.vr.then(|| {
                    let eye = self.xr_eye_size().unwrap_or((0, 0));
                    let hz = self.xr_display_refresh_hz();
                    let xr_wait_ms = self.xr_last_wait_ms();
                    let render = self.perf.render.take_summary();
                    let render_ms = render.map_or(0.0, |r| r.avg_ms);
                    let render_ms_1pct = render.map_or(0.0, |r| {
                        if r.low_1pct_fps > 0.0 {
                            1000.0 / r.low_1pct_fps
                        } else {
                            0.0
                        }
                    });
                    let headroom_ms = hz.map_or(0.0, |hz| 1000.0 / hz as f64 - render_ms);
                    format!(
                        " vr={}x{}x2 scale={:.2} hz={} render_ms={render_ms:.3} \
                         render_ms_1pct={render_ms_1pct:.3} xr_wait_ms={xr_wait_ms:.3} \
                         headroom_ms={headroom_ms:.3} synth={}",
                        eye.0,
                        eye.1,
                        self.cfg.vr_scale,
                        hz.map_or_else(|| "unknown".to_string(), |h| format!("{h:.1}")),
                        self.xr_is_synth() as u8,
                    )
                });
                log::info!(
                    "perf {}x{} {}xMSAA vsync={} gpu_sync={}: {:.1} fps avg \
                     | 1% low {:.1} fps | frame avg {:.2}ms worst {:.2}ms \
                     best {:.2}ms | {} frames{}",
                    self.config.width,
                    self.config.height,
                    self.cfg.msaa,
                    if self.cfg.vsync { "on" } else { "off" },
                    self.cfg.gpu_sync,
                    s.avg_fps,
                    s.low_1pct_fps,
                    s.avg_ms,
                    s.worst_ms,
                    s.best_ms,
                    s.frames,
                    vr_stamp.unwrap_or_default(),
                );
                if let (Some(c), Some(w)) =
                    (self.perf.cpu.take_summary(), self.perf.wait.take_summary())
                {
                    log::info!(
                        "perf split: cpu {:.3}ms avg (worst {:.3}) | swapchain wait \
                         {:.2}ms avg (worst {:.2}) — the wait is GPU and vsync, \
                         not CPU",
                        c.avg_ms,
                        c.worst_ms,
                        w.avg_ms,
                        w.worst_ms,
                    );
                }
            }
        }
    }
}

struct Game {
    /// The headset this frame, when one is driving the view.
    vr: Option<VrView>,
    /// Which of the headset's eyes is being drawn.
    vr_eye: usize,
    /// Set by VR RECENTRE; consumed by the XR session's own frame code,
    /// which owns the tracked space and clears it once re-seated.
    vr_recentre: bool,
    params: sim::WorldParams,
    state: sim::WorldState,
    input: InputState,
    /// Rotational assist. On by default: the ship is hard enough to fly with
    /// momentum intact, and the pilot can turn it off to feel that.
    assist: bool,
    /// Freeze the simulation, for repeatable measurement.
    frozen: bool,
    accumulator: f64,
    last_frame: Instant,
    started: Instant,
    /// Smoothed thrust effort in [0,1], driving the camera's response.
    /// Render-side only — it must never feed back into the sim.
    effort: f32,
    /// Relevance fade for the velocity hologram. Render-side only.
    gauge_fade: GaugeFade,
    /// Relevance fade for the altimeter: the ground's own instrument.
    alt_fade: AltitudeFade,
    /// Hologram inertia: instruments lag rotation for parallax.
    holo_sway: HoloSway,
    /// The camera on the pilot's head, and the bench's parked pose.
    shake: shake::Shake,
    bench_shake: Option<[f32; 3]>,
    /// FARFALL_BENCH_WIND: a forced wind (m/s, degrees off the nose it
    /// blows FROM) for the ribbons and the readout in captures. Visuals
    /// only — a bench's sim is frozen and never feels it.
    bench_wind: Option<(f64, f64)>,
    /// The sound-barrier flash, fired by the same edge as the sonic boom.
    mach_alert: MachAlert,
    /// Last wall-clock frame time, clamped, for presentation-side integrators.
    frame_dt: f32,
    /// The predicted path on the glass: T toggles it, and it fades rather
    /// than pops, like every other instrument.
    trajectory_vis: f32,
    /// The pilot's choices: graphics, keys, cockpit layout. The file is the
    /// state; the menu edits it in place.
    settings: Settings,
    menu: Menu,
    /// The map and its DRIVE panel (M).
    map_panel: Menu,
    horizon_fade: HorizonFade,
    /// The pilot's head: freelook, separate from the nose.
    look: Look,
    /// What the gaze is dragging, and where it sits relative to the point
    /// the pilot is looking at.
    drag: Option<(Dragged, [f32; 2])>,
    /// The map's orbiting camera.
    map_view: map::MapView,
    /// The SHIP bay: its card and its orbiting camera.
    bay_panel: Menu,
    bay_view: bay::BayView,
    /// The bay card's open dropdown, by hardpoint.
    bay_dropdown: Option<usize>,
    /// The pointer's click flash, 1 on a press, fading.
    press_flash: f32,
    /// Jumps made, for the drive's crack.
    jumps: u32,
    /// Bench: thrust and RCS demands forced for a capture.
    bench_thrust: Option<[f32; 4]>,
    /// DESIGN mode (K): lay the cockpit out — the guide on, the look
    /// locked, the dial under the gaze selected and its own settings on a
    /// card beside it.
    design: bool,
    /// The HUD file worn or saved last (its hud-<n> number), so SAVE HUD
    /// overwrites it rather than piling up copies.
    hud_loaded: Option<u32>,
    /// The CONTROLS card is up: any key puts it away.
    card_open: bool,
    /// LANDING mode (G): the hoops close up and judge the touchdown.
    landing: bool,
    /// The predicted touchdown, refreshed each frame in landing mode.
    touchdown: Option<landing::Touchdown>,
    /// How the last touchdown went, from the tick the ground was met.
    touchdown_record: Option<landing::Record>,
    /// DISEMBARK's answer, and when it was given (it shows for a moment).
    disembark_notice: Option<(&'static str, Instant)>,
    /// On foot (SPEC §6.5b): the EVA walker, when someone has stepped out.
    /// App state, like the mimics — never the sim's, never in the hash.
    eva: Option<eva::Walker>,
    /// The movement keys as held, for the walker's step.
    eva_keys: eva::Keys,
    /// Mouse: last cursor position and whether the left button is down,
    /// for dragging the map round; the window's size, to read it.
    cursor: Option<(f32, f32)>,
    window_size: (f32, f32),
    left_down: bool,
    /// The wormhole drive's sequence.
    warp: Warp,
    /// The hyper drive's level 0..1, eased — the field takes a moment to form.
    hyper: f32,
    /// The after-image of the last WARP STOP, while it lasts.
    ghost: Option<Ghost>,
    /// The asteroid belt's live rocks, when the ship is in Uranus' ring.
    belt: belt::Belt,
    /// The arms, and the trigger.
    arms: arms::Arms,
    fire_held: bool,
    /// The trigger on the stick.
    stick_fire: bool,
    /// What key each stick button pressed, so its release releases the
    /// same key even if SHIFT or the surface changed in between.
    stick_sent: [Option<KeyCode>; stick::MAX_BUTTONS as usize],
    /// The stick (a HOTAS through winmm / the Gamepad API), the wizard
    /// that maps it while it is up, and a frame count between the
    /// flight-log lines it writes.
    stick: stick::Reader,
    wizard: Option<stick::Wizard>,
    stick_log: u32,
    /// The throttle gestures: lever hard back brakes, a slam bursts.
    stick_gestures: stick::Gestures,
    /// The mimics — ships in the rocks — and what the guns bring in.
    mimics: mimic::Mimics,
    haul: mimic::Haul,
    /// The helicopters: the pads, who is flying what, the waiting fighter.
    helis: heli::Helis,
    /// The fighter's own sim parameters, restored on re-boarding.
    fighter_ship: sim::ShipParams,
    /// The miners working the ring.
    miners: miner::Miners,
    /// HOLD: the smart lock on the flight controls.
    hold: hold::Hold,
    /// Was the field up last frame (to feel its collapse).
    hyper_was: bool,
    /// The bench holding the field up.
    bench_hyper: bool,
    /// The drive's strain 0..1 from running the hyper field, and the point
    /// (drawn fresh each time) at which it slips: the wormhole drive fires
    /// of its own accord and drops the ship somewhere else entirely.
    hyper_strain: f32,
    slip_at: f32,
    pending_slip: bool,
    /// Felt acceleration over the last sim step, g, and the meter's fade.
    felt_g: f32,
    /// The same, as a vector in the ship's frame: right, up, forward (g).
    felt_g_body: [f32; 3],
    g_fade: GForceFade,
    /// Metres of path flown, so the path's marks can stay fixed to the
    /// world. Presentation only: a wrapped f32 is fine for a phase.
    odometer_m: f64,
    /// Hoops that have passed the ship while the path was showing: the
    /// audio womps on every increment.
    hoops_passed: u32,
    /// The shell's recent impacts (newest first), for the shield's ripples.
    impacts: Vec<Impact>,
    /// Micrometeorite strikes so far, the latest one's size, and the dice.
    strikes: u32,
    strike_size: f32,
    strike_rng: u32,
    /// Which world we are looking at. Cycled with the number keys until there
    /// is a real settings panel.
    appearance: PlanetAppearance,
    appearance_index: usize,
    /// Sim time of the next autosave (see [`AUTOSAVE_INTERVAL_S`]).
    next_save_s: f64,
}

impl Game {
    fn new() -> Self {
        let mut params = sim::presets::earth_compact();
        let altitude = std::env::var("FARFALL_BENCH_ALT")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(SPAWN_ALTITUDE_M);
        let mut state = sim::presets::circular_orbit(&params, altitude);
        state.ship.orient = spawn_attitude();
        if std::env::var("FARFALL_BENCH_ALT").is_ok() && altitude < LOW_BENCH_CEILING_M {
            low_bench_flight(&mut state);
        }
        if let Some(pos) = std::env::var("FARFALL_BENCH_POS")
            .ok()
            .and_then(|v| parse_vec3(&v))
        {
            state.ship.pos_m = pos;
            state.ship.vel_mps = std::env::var("FARFALL_BENCH_VEL")
                .ok()
                .and_then(|v| parse_vec3(&v))
                .unwrap_or(DVec3::ZERO);
            let aim = std::env::var("FARFALL_BENCH_LOOK")
                .ok()
                .and_then(|v| parse_vec3(&v))
                .unwrap_or(-pos);
            state.ship.orient = look_at(aim, DVec3::Y);
            // FARFALL_BENCH_ROLL=rad: rolled about the look axis — for
            // checking that what belongs to the world (the Sun's own
            // weather) holds still while the ship turns over.
            if let Some(roll) = std::env::var("FARFALL_BENCH_ROLL")
                .ok()
                .and_then(|v| v.trim().parse::<f64>().ok())
            {
                state.ship.orient *= DQuat::from_rotation_z(roll);
            }
        }
        // FARFALL_SPAWN=belt|uranus|moon|sun: start where the wormhole
        // drive would land you — the belt (Uranus' ring) is where the
        // mimics live. Nose prograde, the body below.
        if let Some(dest) = std::env::var("FARFALL_SPAWN").ok().and_then(|v| {
            let k = v.trim().to_ascii_lowercase();
            warp::Destination::from_key(if k == "belt" { "uranus" } else { &k })
        }) {
            let plan = warp::Plan {
                dest,
                ..warp::Plan::default()
            };
            let t = state.time_s;
            let (pos, vel) = plan.arrival(&params, &state.ship, t);
            state.ship.pos_m = pos;
            state.ship.vel_mps = vel;
            let up = (pos - dest.centre(&params, t)).normalize();
            state.ship.orient = look_at(vel - dest.velocity(&params, t), up);
            log::info!("spawn: {} at {:.0} m/s", dest.name(), vel.length());
        }
        // FARFALL_BENCH_LANDED=1: parked on the ground, LANDED, for the
        // capture of the settled state and its readout.
        // FARFALL_BENCH_EVA=1 parks the same way — the walk-out itself is
        // staged once the game exists, beside the other bench triggers.
        let mut helis = heli::Helis::default();
        let bench_landed = std::env::var("FARFALL_BENCH_LANDED").is_ok();
        if bench_landed || std::env::var("FARFALL_BENCH_EVA").is_ok() {
            state.ship = landing::parked(&params, 0);
        }
        // FARFALL_BENCH_HELI=1: parked beside the coast pad's helicopter,
        // LANDED, the pad and the boarding offer in frame.
        let bench_heli = std::env::var("FARFALL_BENCH_HELI").unwrap_or_default();
        if !bench_heli.is_empty() {
            let heli_at = heli::parked(&params, 0);
            let mut pos = heli_at.pos_m + heli_at.orient * DVec3::new(26.0, 0.0, -4.0);
            let up = pos.normalize();
            pos = up * (params.planet.radius_m + sim::GEAR_HEIGHT_M);
            // Face the helicopter from beside its pad.
            let aim = (heli_at.pos_m - pos).normalize();
            let nose = (aim - up * aim.dot(up)).normalize();
            let right = nose.cross(up).normalize();
            state.ship = sim::ShipState {
                pos_m: pos,
                vel_mps: DVec3::ZERO,
                orient: DQuat::from_mat3(&glam::DMat3::from_cols(right, up, -nose)),
                ang_vel_radps: DVec3::ZERO,
                ground: sim::Ground::Landed { body: 0, up },
            };
            if bench_heli == "fly" {
                // =fly: boarded and hovering over the pad, for the
                // capture of the helicopter as the pilot's own ship.
                let mut h = helis.board(&params, 0, state.ship);
                h.pos_m += heli::pad_up(0) * 16.0;
                h.vel_mps = DVec3::ZERO;
                h.ground = sim::Ground::Flight;
                state.ship = h;
                params.ship = heli::heli_params();
            }
        }
        let now = Instant::now();
        let spawn_time_s = state.time_s;
        Self {
            vr: None,
            vr_eye: 0,
            vr_recentre: false,
            params,
            state,
            input: InputState::default(),
            assist: true,
            frozen: Config::from_env(&Settings::default()).bench,
            accumulator: 0.0,
            last_frame: now,
            started: now,
            effort: 0.0,
            gauge_fade: GaugeFade::new(),
            alt_fade: AltitudeFade::new(),
            holo_sway: HoloSway::new(),
            shake: shake::Shake::new(1.0),
            bench_shake: None,
            bench_wind: std::env::var("FARFALL_BENCH_WIND").ok().and_then(|v| {
                let xs: Vec<f64> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                match xs.as_slice() {
                    [mps] => Some((*mps, 90.0)),
                    [mps, deg] => Some((*mps, *deg)),
                    _ => None,
                }
            }),
            mach_alert: MachAlert::new(),
            frame_dt: 0.0,
            trajectory_vis: 1.0,
            settings: Settings::default(),
            menu: Menu::new(),
            map_panel: Menu::map_panel(),
            horizon_fade: HorizonFade::new(),
            look: Look::new(),
            drag: None,
            map_view: map::MapView::default(),
            bay_panel: Menu::ship_panel(),
            bay_view: bay::BayView::default(),
            bay_dropdown: None,
            press_flash: 0.0,
            jumps: 0,
            bench_thrust: std::env::var("FARFALL_BENCH_THRUST").ok().and_then(|v| {
                let xs: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                (xs.len() == 4).then(|| [xs[0], xs[1], xs[2], xs[3]])
            }),
            design: false,
            hud_loaded: None,
            card_open: false,
            landing: false,
            touchdown: None,
            touchdown_record: bench_landed.then(landing::Record::sample),
            disembark_notice: None,
            eva: None,
            eva_keys: eva::Keys::default(),
            cursor: None,
            window_size: (1.0, 1.0),
            left_down: false,
            warp: Warp::new(),
            hyper: 0.0,
            ghost: None,
            belt: belt::Belt::default(),
            arms: arms::Arms::default(),
            fire_held: false,
            stick_fire: false,
            stick_sent: [None; stick::MAX_BUTTONS as usize],
            stick: stick::Reader::default(),
            wizard: None,
            stick_log: 0,
            stick_gestures: stick::Gestures::default(),
            mimics: mimic::Mimics::default(),
            haul: mimic::Haul::default(),
            helis,
            fighter_ship: params.ship,
            miners: miner::Miners::default(),
            hold: hold::Hold::default(),
            hyper_was: false,
            bench_hyper: false,
            hyper_strain: 0.0,
            slip_at: 0.85,
            pending_slip: false,
            felt_g: 0.0,
            felt_g_body: [0.0; 3],
            g_fade: GForceFade::new(),
            odometer_m: 0.0,
            hoops_passed: 0,
            impacts: Vec::new(),
            strikes: 0,
            strike_size: 0.0,
            strike_rng: 0x9E37_79B9,
            appearance: PlanetAppearance::EARTHLIKE,
            appearance_index: 0,
            next_save_s: spawn_time_s + AUTOSAVE_INTERVAL_S,
        }
    }

    /// The settings panel's text: the wizard while it is up, else the menu.
    fn render_menu(&self, text: &mut TextBitmap) {
        match &self.wizard {
            Some(w) => w.render(text, &self.settings.stick, self.stick.device.as_ref()),
            None => self.menu.render(text, &self.settings),
        }
    }

    /// Take on a settings block: the key map goes to the input, the rest is
    /// read where it is used.
    fn apply_settings(&mut self, settings: Settings) {
        self.settings = settings;
        self.input.set_bindings(settings.bindings);
        self.look.sensitivity = settings.look_sensitivity;
        // The guns fire from whatever the bay mounted.
        self.arms.mounts = settings.mounts;
        // These used to be pushed only when the bay's dropdown changed a
        // mount (`bay_click`), so a non-default value loaded straight
        // from the settings file had no effect until the bay was opened.
        self.mimics.chance = settings.mimics_chance;
        self.mimics.hostility = settings.mimics_hostility;
        self.haul.yield_ = settings.arms_ore;
    }

    /// Every field the world file keeps, as of right now.
    fn snapshot(&self) -> save::Save {
        save::Save {
            time_s: self.state.time_s,
            ship_pos: self.state.ship.pos_m,
            ship_vel: self.state.ship.vel_mps,
            ship_orient: self.state.ship.orient,
            ship_spin: self.state.ship.ang_vel_radps,
            assist: self.assist,
            landing: self.landing,
            appearance_index: self.appearance_index,
            hyper_strain: self.hyper_strain,
            slip_at: self.slip_at,
            jumps: self.jumps,
            arms_selected: self.arms.selected,
            arms_ammo: self.arms.ammo,
            arms_jammed: self.arms.jammed,
            arms_heat: self.arms.heat,
            arms_charge: self.arms.charge,
            haul_tonnes: self.haul.tonnes,
            hull: self.mimics.hull,
            strikes: self.strikes,
            strike_rng: self.strike_rng,
            odometer_m: self.odometer_m,
            hoops_passed: self.hoops_passed,
            belt_dead: self.belt.dead.clone(),
            belt_wounds: self.belt.wounds.clone(),
            mimics_revealed: self.mimics.revealed.clone(),
            mimics: self
                .mimics
                .ships
                .iter()
                .copied()
                .map(mimic::Mimic::to_save)
                .collect(),
        }
    }

    /// Take on a save: the sim state, and every app-side field a save
    /// keeps. Everything NOT in that list — the accumulator, the warp
    /// sequence, a ghost, an open hold lock, every fade/shake integrator,
    /// the settings themselves — is reset instead, exactly as a fresh
    /// [`Game::new`] would leave it, because none of it is a fact about
    /// the world: it is mid-frame or mid-gesture presentation state that
    /// means nothing once the session that produced it is gone.
    fn restore(&mut self, save: &save::Save) {
        self.state = sim::WorldState {
            time_s: save.time_s,
            ship: sim::ShipState {
                pos_m: save.ship_pos,
                vel_mps: save.ship_vel,
                orient: save.ship_orient,
                ang_vel_radps: save.ship_spin,
                // The file predates ground state and the hash ignores it:
                // a ship saved parked wakes in Flight at rest on its pad
                // and settles clean within the first ticks.
                ground: sim::Ground::Flight,
            },
        };
        self.assist = save.assist;
        self.landing = save.landing;
        self.appearance_index = save
            .appearance_index
            .min(PlanetAppearance::PRESETS.len() - 1);
        self.appearance = PlanetAppearance::PRESETS[self.appearance_index];
        self.hyper_strain = save.hyper_strain;
        self.slip_at = save.slip_at;
        self.jumps = save.jumps;
        self.arms.selected = save.arms_selected;
        self.arms.ammo = save.arms_ammo;
        self.arms.jammed = save.arms_jammed;
        self.arms.heat = save.arms_heat;
        self.arms.charge = save.arms_charge;
        self.haul.tonnes = save.haul_tonnes;
        self.haul.last = None;
        self.mimics.hull = save.hull;
        self.strikes = save.strikes;
        self.strike_size = 0.0;
        self.strike_rng = save.strike_rng;
        self.odometer_m = save.odometer_m;
        self.hoops_passed = save.hoops_passed;
        self.belt.rocks.clear();
        self.belt.hits.clear();
        self.belt.dead = save.belt_dead.clone();
        self.belt.wounds = save.belt_wounds.clone();
        self.mimics.revealed = save.mimics_revealed.clone();
        self.mimics.ships = save
            .mimics
            .iter()
            .map(|m| mimic::Mimic::from_save(m, save.time_s))
            .collect();
        self.mimics.slugs.clear();
        self.mimics.own_hits.clear();

        // Reset: mid-frame and mid-gesture presentation state, none of it
        // a fact about the world.
        let now = Instant::now();
        self.started = now;
        self.last_frame = now;
        self.accumulator = 0.0;
        self.warp = Warp::new();
        self.hyper = 0.0;
        self.hyper_was = false;
        self.bench_hyper = false;
        self.pending_slip = false;
        self.ghost = None;
        self.impacts.clear();
        self.touchdown = None;
        self.hold.release();
        self.frozen = false;
        self.gauge_fade = GaugeFade::new();
        self.alt_fade = AltitudeFade::new();
        self.holo_sway = HoloSway::new();
        self.shake = shake::Shake::new(1.0);
        self.mach_alert = MachAlert::new();
        self.horizon_fade = HorizonFade::new();
        self.g_fade = GForceFade::new();
        self.felt_g = 0.0;
        self.felt_g_body = [0.0; 3];
        self.next_save_s = save.time_s + AUTOSAVE_INTERVAL_S;

        self.mimics.line = Some((
            format!("RESUMED  T+{}", format_hms(save.time_s)),
            save.time_s + 4.0,
        ));
        log::info!(
            "world: resumed t={:.1}s hash={:016x}",
            self.state.time_s,
            save.seal()
        );
    }

    /// NEW GAME and the world's first spawn share this: everything about
    /// the sim and the gameplay state goes back to a fresh start.
    /// `settings`, `look`, the menus (and where they are open to) and the
    /// mouse/window bookkeeping are kept — only the world itself is new.
    fn reset_world(&mut self) {
        let settings = self.settings;
        let look = self.look;
        let menu = self.menu;
        let map_panel = self.map_panel;
        let bay_panel = self.bay_panel;
        let window_size = self.window_size;
        let cursor = self.cursor;
        let left_down = self.left_down;
        let vr = self.vr;
        let vr_eye = self.vr_eye;

        *self = Game::new();
        self.apply_settings(settings);
        self.look = look;
        self.menu = menu;
        self.map_panel = map_panel;
        self.bay_panel = bay_panel;
        self.window_size = window_size;
        self.cursor = cursor;
        self.left_down = left_down;
        self.vr = vr;
        self.vr_eye = vr_eye;
    }

    /// Write the world file, if RESUME allows it right now. Shared by the
    /// autosave (`tick`), every real exit (`log_exit`), and the web
    /// build's `pagehide`/hidden-`visibilitychange` listeners — a tab can
    /// vanish with no exit event at all.
    fn maybe_store_world(&self) {
        if resume_allowed(
            self.settings.resume,
            self.frozen,
            env_resume().as_deref(),
            bench_spawn_env_present(),
        ) {
            self.snapshot().store();
        }
    }

    /// The Sun and the Moon as the camera sees them: where the sim has them
    /// at sim time, subtracted in f64 (P3).
    fn bodies_uniforms(&self, pose: &ViewPose, height_px: f32) -> BodiesUniforms {
        let cam = &pose.cam;
        let eye = self.eye_m(pose);
        let [_, moon, sun, uranus] = self.params.bodies(self.state.time_s);
        let tags = if self.settings.layout.shown(Instrument::BodyTags) {
            1.0
        } else {
            0.0
        };
        BodiesUniforms::new(
            cam,
            ((moon.centre - eye).as_vec3(), moon.radius_m as f32),
            ((sun.centre - eye).as_vec3(), sun.radius_m as f32),
            ((uranus.centre - eye).as_vec3(), uranus.radius_m as f32),
            tags,
            height_px,
        )
        .with_planet_and_flare(
            (
                (DVec3::ZERO - eye).as_vec3(),
                self.params.planet.radius_m as f32,
            ),
            self.settings.flare,
        )
        .with_ring_phase(
            (belt::ring_rate_radps(&uranus) * self.state.time_s).rem_euclid(std::f64::consts::TAU)
                as f32,
        )
    }

    /// The system map, from the plan and the pilot's view of it.
    fn map_uniforms(&self, aspect: f32, time_s: f32) -> map::MapUniforms {
        let [_, moon, sun, uranus] = self.params.bodies(self.state.time_s);
        let dest = self.settings.plan.dest;
        let world = map::MapWorld {
            ship: self.state.ship.pos_m,
            ship_orient: self.state.ship.orient,
            moon: moon.centre,
            sun: sun.centre,
            uranus: uranus.centre,
            dest_centre: dest.centre(&self.params, self.state.time_s),
            dest_arrival_m: dest.radius_m(&self.params) + self.settings.plan.safe_m(&self.params),
        };
        // The full map on M; else the mini map, a gauge on the glass at
        // its anchor, re-projected like a dial, with no dim round it.
        let mini = !self.map_open() && self.mini_map_shown();
        let look = map::MapLook {
            view: self.map_view,
            rings: self.settings.map_rings,
            grid: self.settings.map_grid,
            craft: self.settings.craft.kind(),
            visibility: if self.map_open() || mini { 1.0 } else { 0.0 },
            aspect,
            time_s,
            centre: if mini {
                let cam = self.camera(aspect);
                let a = self.settings.layout.inset(self.mini_map_anchor());
                let c = on_glass(
                    self.glass_head(),
                    self.glass_eye_pos(),
                    &cam,
                    self.ref_tan(),
                    a,
                );
                // An instrument, not scenery: a far-turned head must
                // not swing the pane off the screen's edge.
                map::mini_centre_on_screen(aspect, c, self.mini_map_half_h())
            } else {
                self.settings.map_anchor
            },
            half_h: if mini {
                self.mini_map_half_h()
            } else {
                map::PANE_HALF_H
            },
            dim: !mini,
        };
        map::MapUniforms::new(&world, &look)
    }

    /// The path's world-fixed marks, from the odometer and the settings.
    /// In LANDING mode with a touchdown ahead the grid is phased onto the
    /// touchdown instead, so the hoops converge on the pad drawn there
    /// (`eye_m` is the camera, for the pad's camera-relative position).
    fn marks(&self, eye_m: DVec3) -> farfall_render::trajectory::Marks {
        let spacing = self.mark_spacing_m();
        let approach = self
            .touchdown
            .filter(|_| self.landing && self.state.ship.ground == sim::Ground::Flight);
        farfall_render::trajectory::Marks {
            odometer_m: match approach {
                Some(t) => landing::hoop_phase(t.path_m, spacing),
                None => (self.odometer_m % 1.0e6) as f32,
            },
            hoops: self.settings.layout.shown(Instrument::Hoops),
            // Landing hoops are big: a gate to fly through, not a bead.
            hoop_scale: self.settings.hoop_size * if self.landing { 2.5 } else { 1.0 },
            spacing_m: spacing,
            landing: if self.landing {
                Some(self.touchdown.map_or(0.0, |t| t.danger()))
            } else {
                None
            },
            pad: approach.filter(|_| self.settings.landing_pad).map(|t| {
                farfall_render::trajectory::Pad {
                    rel: (t.pos - eye_m).as_vec3(),
                    up: t.up.as_vec3(),
                }
            }),
        }
    }

    /// The marks' spacing: the landing hoops' setting in LANDING mode, a
    /// kilometre otherwise.
    fn mark_spacing_m(&self) -> f32 {
        if self.landing {
            self.settings.landing_spacing_m
        } else {
            MARK_SPACING_M
        }
    }

    /// The map (and its DRIVE panel) is up.
    fn map_open(&self) -> bool {
        self.map_panel.open
    }

    /// The mini map is a stock gauge: shown on the glass while no pane or
    /// card covers it and the view is the cockpit's. DESIGN mode keeps it
    /// up — it is one of the things being laid out.
    fn mini_map_shown(&self) -> bool {
        let covered =
            self.menu.open || self.map_panel.open || self.bay_panel.open || self.card_open;
        self.settings.layout.shown(Instrument::Map) && !covered && !self.exterior_view()
    }

    /// Where the mini map's centre sits before the safe-edge inset: the
    /// stock corner, or wherever the pilot dragged it (kept on ui.map).
    fn mini_map_anchor(&self) -> [f32; 2] {
        self.settings
            .layout
            .free(Instrument::Map)
            .unwrap_or(map::MINI_ANCHOR)
    }

    /// The mini map's half height: the stock pane times its own SIZE
    /// (ui.map.size) — a gauge, sized like any dial.
    fn mini_map_half_h(&self) -> f32 {
        map::MINI_HALF_H
            * self.settings.dials[Instrument::Map as usize]
                .size
                .clamp(settings::DIAL_SIZE_MIN, settings::DIAL_SIZE_MAX)
    }

    /// The SHIP bay (and its card) is up.
    fn bay_open(&self) -> bool {
        self.bay_panel.open
    }

    /// A pane with a picture is up: the map or the bay.
    fn pane_open(&self) -> bool {
        self.map_open() || self.bay_open()
    }

    /// Any panel is up: the world waits and the keys go to it.
    fn panel_open(&self) -> bool {
        self.menu.open
            || self.map_panel.open
            || self.bay_panel.open
            || self.design
            || self.card_open
    }

    /// M: the map up or down. One panel at a time.
    fn toggle_map(&mut self) {
        self.map_panel.toggle();
        if self.map_panel.open {
            self.menu.open = false;
            self.bay_panel.open = false;
        }
        self.input.release_all();
    }

    /// B: the SHIP bay up or down. One panel at a time.
    fn toggle_bay(&mut self) {
        self.bay_panel.toggle();
        if self.bay_panel.open {
            self.menu.open = false;
            self.map_panel.open = false;
        }
        self.input.release_all();
    }

    /// F1 (and the first run): the CONTROLS card up, over everything.
    fn open_card(&mut self) {
        self.card_open = true;
        self.menu.open = false;
        self.map_panel.open = false;
        self.bay_panel.open = false;
        self.input.release_all();
    }

    /// Any key: the card away. The settings are written so the file
    /// exists and the card does not show itself again unasked.
    fn close_card(&mut self) {
        self.card_open = false;
        self.settings.save();
    }

    /// HOLO RANGE by a notch: the hologram shows more space (positive)
    /// or less, and the setting is kept.
    fn zoom_holo(&mut self, notches: f32) {
        let before = self.settings.holo_range;
        self.settings.holo_range = (before + notches * settings::HOLO_RANGE_STEP)
            .clamp(settings::HOLO_RANGE_MIN, settings::HOLO_RANGE_MAX);
        if (self.settings.holo_range - before).abs() > 1e-6 {
            self.settings.save();
            log::info!("holo range {:.1}x", self.settings.holo_range);
        }
    }

    /// Esc: the settings menu up or down. One panel at a time.
    fn toggle_menu(&mut self) {
        self.menu.toggle();
        if self.menu.open {
            self.map_panel.open = false;
            self.bay_panel.open = false;
        }
        self.input.release_all();
        self.eva_keys = eva::Keys::default();
    }

    /// The bay takes the whole screen: the hologram's centre sits left of
    /// the middle (the card is on the right) and the "pane" covers it all.
    /// The anchor is where the card hangs; HOLO SIZE scales the picture.
    fn bay_pane(&self) -> [f32; 3] {
        [
            -0.22,
            -0.05,
            1.3 * self.settings.bay_size / settings::BAY_SIZE_DEFAULT,
        ]
    }

    /// Where the bay's card starts: its top-right at the anchor. A flat
    /// block's width on the screen is `text_w` over the aspect.
    fn bay_text_anchor(&self, aspect: f32, text_w: f32) -> [f32; 2] {
        let a = self.settings.bay_anchor;
        let w = text_w / aspect;
        [(a[0] - w).clamp(-0.98, 0.98 - w), a[1].clamp(-0.5, 0.95)]
    }

    /// The card's rows, top to bottom: the header, a row per hardpoint
    /// (its dropdown's options under the open one), the footer.
    fn bay_rows(&self) -> Vec<BayRow> {
        let mut rows = vec![BayRow::Header, BayRow::Craft];
        for i in 0..bay::Hardpoint::ALL.len() {
            rows.push(BayRow::Slot(i));
            if self.bay_dropdown == Some(i) {
                for m in bay::Mount::ALL {
                    rows.push(BayRow::Option(i, m));
                }
            }
        }
        rows.push(BayRow::Footer);
        rows
    }

    /// The card: a diagram's labels, not a menu — one line per slot with
    /// its mount, the open slot's options under it.
    fn render_bay_card(&self, text: &mut TextBitmap) {
        text.clear();
        let selected = self.bay_panel.bay_selected();
        for (row, r) in self.bay_rows().iter().enumerate() {
            let line = match *r {
                BayRow::Header => "SHIP BAY                    FIT".to_string(),
                BayRow::Craft => {
                    let cur = if self.bay_panel.bay_on_craft() {
                        ">"
                    } else {
                        " "
                    };
                    format!(
                        "{cur}{:<9}{:>18} \u{2194}",
                        "CRAFT",
                        self.settings.craft.name()
                    )
                }
                BayRow::Slot(i) => {
                    let h = bay::Hardpoint::ALL[i];
                    let open = self.bay_dropdown == Some(i);
                    let mark = if open { "\u{2191}" } else { "\u{2193}" };
                    let cur = if selected == Some(i) { ">" } else { " " };
                    format!(
                        "{cur}{:<9}{:>18} {mark}",
                        h.name(),
                        self.settings.mounts[i].name()
                    )
                }
                BayRow::Option(i, m) => {
                    let on = self.settings.mounts[i] == m;
                    format!("   {} {}", if on { "*" } else { " " }, m.name())
                }
                BayRow::Footer => "CLICK A SLOT  DRAG TURN  B CLOSE".to_string(),
            };
            text.draw_line(0, row, &line);
        }
    }

    /// The card's chosen slot row, for the highlight band.
    fn bay_highlight_row(&self) -> Option<(f32, f32)> {
        let sel = self.bay_panel.bay_selected()?;
        self.bay_rows()
            .iter()
            .position(|r| *r == BayRow::Slot(sel))
            .map(|row| ((row * LINE) as f32, LINE as f32))
    }

    /// A click in the bay, at `at` (NDC): a card row (a slot opens or
    /// closes its dropdown; an option fits it), or a pip on the hologram
    /// (its slot is chosen). True when the click was taken.
    fn bay_click(&mut self, at: [f32; 2], aspect: f32, text_w: f32, px: f32) -> bool {
        let anchor = self.bay_text_anchor(aspect, text_w);
        let rows = self.bay_rows();
        let w = text_w / aspect;
        let on_card = at[0] >= anchor[0] - 0.01 && at[0] <= anchor[0] + w + 0.01;
        if on_card && at[1] <= anchor[1] {
            let row = ((anchor[1] - at[1]) / (LINE as f32 * px)).floor() as usize;
            match rows.get(row).copied() {
                Some(BayRow::Craft) => {
                    self.bay_panel.set_cursor(0);
                    self.settings.craft = self.settings.craft.next(true);
                    self.settings.save();
                    self.bay_dropdown = None;
                    return true;
                }
                Some(BayRow::Slot(i)) => {
                    self.bay_panel.select_mount(i);
                    self.bay_dropdown = if self.bay_dropdown == Some(i) {
                        None
                    } else {
                        Some(i)
                    };
                    return true;
                }
                Some(BayRow::Option(i, m)) => {
                    self.settings.mounts[i] = m;
                    self.arms.mounts = self.settings.mounts;
                    self.mimics.chance = self.settings.mimics_chance;
                    self.mimics.hostility = self.settings.mimics_hostility;
                    self.haul.yield_ = self.settings.arms_ore;
                    self.settings.save();
                    self.bay_dropdown = None;
                    return true;
                }
                Some(BayRow::Header) | Some(BayRow::Footer) => return true,
                None => {}
            }
        }
        // A pip: the nearest hardpoint on the screen, if close.
        let [cx, cy, hw] = self.bay_pane();
        let v = &self.bay_view;
        let cam = HologramCamera::orbit(v.yaw, v.pitch, v.dist, BAY_TAN_HALF_FOV);
        let mut best: Option<(usize, f32)> = None;
        for (i, h) in bay::Hardpoint::ALL.iter().enumerate() {
            if let Some(p) = cam.project(h.pos().as_vec3(), aspect, [cx, cy], hw) {
                let d = ((p[0] - at[0]) * aspect).hypot(p[1] - at[1]);
                if d < 0.05 && best.is_none_or(|b| d < b.1) {
                    best = Some((i, d));
                }
            }
        }
        if let Some((i, _)) = best {
            self.bay_panel.select_mount(i);
            self.bay_dropdown = Some(i);
            return true;
        }
        false
    }

    /// The bay's hologram this frame. `text`: the card's anchor (NDC) and
    /// font pixel, for the callouts' label points.
    fn hologram_uniforms(
        &self,
        aspect: f32,
        time_s: f32,
        height_px: f32,
        text: ([f32; 2], f32),
    ) -> HologramUniforms {
        let [cx, cy, hw] = self.bay_pane();
        let rows = self.bay_rows();
        let mut callouts = [None; farfall_render::hologram::HARDPOINTS];
        for (i, c) in callouts.iter_mut().enumerate() {
            if let Some(row) = rows.iter().position(|r| *r == BayRow::Slot(i)) {
                *c = Some(Callout {
                    at: [
                        text.0[0] - 0.012,
                        text.0[1] - (row as f32 + 0.5) * 6.0 * text.1,
                    ],
                    open: self.bay_dropdown == Some(i),
                });
            }
        }
        let v = &self.bay_view;
        // One source: the same table the cockpit's glass and the chase
        // view read (bay::fit_views).
        let mounts = bay::fit_views(&self.settings.mounts);
        HologramUniforms::new(&HologramScene {
            camera: HologramCamera::orbit(v.yaw, v.pitch, v.dist, BAY_TAN_HALF_FOV),
            pane_centre: [cx, cy],
            pane_half_w: hw,
            aspect,
            visibility: if self.bay_open() { 1.0 } else { 0.0 },
            hue: self.settings.bay_hue,
            saturation: self.settings.bay_saturation,
            scanlines: self.settings.bay_scanlines,
            selected: self.bay_panel.bay_selected(),
            mounts,
            time_s,
            height_px,
            fullscreen: true,
            callouts,
            craft: self.settings.craft.kind(),
        })
    }

    /// The pointer this frame: at the cursor whenever a panel is up.
    fn pointer_uniforms(&self, aspect: f32, time_s: f32) -> PointerUniforms {
        let tip = if self.panel_open() {
            self.cursor_screen()
        } else {
            None
        };
        PointerUniforms::new(
            tip,
            self.settings.pointer_size,
            aspect,
            self.press_flash,
            time_s,
        )
    }

    /// The map pane's geometry this frame.
    fn map_pane(&self, aspect: f32) -> [f32; 3] {
        map::pane_rect(aspect, self.settings.map_anchor)
    }

    /// Where the DRIVE panel's text block starts: hung off the map pane's
    /// top-left, `text_w` (NDC) to its left.
    fn drive_text_anchor(&self, aspect: f32, text_w: f32) -> [f32; 2] {
        let [cx, cy, hw] = self.map_pane(aspect);
        [cx - hw - text_w / aspect - 0.03, cy + hw * aspect]
    }

    /// The current text block's width in characters: the settings card and
    /// the CONTROLS card are wide; a panel beside a picture, and the
    /// readout, are narrow.
    fn text_cols(&self) -> usize {
        if self.card_open || self.menu.open {
            MENU_COLS
        } else {
            PANEL_COLS
        }
    }

    /// The current text block's width in canopy units.
    fn text_w(&self, px: f32) -> f32 {
        panel::block_ndc(self.text_cols(), px)
    }

    /// A card kept by its centre: its top-left on the screen, for a card
    /// `extent` font px big at `px` a font pixel.
    fn centred_card(centre: [f32; 2], extent: (usize, usize), px: f32, aspect: f32) -> [f32; 2] {
        [
            centre[0] - extent.0 as f32 * px / aspect * 0.5,
            centre[1] + extent.1 as f32 * px * 0.5,
        ]
    }

    /// Where the text block's top-left sits this frame: the CONTROLS card
    /// centred, the DRIVE panel beside the map, the settings card about
    /// its anchor, else the readout.
    fn text_anchor(&self, aspect: f32, px: f32) -> [f32; 2] {
        let text_w = self.text_w(px);
        if self.card_open {
            return Self::centred_card([0.0, 0.05], card::extent(), px, aspect);
        }
        if self.design {
            // Beside the selected element, or where the readout lives.
            let anchor = self.design_target(aspect).and_then(|el| match el {
                DesignEl::Dial(i) => self.settings.layout.anchor(i),
                DesignEl::Holo => Some(self.settings.holo_anchor),
                DesignEl::MiniMap => Some(self.settings.layout.inset(self.mini_map_anchor())),
                DesignEl::Readout => Some(self.settings.readout_anchor),
            });
            return match anchor {
                Some(a) => [a[0] + 0.2, a[1] + 0.12],
                None => self.settings.readout_anchor,
            };
        }
        if self.map_open() {
            self.drive_text_anchor(aspect, text_w)
        } else if self.bay_open() {
            self.bay_text_anchor(aspect, text_w)
        } else if self.menu.open {
            Self::centred_card(self.settings.menu_anchor, self.menu.extent(), px, aspect)
        } else {
            self.settings.readout_anchor
        }
    }

    /// The cabin: the head's turn, the Sun in the ship's frame, and a
    /// socket under every dial the pilot has on the glass.
    fn cabin_uniforms(
        &self,
        cam: &CameraFrame,
    ) -> (
        farfall_render::cabin::CabinUniforms,
        farfall_render::cabin::BlitUniforms,
    ) {
        use farfall_render::cabin::{anchor_direction, CabinLook, CabinUniforms};
        let ship_inv = self.state.ship.orient.as_quat().inverse();
        let sun_ship = ship_inv * self.params.sun.dir.as_vec3();
        let ref_tan = self.ref_tan();
        let sockets: Vec<farfall_render::cabin::Socket> = Instrument::ALL
            .iter()
            .copied()
            .filter(|i| i.slotted())
            .filter_map(|i| self.settings.layout.anchor(i).map(|a| (i, a)))
            .map(|(i, a)| {
                let tw = self.dial_tweak(i);
                let dir = anchor_direction(a, ref_tan, cam.aspect);
                farfall_render::cabin::Socket {
                    dir,
                    // The gyro's JET and WARTHOG are the ball itself.
                    style: farfall_render::cabin::seated_style(
                        if i == Instrument::Gyro
                            && matches!(
                                tw.style,
                                settings::GaugeStyle::Jet | settings::GaugeStyle::Warthog
                            )
                        {
                            3
                        } else if tw.style == settings::GaugeStyle::Warthog {
                            // The Warthog's face sits on the DIAL's plate.
                            2
                        } else {
                            tw.style.index()
                        },
                        dir,
                    ),
                    size: tw.size,
                    tilt: tw.tilt,
                    lean: tw.lean,
                }
            })
            .take(6)
            .collect();
        let look = CabinLook {
            glow: self.settings.cockpit_glow,
            metal: self.settings.cockpit_hull,
            on: self.settings.cockpit_frame,
            style: self.settings.gauge_style.index(),
            thrust: self.thrust_look(),
        };
        let fit = bay::fit_views(&self.settings.mounts);
        // The console's control column mirrors the live demand (HOTAS
        // or keys): pitch, roll, yaw, throttle in the pilot's sense — in
        // a helicopter the lever is the collective, so it rides the
        // routed demand (0 flat, full forward all the way up).
        let cu = CabinUniforms::new(cam, self.head(), sun_ship, look, &sockets, &fit)
            .with_craft(self.settings.craft.kind())
            .with_stick(if self.settings.cockpit_stick {
                let c = self.input.controls(self.assist);
                let lever = if self.flying_heli() {
                    heli::route_controls(c).thrust_body.y * 2.0 - 1.0
                } else {
                    -c.thrust_body.z
                };
                [
                    c.torque_body.x as f32,
                    -c.torque_body.z as f32,
                    -c.torque_body.y as f32,
                    lever as f32,
                ]
            } else {
                [0.0; 4]
            });
        let bu = cu.blit(look).with_time(cam.time_s);
        (cu, bu)
    }

    /// What the engines and the RCS are doing, for the plumes and puffs:
    /// main thrust 0..1 and the pitch / yaw / roll demands -1..1 — the
    /// bench's forced numbers when it has them, so the cabin's plumes and
    /// the chase view's agree.
    fn thrust_look(&self) -> [f32; 4] {
        self.bench_thrust.unwrap_or_else(|| {
            let c = self.input.controls(self.assist);
            [
                self.effort,
                c.torque_body.x as f32,
                c.torque_body.y as f32,
                c.torque_body.z as f32,
            ]
        })
    }

    /// The nearest body by altitude, as the altimeter picks it: its
    /// direction in the ship's frame and the sine of its angular radius
    /// (0 far away, 1 filling half the sky) — the light it throws on the
    /// hull and the dust.
    fn nearest_body_ship(&self) -> (Vec3, f32) {
        let ship_inv = self.state.ship.orient.inverse();
        let t = self.state.time_s;
        let mut near: Option<(f64, DVec3, f64)> = None;
        for b in self.params.bodies(t) {
            let rel = b.centre - self.state.ship.pos_m;
            let alt = rel.length() - b.radius_m;
            if near.is_none_or(|n| alt < n.0) {
                near = Some((alt, rel, b.radius_m));
            }
        }
        match near {
            Some((_, rel, r)) => (
                (ship_inv * rel).normalize_or_zero().as_vec3(),
                (r / rel.length().max(1.0)).clamp(0.0, 1.0) as f32,
            ),
            None => (Vec3::ZERO, 0.0),
        }
    }

    /// tan(fov/2) of the reference projection the glass is laid out in.
    fn ref_tan(&self) -> f32 {
        LAYOUT_TAN
    }

    /// The mouse cursor as a point on the glass (reference NDC): the
    /// screen pixel's direction through the live view and the head, back
    /// to where on the laid-out glass that is.
    fn cursor_on_glass(&self, cam: &CameraFrame) -> Option<[f32; 2]> {
        let (cx, cy) = self.cursor?;
        let (w, h) = self.window_size;
        if w < 1.0 || h < 1.0 {
            return None;
        }
        let ndc = [cx / w * 2.0 - 1.0, 1.0 - cy / h * 2.0];
        let t = (cam.fov_y * 0.5).tan();
        Some(self.look.glass_point(ndc, t, self.ref_tan(), cam.aspect))
    }

    /// The pointer for picking and dragging: the cursor in design mode,
    /// the gaze while looking.
    fn pointer(&self, cam: &CameraFrame) -> Option<[f32; 2]> {
        if self.menu.open || self.map_open() {
            // The pause panels are fixed to the screen, and so is the
            // pointer that moves them: the cursor, as it is.
            self.cursor_screen()
        } else if self.design {
            self.cursor_on_glass(cam)
        } else if self.look.engaged() {
            Some(self.look.gaze(self.ref_tan(), cam.aspect))
        } else {
            None
        }
    }

    /// The cursor as screen NDC.
    fn cursor_screen(&self) -> Option<[f32; 2]> {
        let (cx, cy) = self.cursor?;
        let (w, h) = self.window_size;
        if w < 1.0 || h < 1.0 {
            return None;
        }
        Some([cx / w * 2.0 - 1.0, 1.0 - cy / h * 2.0])
    }

    /// Where the text block goes on the SCREEN this frame: the pause
    /// panels sit on the screen and follow the head; the design card is
    /// on the glass, re-projected like a dial, beside the element it
    /// describes. The readout is the pilot's diagnostics, not glassware:
    /// in flat flight it keeps its screen place however the head is
    /// turned (glass-fixed it drifted mid-sky through a spinning bench
    /// capture) — projected with a centred head, so its saved anchor
    /// still means the same spot and it still breathes with the field of
    /// view. In VR "the screen" is the pilot's own face, so that same
    /// centred projection would glue it there instead — `glass_head`
    /// swaps in the real headset rotation there, cockpit-fixed like a
    /// dash dial (SPEC §5.3).
    fn text_screen_anchor(&self, cam: &CameraFrame, px: f32) -> [f32; 2] {
        let a = self.text_anchor(cam.aspect, px);
        if self.menu.open || self.pane_open() || self.card_open {
            a
        } else if self.design {
            on_glass(self.look.rotation(), Vec3::ZERO, cam, self.ref_tan(), a)
        } else if self.vr.is_some() {
            on_glass(
                self.glass_head(),
                self.glass_eye_pos(),
                cam,
                self.ref_tan(),
                a,
            )
        } else {
            on_glass(Quat::IDENTITY, Vec3::ZERO, cam, self.ref_tan(), a)
        }
    }

    /// This frame's text block for the HUD pass: where it sits, whether
    /// it is a flat card, and the card's furniture (band, rules, bar).
    fn hud_block(&self, cam: &CameraFrame, px: f32, height_px: f32) -> HudBlock {
        let mut b = HudBlock::glass(self.text_screen_anchor(cam, px), px, cam.aspect, height_px);
        b.sway = self.holo_sway.sway();
        b.flat = self.menu.open || self.pane_open() || self.card_open;
        b.highlight = self.highlight_row();
        if self.card_open {
            b.extent = Some(card::extent());
            b.rules = card::rules(&self.settings.bindings);
        } else if self.menu.open {
            b.extent = Some(self.menu.extent());
            b.scrollbar = self.menu.scrollbar();
            b.rules = self.menu.rules();
        } else if self.map_open() {
            b.extent = Some(self.map_panel.extent());
            b.rules = self.map_panel.rules();
        } else if self.bay_open() {
            let rows = self.bay_rows().len();
            b.rules = [
                Some(LINE as f32 - 1.5),
                Some(((rows - 1) * LINE) as f32 - 1.5),
            ];
        }
        b
    }

    /// The pause panel's chosen row, for the card's band: (top, height)
    /// in font pixels.
    fn highlight_row(&self) -> Option<(f32, f32)> {
        if self.wizard.is_some() {
            None
        } else if self.menu.open {
            Some(self.menu.cursor_row_px())
        } else if self.map_panel.open {
            Some(self.map_panel.cursor_row_px())
        } else if self.bay_panel.open {
            self.bay_highlight_row()
        } else {
            None
        }
    }

    /// The gyro as a geometric ball: when it is JET or WARTHOG and sits
    /// on the dash, its sphere's placement and the world's up and east in
    /// the ship's frame, for the gyro pass to paint.
    /// `eye_ship`: the pose's own eye seat (`pose.eye_ship`), so the
    /// ball's own ray-sphere cast starts from the same seat as the
    /// ray-marched dash around it (SPEC §5.3), matching `in_dash`.
    fn gyro_ball(
        &self,
        cam: &CameraFrame,
        tw: DialEffective,
        eye_ship: glam::Vec3,
    ) -> Option<(farfall_render::cabin::Placement, glam::Vec3, glam::Vec3)> {
        use farfall_render::cabin::anchor_direction;
        use glam::Vec3;
        if !matches!(
            tw.style,
            settings::GaugeStyle::Jet | settings::GaugeStyle::Warthog
        ) {
            return None;
        }
        let a = self.settings.layout.anchor(Instrument::Gyro)?;
        // The ball is a real sphere cast in the dash, and near the rim of
        // a far-turned view its projection blows out: a corner scissor
        // patch fills with magnified globe (a beige plate poking into
        // frame). Once its glass anchor has left the screen there is no
        // honest picture of it left to draw — cull it.
        let live = on_glass(
            self.glass_head(),
            self.glass_eye_pos(),
            cam,
            self.ref_tan(),
            a,
        );
        if live[0].abs() > 1.2 || live[1].abs() > 1.2 {
            return None;
        }
        let dir = anchor_direction(a, self.ref_tan(), cam.aspect);
        let t = (cam.fov_y * 0.5).tan();
        let place = farfall_render::cabin::Placement::ball(self.head(), t, dir, tw.size, eye_ship)?;
        let ship_inv = self.state.ship.orient.as_quat().inverse();
        let up_w = self.up_world().as_vec3();
        let east_w = {
            let e = Vec3::Y.cross(up_w);
            if e.length() > 1e-4 {
                e.normalize()
            } else {
                Vec3::X
            }
        };
        Some((place, ship_inv * up_w, ship_inv * east_w))
    }

    /// A glass text block is a thing on the glass, like a dial: it scales
    /// with the view the way the dials do (smaller as the field of view
    /// opens), so it never grows across the screen. The pause panels are
    /// on the screen and keep their size.
    fn text_fov_scale(&self, cam: &CameraFrame) -> f32 {
        if self.menu.open || self.map_open() || self.card_open {
            return 1.0;
        }
        let t = (cam.fov_y * 0.5).tan().max(1e-4);
        let mut base = (self.ref_tan() / t).clamp(0.4, 1.25);
        // SPEC §5.3: this whole formula holds *angular* size constant
        // across fov by design (px_canopy's own NDC-per-glyph is
        // fov-independent, so ref_tan/live_tan here cancels back out to
        // ref_tan alone once it reaches the shader) — but the 1.25
        // ceiling above, tuned for flat play's 50-110 degree FOV_MIN/MAX
        // range, is nowhere near what a headset's own much wider render
        // fov needs capped. `vr.text-scale` (ui.readout.size's own VR
        // sibling, `settings::VR_TEXT_SCALE_DEFAULT`) closes that gap —
        // a MEASURED figure (see
        // `tests::the_vr_readout_measures_about_1_2_degrees_a_glyph`),
        // not an estimate: an early guess of 6.0 put a real headset
        // capture's glyph at roughly 10° (a tenth of the vertical
        // field) against a 1.2° target, wrong by close to an order of
        // magnitude. Still a pilot's own knob (VR_TEXT_SCALE_MIN/MAX) —
        // the measured default is a starting point, not a promise.
        if self.vr.is_some() {
            base *= self.settings.vr_text_scale;
        }
        // In flight the block is the readout — a glass element with its
        // own SIZE (ui.readout.size), scaled like any dial.
        if self.panel_open() {
            base
        } else {
            base * self.settings.dials[Instrument::Readout as usize]
                .size
                .clamp(settings::DIAL_SIZE_MIN, settings::DIAL_SIZE_MAX)
        }
    }

    /// How fast it looks, 0..1, for the picture's streaks and cool rim:
    /// the Chaos Drive's field first, else the speed against the wall —
    /// or the wormhole sequence's own stretch, space drawing into threads.
    fn speed_look(&self) -> f32 {
        // Only real speed shows: from a third of the way to the wall.
        let wall = (self.state.ship.vel_mps.length() / sim::RELATIVITY_FROM_MPS) as f32;
        let fast = ((wall - 0.3) / 0.7).clamp(0.0, 1.0) * 0.5;
        (self.hyper * (0.5 + 0.5 * self.hyper_level() as f32))
            .max(fast)
            .max(self.warp.look().stretch)
    }

    /// The drive's look this frame: the wormhole sequence, with the hyper
    /// drive's half-charge over it.
    fn warp_look(&self) -> warp::Look {
        let mut l = self.warp.look().with_hyper(self.hyper);
        // The strain shows: the drive's glow and hum climb toward the slip.
        l.charge = l.charge.max(self.hyper * (0.7 + 0.3 * self.hyper_strain));
        l
    }

    /// A dial's effective settings: its own over the cockpit's.
    fn dial_tweak(&self, i: Instrument) -> DialEffective {
        let tw = self.settings.dials[i as usize];
        DialEffective {
            size: tw.size,
            style: settings::style_for(tw.style.unwrap_or(self.settings.gauge_style), i),
            stay: tw.stay.unwrap_or(self.settings.gauges_stay),
            tilt: tw.tilt_deg.to_radians(),
            lean: tw.lean_deg.to_radians(),
            rotate: tw.rotate_deg.to_radians(),
        }
    }

    /// K: design mode on or off. On: the look locks so the gaze can be
    /// steered, the guide comes up; off: everything is saved.
    fn toggle_design(&mut self) {
        self.design = !self.design;
        if self.design {
            self.menu.open = false;
            self.map_panel.open = false;
            self.bay_panel.open = false;
        } else {
            self.settings.save();
        }
        self.input.release_all();
        log::info!("design mode {}", if self.design { "on" } else { "off" });
    }

    /// The element under the pointer (within reach), in design mode: the
    /// nearest of the dials, the holo3PP, the mini map and the readout.
    fn design_target(&self, aspect: f32) -> Option<DesignEl> {
        let gaze = self.design_pointer(aspect)?;
        let dist = |a: [f32; 2]| {
            let dx = (a[0] - gaze[0]) * aspect;
            let dy = a[1] - gaze[1];
            (dx * dx + dy * dy).sqrt()
        };
        let mut best: Option<(DesignEl, f32)> = None;
        let mut offer = |el: DesignEl, d: f32| {
            if best.is_none_or(|b| d < b.1) {
                best = Some((el, d));
            }
        };
        for i in Instrument::ALL.iter().copied().filter(|i| i.slotted()) {
            if let Some(a) = self.settings.layout.anchor(i) {
                let d = dist(a);
                if d < DRAG_REACH {
                    offer(DesignEl::Dial(i), d);
                }
            }
        }
        if self.holo_active() {
            let a = self.settings.holo_anchor;
            let r = self.settings.holo_size * 0.9;
            if (gaze[0] - a[0]).abs() <= r + 0.02 && (gaze[1] - a[1]).abs() <= r + 0.02 {
                offer(DesignEl::Holo, dist(a));
            }
        }
        if self.mini_map_shown() {
            let a = self.settings.layout.inset(self.mini_map_anchor());
            let [cx, cy, hw] = map::pane_rect_sized(aspect, a, self.mini_map_half_h());
            let hh = hw * aspect;
            if (gaze[0] - cx).abs() <= hw + 0.02 && (gaze[1] - cy).abs() <= hh + 0.02 {
                offer(DesignEl::MiniMap, dist([cx, cy]));
            }
        }
        if self.settings.layout.shown(Instrument::Readout) {
            // The block hangs down-right of its top-left anchor.
            let a = self.settings.readout_anchor;
            if gaze[0] >= a[0] - 0.02
                && gaze[0] <= a[0] + 0.6
                && gaze[1] <= a[1] + 0.02
                && gaze[1] >= a[1] - 0.45
            {
                offer(DesignEl::Readout, dist([a[0] + 0.25, a[1] - 0.2]));
            }
        }
        best.map(|b| b.0)
    }

    /// The design pointer: the mouse cursor on the glass (design mode needs
    /// no locked look — point and click).
    fn design_pointer(&self, aspect: f32) -> Option<[f32; 2]> {
        let cam = self.camera(aspect);
        self.cursor_on_glass(&cam)
    }

    /// A key in design mode: the selected element's own settings.
    fn design_key(&mut self, code: KeyCode, aspect: f32) {
        let Some(el) = self.design_target(aspect) else {
            return;
        };
        match el {
            DesignEl::Dial(i) => {
                let d = &mut self.settings.dials[i as usize];
                match code {
                    KeyCode::Equal | KeyCode::NumpadAdd => {
                        d.size = (d.size + 0.125).min(settings::DIAL_SIZE_MAX);
                    }
                    KeyCode::Minus | KeyCode::NumpadSubtract => {
                        d.size = (d.size - 0.125).max(settings::DIAL_SIZE_MIN);
                    }
                    KeyCode::Tab => d.style = settings::next_dial_style(d.style, i, true),
                    KeyCode::Comma => {
                        d.tilt_deg = (d.tilt_deg - 5.0).max(settings::TILT_MIN);
                    }
                    KeyCode::Period => {
                        d.tilt_deg = (d.tilt_deg + 5.0).min(settings::TILT_MAX);
                    }
                    KeyCode::Semicolon => {
                        d.lean_deg = (d.lean_deg - 5.0).max(settings::TILT_MIN);
                    }
                    KeyCode::Quote => {
                        d.lean_deg = (d.lean_deg + 5.0).min(settings::TILT_MAX);
                    }
                    KeyCode::Digit9 => {
                        d.rotate_deg = (d.rotate_deg - 15.0).max(settings::ROTATE_MIN);
                    }
                    KeyCode::Digit0 => {
                        d.rotate_deg = (d.rotate_deg + 15.0).min(settings::ROTATE_MAX);
                    }
                    KeyCode::KeyF => {
                        d.stay = match d.stay {
                            None => Some(true),
                            Some(true) => Some(false),
                            Some(false) => None,
                        };
                    }
                    KeyCode::Backspace => *d = settings::DialTweak::DEFAULT,
                    _ => {}
                }
            }
            DesignEl::Holo => match code {
                KeyCode::Equal | KeyCode::NumpadAdd => {
                    self.settings.holo_size =
                        (self.settings.holo_size + 0.04).min(settings::HOLO_SIZE_MAX);
                }
                KeyCode::Minus | KeyCode::NumpadSubtract => {
                    self.settings.holo_size =
                        (self.settings.holo_size - 0.04).max(settings::HOLO_SIZE_MIN);
                }
                KeyCode::Backspace => {
                    self.settings.holo_size = Settings::default().holo_size;
                    self.settings.holo_anchor = settings::HOLO_ANCHOR_DEFAULT;
                }
                _ => {}
            },
            DesignEl::MiniMap => {
                let d = &mut self.settings.dials[Instrument::Map as usize];
                match code {
                    KeyCode::Equal | KeyCode::NumpadAdd => {
                        d.size = (d.size + 0.125).min(settings::DIAL_SIZE_MAX);
                    }
                    KeyCode::Minus | KeyCode::NumpadSubtract => {
                        d.size = (d.size - 0.125).max(settings::DIAL_SIZE_MIN);
                    }
                    KeyCode::Backspace => {
                        *d = settings::DialTweak::DEFAULT;
                        // On, at the stock corner: the slot lets go of
                        // any dragged anchor.
                        self.settings.layout.set(Instrument::Map, cockpit::Slot::On);
                    }
                    _ => {}
                }
            }
            DesignEl::Readout => {
                let d = &mut self.settings.dials[Instrument::Readout as usize];
                match code {
                    KeyCode::Equal | KeyCode::NumpadAdd => {
                        d.size = (d.size + 0.125).min(settings::DIAL_SIZE_MAX);
                    }
                    KeyCode::Minus | KeyCode::NumpadSubtract => {
                        d.size = (d.size - 0.125).max(settings::DIAL_SIZE_MIN);
                    }
                    KeyCode::Backspace => {
                        *d = settings::DialTweak::DEFAULT;
                        self.settings.readout_anchor = settings::READOUT_ANCHOR_DEFAULT;
                    }
                    _ => {}
                }
            }
        }
    }

    /// The design card: the selected dial's settings, a few short lines,
    /// and the keys.
    fn design_text(&self, aspect: f32) -> Vec<String> {
        let sized_card = |name: &str, size: String| {
            vec![
                format!("[{name}]"),
                size,
                "- = SIZE  DRAG MOVE".to_string(),
                "BKSP RESET  K DONE".to_string(),
            ]
        };
        match self.design_target(aspect) {
            Some(DesignEl::Holo) => sized_card(
                "HOLO3PP",
                format!("SIZE {:.0}%", self.settings.holo_size * 100.0),
            ),
            Some(DesignEl::MiniMap) => sized_card(
                "MINI MAP",
                format!(
                    "SIZE {:.2}X",
                    self.settings.dials[Instrument::Map as usize].size
                ),
            ),
            Some(DesignEl::Readout) => sized_card(
                "READOUT",
                format!(
                    "SIZE {:.2}X",
                    self.settings.dials[Instrument::Readout as usize].size
                ),
            ),
            Some(DesignEl::Dial(i)) => {
                let d = self.settings.dials[i as usize];
                let eff = self.dial_tweak(i);
                vec![
                    format!("[{}]", i.name()),
                    format!("SIZE {:.2}X", d.size),
                    format!(
                        "STYLE {}{}",
                        eff.style.name(),
                        if d.style.is_none() { " (AUTO)" } else { "" }
                    ),
                    format!(
                        "{}{}  TILT {:+.0}",
                        if eff.stay { "STAY" } else { "FADE" },
                        if d.stay.is_none() { " (AUTO)" } else { "" },
                        d.tilt_deg
                    ),
                    format!("LEAN {:+.0}  ROT {:+.0}", d.lean_deg, d.rotate_deg),
                    "- = SIZE  TAB STYLE  F FADE".to_string(),
                    ", . TILT  ; ' LEAN  9 0 ROT".to_string(),
                    "BKSP RESET  K DONE".to_string(),
                ]
            }
            None => vec![
                "[DESIGN]".to_string(),
                "POINT AT ANY GLASS PIECE:".to_string(),
                "DIALS  MAP  HOLO  READOUT".to_string(),
                "CLICK DRAG TO MOVE  K DONE".to_string(),
            ],
        }
    }

    /// G: landing mode on or off.
    fn toggle_landing(&mut self) {
        self.landing = !self.landing;
        if !self.landing {
            self.touchdown = None;
        }
        log::info!("landing mode {}", if self.landing { "on" } else { "off" });
    }

    /// On the ground — down or landed — as the sim says.
    fn on_ground(&self) -> bool {
        self.state.ship.ground != sim::Ground::Flight
    }

    /// The ground's speed under the ship: over the body it is on, or the
    /// nearest one.
    fn ground_speed(&self) -> f64 {
        let body = match self.state.ship.ground {
            sim::Ground::Down { body, .. } | sim::Ground::Landed { body, .. } => body,
            sim::Ground::Flight => 0,
        };
        let v = self.params.body_velocities(self.state.time_s)[body];
        let rel = self.state.ship.vel_mps - v;
        let up = self.up_world();
        (rel - up * rel.dot(up)).length()
    }

    /// The pilot's own ship parameters, by the SHIP page's CRAFT row
    /// (SPEC §6.5c): the fighter's set, or the FARFALL helicopter's.
    fn own_ship_params(&self) -> sim::ShipParams {
        match self.settings.craft {
            bay::Craft::Fighter => self.fighter_ship,
            bay::Craft::Helicopter => heli::farfall_heli_params(),
        }
    }

    /// Is the sim's ship a helicopter right now — a pad's, or the
    /// pilot's own craft?
    fn flying_heli(&self) -> bool {
        self.helis.in_heli || self.settings.craft == bay::Craft::Helicopter
    }

    /// DISEMBARK (I): leave the ship. LANDED, it walks out — the EVA
    /// walker (SPEC §6.5b) — unless a pad's helicopter is closer business;
    /// anywhere else the readout says why not.
    fn disembark(&mut self) {
        let landed = matches!(self.state.ship.ground, sim::Ground::Landed { .. });
        if self.helis.in_heli {
            // Set down beside the fighter, the same key swaps back.
            if landed {
                if let Some(f) = self.helis.fighter {
                    if (f.pos_m - self.state.ship.pos_m).length() <= heli::BOARD_RANGE_M {
                        let heli_state = self.state.ship;
                        if let Some(fighter) = self.helis.disembark(heli_state) {
                            self.state.ship = fighter;
                            self.params.ship = self.own_ship_params();
                            self.input.release_all();
                            self.disembark_notice = Some(("BOARDED SHIP", Instant::now()));
                            log::info!("heli: back in the fighter");
                            return;
                        }
                    }
                }
                self.disembark_notice = Some(("LAND BY YOUR SHIP TO SWAP", Instant::now()));
                return;
            }
            self.disembark_notice = Some(("DISEMBARK  NOT LANDED", Instant::now()));
            return;
        }
        // Landed beside a pad's helicopter, the key boards it: the sim's
        // ship becomes the helicopter - parameters and all - and the
        // fighter waits exactly where it stands.
        if landed && self.settings.helis {
            if let Some((i, _)) = self.helis.nearest_heli(&self.params, self.state.ship.pos_m) {
                let fighter = self.state.ship;
                self.state.ship = self.helis.board(&self.params, i, fighter);
                self.params.ship = heli::heli_params();
                self.input.release_all();
                self.disembark_notice = Some(("BOARDED HELI", Instant::now()));
                log::info!("heli: boarded pad {i}'s helicopter");
                return;
            }
        }
        if landed {
            self.enter_eva();
            return;
        }
        let notice = landing::disembark_notice(self.state.ship.ground);
        self.disembark_notice = Some((notice, Instant::now()));
        log::info!("disembark: {notice}");
    }

    fn eva_active(&self) -> bool {
        self.eva.is_some()
    }

    /// The walk-out: boots on the ground beside the ship, facing the
    /// hull. The ship stays LANDED exactly where it is.
    fn enter_eva(&mut self) {
        let sim::Ground::Landed { body, .. } = self.state.ship.ground else {
            return;
        };
        let b = self.params.bodies(self.state.time_s)[body];
        self.eva = Some(eva::Walker::disembarked(
            body,
            self.state.ship.pos_m - b.centre,
            self.state.ship.orient,
            b.radius_m,
            eva::EXIT_M,
        ));
        self.eva_keys = eva::Keys::default();
        self.input.release_all();
        log::info!("eva: on foot beside the ship");
    }

    /// How far the boots stand from the ship, metres.
    fn eva_ship_m(&self) -> Option<f64> {
        let w = self.eva.as_ref()?;
        let b = self.params.bodies(self.state.time_s)[w.body];
        Some((b.centre + w.feet_m - self.state.ship.pos_m).length())
    }

    /// The DISEMBARK key on foot: board the ship, if it is in reach.
    fn try_board(&mut self) {
        let Some(dist) = self.eva_ship_m() else {
            return;
        };
        if dist <= eva::BOARD_RANGE_M {
            self.eva = None;
            self.eva_keys = eva::Keys::default();
            self.input.release_all();
            self.disembark_notice = Some(("BOARDED SHIP", Instant::now()));
            log::info!("eva: back in the seat");
        } else {
            log::info!("eva: the ship is {dist:.0} m away — walk back to board");
        }
    }

    /// A movement key on foot. The translation binds walk, BOOST's key
    /// runs, BRAKE's key jumps — all through the pilot's own bindings.
    fn eva_key(&mut self, code: KeyCode, pressed: bool) {
        let b = &self.settings.bindings;
        let k = &mut self.eva_keys;
        match code {
            c if c == b.key_for(input::Action::ThrustForward) => k.fwd = pressed,
            c if c == b.key_for(input::Action::ThrustBack) => k.back = pressed,
            c if c == b.key_for(input::Action::StrafeLeft) => k.left = pressed,
            c if c == b.key_for(input::Action::StrafeRight) => k.right = pressed,
            c if c == b.named(Named::Boost) => k.run = pressed,
            c if c == b.named(Named::Brake) => k.jump = pressed,
            _ => {}
        }
    }

    /// One fixed step of the walker, after the sim's own. A panel up
    /// means the keys are nobody's: the boots stand still.
    fn step_eva(&mut self) {
        let Some(body) = self.eva.as_ref().map(|w| w.body) else {
            return;
        };
        let keys = if self.panel_open() {
            eva::Keys::default()
        } else {
            self.eva_keys
        };
        let b = self.params.bodies(self.state.time_s)[body];
        if let Some(w) = self.eva.as_mut() {
            w.step(&keys, b.mu, b.radius_m, sim::DT);
        }
    }

    /// The bench's walker: out at the bench's distance, looking back at
    /// the hull with a little pitch up to take it in.
    fn stage_eva_bench(&mut self) {
        let Some(body) = self.eva.as_ref().map(|w| w.body) else {
            return;
        };
        let b = self.params.bodies(self.state.time_s)[body];
        let ship_local = self.state.ship.pos_m - b.centre;
        let mut w = eva::Walker::disembarked(
            body,
            ship_local,
            self.state.ship.orient,
            b.radius_m,
            eva::BENCH_OUT_M,
        );
        w.tilt(0.10);
        self.eva = Some(w);
    }

    /// The suit's readout lines, on foot.
    fn eva_text(&self) -> Option<String> {
        let dist = self.eva_ship_m()?;
        let key = input::key_name(self.bind(Named::Disembark));
        Some(eva::lines(dist, key).join("\n"))
    }

    /// The boarding offer for the readout: landed beside a helicopter (or
    /// back beside the fighter), the DISEMBARK key reads as BOARD.
    fn board_offer(&self) -> Option<String> {
        if !matches!(self.state.ship.ground, sim::Ground::Landed { .. }) {
            return None;
        }
        let key = input::key_name(self.bind(Named::Disembark));
        if self.helis.in_heli {
            let f = self.helis.fighter?;
            ((f.pos_m - self.state.ship.pos_m).length() <= heli::BOARD_RANGE_M)
                .then(|| format!("{key} BOARD SHIP"))
        } else if self.settings.helis {
            self.helis
                .nearest_heli(&self.params, self.state.ship.pos_m)
                .map(|_| format!("{key} BOARD HELI"))
        } else {
            None
        }
    }

    /// The landing readout lines, newline-joined, if there are any: the
    /// approach in LANDING mode; DOWN or LANDED whenever the ship is on
    /// the ground, mode or no mode.
    fn landing_text(&self) -> Option<String> {
        let (altitude_m, vspeed_mps) = self.altitude_vspeed();
        let offer = self.board_offer();
        let notice = self
            .disembark_notice
            .filter(|(_, at)| at.elapsed() < Duration::from_secs(4))
            .map(|(n, _)| n)
            .or(offer.as_deref());
        let view = landing::View {
            mode: self.landing,
            ground: self.state.ship.ground,
            touchdown: self.touchdown,
            vspeed_mps,
            altitude_m,
            ground_speed_mps: self.ground_speed(),
            tilt_deg: landing::tilt_deg(self.state.ship.orient, self.up_world()),
            record: self.touchdown_record,
            notice,
            disembark_key: input::key_name(self.bind(Named::Disembark)),
        };
        let lines = landing::lines(&view);
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    /// Gravity's up at the ship, world frame — whichever body is pulling.
    fn up_world(&self) -> DVec3 {
        let g = sim::gravity_all(&self.params, self.state.time_s, self.state.ship.pos_m);
        if g.length() > 1e-9 {
            -g.normalize()
        } else {
            self.state.ship.pos_m / self.state.ship.pos_m.length().max(1.0)
        }
    }

    fn attitude(&self) -> Attitude {
        Attitude::from_world(
            self.state.ship.orient.as_quat(),
            self.up_world().as_vec3(),
            self.state.ship.vel_mps.as_vec3(),
        )
    }

    /// Fire the drive at the plan. Refused mid-sequence.
    fn engage_warp(&mut self) {
        if self.helis.in_heli {
            log::info!("warp: the helicopter has no drives");
            return;
        }
        if self.warp.engage() {
            self.input.release_all();
            log::info!(
                "warp: engaged to {} at {:.2} radii",
                self.settings.plan.dest.name(),
                self.settings.plan.safe_radii
            );
        }
    }

    /// The hyper drive's price. Held, the field strains the drive — faster
    /// when it is running hard — and past its limit the drive slips: the
    /// whole wormhole fires uncharged and drops the ship into an unstable
    /// orbit of some body, any body, going any way. Let go of it and the
    /// field collapses: whatever space was doing stops, and the ship is
    /// left at a crawl down its nose. Off, the strain eases.
    fn run_hyper_strain(&mut self, dt: f32) {
        let held =
            self.input.controls(self.assist).hyper && !self.warp.active() && !self.helis.in_heli;
        if self.hyper_was && !held {
            log::info!(
                "chaos drive: released at {:.3e} m/s, entropy {:.0}%",
                self.state.ship.vel_mps.length(),
                self.hyper_strain * 100.0
            );
        }
        self.hyper_was = held;
        if held {
            let run = (self.state.ship.vel_mps.length() / self.params.ship.hyper_max_mps) as f32;
            self.hyper_strain += dt / HYPER_STRAIN_S * (0.3 + 0.7 * run.clamp(0.0, 1.0));
            if self.hyper_strain >= self.slip_at {
                self.hyper_strain = 0.0;
                self.slip_at = 0.7 + 0.3 * self.next_unit();
                if self.warp.slip() {
                    self.pending_slip = true;
                    self.input.release_all();
                    log::warn!("hyper: the drive slipped");
                }
            }
        } else {
            self.hyper_strain = (self.hyper_strain - dt / HYPER_EASE_S).max(0.0);
        }
    }

    /// The force field this frame: the head's frame, the clock, the
    /// setting and the shell's recent impacts.
    fn shield_uniforms(&self, pose: &ViewPose) -> ShieldUniforms {
        ShieldUniforms::new(
            &pose.cam,
            pose.head,
            self.started.elapsed().as_secs_f32(),
            self.settings.shield,
            &self.impacts,
        )
        .with_hyper(self.hyper)
        .with_eye(pose.eye_ship.as_vec3())
    }

    /// The mimics and the haul, one fixed step, after the arms: what
    /// landed on a rock chips ore off it and may show a ship; our slugs
    /// meet the ships; the ships fly, talk or shoot; their slugs ring the
    /// shield.
    fn step_mimics(&mut self) {
        let t = self.state.time_s;
        let own = arms::Ship {
            pos: self.state.ship.pos_m,
            vel: self.state.ship.vel_mps,
            orient: self.state.ship.orient,
            aim: self.aim_world(),
        };
        let landed: Vec<arms::Landed> = self.arms.landed.clone();
        for l in landed {
            self.haul.on_hit(&l.rock, l.energy_j, l.destroyed, t);
            if !l.destroyed {
                self.mimics.on_rock_struck(&l.rock, t, own.pos);
            }
        }
        // A shroud that has gone takes its rock out of the belt for good.
        for id in self.mimics.shroud_off(t) {
            if let Some(i) = self.belt.rocks.iter().position(|r| r.id == id) {
                self.belt.rocks.swap_remove(i);
            }
            self.belt.wounds.remove(&id);
            self.belt.dead.insert(id);
        }
        let mut breaks = self
            .mimics
            .take_fire(&mut self.arms, &mut self.haul, t, sim::DT);
        // The miners: placed when the ring goes live, shot at like any
        // ship, and stepped with the pilot's held rock kept off their list.
        self.miners.populate(&own, &self.belt);
        breaks.extend(self.miners.take_fire(
            &mut self.arms,
            &mut self.haul,
            &mut self.mimics,
            t,
            sim::DT,
        ));
        let held = match self.hold.target {
            Some((hold::Target::Rock(id), _)) => Some(id),
            _ => None,
        };
        let chance = self.mimics.chance;
        self.miners.step(
            t,
            sim::DT,
            &own,
            &mut self.belt,
            held,
            chance,
            &mut self.mimics,
        );
        for (at, vel, seed) in breaks {
            // A wreck comes apart like a rock does: shards off the break.
            let rock = belt::Rock {
                id: (0, 0, 0, 255),
                pos: at,
                vel,
                radius_m: 6.0,
                seed,
                spin: 0.0,
            };
            let n = self.arms.shards_per_break;
            self.arms
                .throw_shards(t, at, rock, (own.pos - at).normalize_or_zero(), n, true);
        }
        self.mimics.step(t, sim::DT, &own);
        let hits: Vec<mimic::OwnHit> = self.mimics.own_hits.clone();
        for h in hits {
            let ship_inv = self.state.ship.orient.inverse();
            let dir = (ship_inv * h.from).as_vec3().normalize_or_zero();
            self.strikes = self.strikes.wrapping_add(1);
            self.strike_size = h.size;
            self.shake.kick(0.9, 0.0);
            self.impacts.insert(
                0,
                Impact {
                    dir,
                    at_s: self.started.elapsed().as_secs_f32(),
                    size: h.size,
                },
            );
            self.impacts.truncate(farfall_render::shield::IMPACTS);
        }
    }

    /// HOLD (O): take the lock on what is under the sight, or let it go.
    fn toggle_hold(&mut self) {
        if self.hold.engaged() {
            self.hold.release();
            log::info!("hold: released");
            return;
        }
        let own = arms::Ship {
            pos: self.state.ship.pos_m,
            vel: self.state.ship.vel_mps,
            orient: self.state.ship.orient,
            aim: self.aim_world(),
        };
        if self
            .hold
            .engage(&own, own.aim, &self.belt, &self.mimics, &self.miners)
        {
            let (t, off) = self.hold.target.unwrap();
            log::info!("hold: locked on a {} at {:.0} m", t.name(), off.length());
        } else {
            self.mimics.line = Some((
                "HOLD: NOTHING UNDER THE SIGHT".to_string(),
                self.state.time_s + 2.0,
            ));
        }
    }

    /// The hold's line for the readout.
    fn hold_text(&self) -> Option<String> {
        self.hold.text()
    }

    /// The engines' effort under the hold: the computer's demand.
    fn hold_effort(&self) -> f64 {
        if self.hold.engaged() {
            self.hold.demand.abs().max_element().clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// The haul's line for the readout, while a gain is fresh.
    fn haul_text(&self) -> Option<String> {
        self.haul.text(self.state.time_s, 6.0)
    }

    /// Every other ship in the air, for anything that marks them (the
    /// sight, the hologram): world position and kind — 0 hailing, 1
    /// hostile, 2 wreck, 3 a miner, 4 a hostile miner — mimics out of
    /// their shrouds first, then the miners.
    fn contacts(&self, t: f64) -> Vec<(DVec3, u8)> {
        self.mimics
            .ships
            .iter()
            .filter(|m| !m.shrouded(t))
            .map(|m| (m.pos, m.kind()))
            .chain(self.miners.ships.iter().map(|m| (m.pos, m.kind())))
            .collect()
    }

    /// The mimics' line: what was said, what is happening, the hull.
    fn mimic_text(&self) -> Option<String> {
        self.mimics.text()
    }

    /// The ships out of the rocks, relative to the eye, the rocks along
    /// to hide them.
    fn mimic_uniforms(&self, pose: &ViewPose) -> MimicUniforms {
        let cam = &pose.cam;
        let head = pose.head;
        if self.mimics.ships.is_empty()
            && self.miners.ships.is_empty()
            && self.helis.fighter.is_none()
        {
            return MimicUniforms::none(cam, head);
        }
        let ship_inv = self.state.ship.orient.inverse();
        let eye = self.eye_m(pose);
        let t = self.state.time_s;
        let now = self.started.elapsed().as_secs_f32();
        let to_ship = |p: DVec3| (ship_inv * (p - eye)).as_vec3();
        let ships: Vec<MimicView> = self
            .mimics
            .ships
            .iter()
            .filter(|m| !m.shrouded(t) || m.reveal(t) > 0.0)
            .map(|m| MimicView {
                size: self.mimics.size.clamp(0.5, 3.0),
                ..MimicView::plain(
                    to_ship(m.pos),
                    (ship_inv * m.orient).as_quat(),
                    m.reveal(t),
                    m.effort,
                    m.kind(),
                    m.wound(),
                    m.seed,
                )
            })
            .chain(self.miners.ships.iter().map(|m| {
                // The beam ends on the claim's face, toward the miner.
                let beam = m.mining().then(|| {
                    self.belt
                        .rocks
                        .iter()
                        .find(|r| Some(r.id) == m.claim)
                        .map(|r| {
                            let toward = (m.pos - r.pos).normalize_or_zero();
                            to_ship(r.pos + toward * r.radius_m * 0.98)
                        })
                });
                MimicView {
                    size: m.size() as f32,
                    tier: m.tier() as u8,
                    shield: m.sheen,
                    beam: beam.flatten(),
                    ..MimicView::plain(
                        to_ship(m.pos),
                        (ship_inv * m.orient).as_quat(),
                        1.0,
                        m.effort,
                        m.kind(),
                        m.wound(),
                        m.seed,
                    )
                }
            }))
            .chain(self.helis.fighter.iter().map(|f| {
                // The fighter waiting where it was left, its beacon on.
                MimicView::plain(
                    to_ship(f.pos_m),
                    (ship_inv * f.orient).as_quat(),
                    1.0,
                    0.0,
                    0,
                    0.0,
                    0.31,
                )
            }))
            .collect();

        let rocks: Vec<Occluder> = self
            .belt
            .rocks
            .iter()
            .map(|r| Occluder {
                centre: to_ship(r.pos),
                radius_m: r.radius_m as f32,
            })
            .collect();
        let sun_ship = (ship_inv * self.params.sun.dir).as_vec3();
        MimicUniforms::new(cam, head, now, sun_ship, &ships, &rocks)
    }

    /// The entropy line for the readout, once there is any.
    fn strain_text(&self) -> Option<String> {
        (self.hyper_strain > 0.02).then(|| {
            let pct = (self.hyper_strain * 100.0).round();
            let chaos = chaos_level(self.hyper_strain, self.slip_at);
            if chaos > 0.3 {
                format!("ENTROPY {pct:.0}%  CHAOS")
            } else if self.hyper_strain > 0.5 {
                format!("ENTROPY {pct:.0}%  !")
            } else {
                format!("ENTROPY {pct:.0}%")
            }
        })
    }

    /// The drive's charge for the field: how far along the entropy is —
    /// the speed climbs with it, the slip with it too. A skill game.
    fn hyper_level(&self) -> f64 {
        (self.hyper_strain / self.slip_at.max(1e-3)).clamp(0.0, 1.0) as f64
    }

    /// WARP STOP: all speed and all spin taken out of the ship at once —
    /// it is warped in place to rest against the nearest body — and the
    /// image it leaves behind carries on down the old vector for a moment.
    fn warp_stop(&mut self) {
        if self.warp.active() {
            return;
        }
        let t = self.state.time_s;
        let bodies = self.params.bodies(t);
        let vels = self.params.body_velocities(t);
        let pos = self.state.ship.pos_m;
        let nearest = bodies
            .iter()
            .zip(vels.iter())
            .min_by(|(a, _), (b, _)| {
                let da = (pos - a.centre).length() / a.radius_m;
                let db = (pos - b.centre).length() / b.radius_m;
                da.partial_cmp(&db).unwrap()
            })
            .map(|(_, v)| *v)
            .unwrap_or(DVec3::ZERO);
        let rel = self.state.ship.vel_mps - nearest;
        if rel.length() > 1e-6 {
            self.ghost = Some(Ghost {
                orient: self.state.ship.orient,
                dir_world: rel.normalize(),
                at_s: self.started.elapsed().as_secs_f32(),
            });
        }
        log::info!(
            "warp stop: {:.3e} m/s and {:.2} rad/s taken out",
            rel.length(),
            self.state.ship.ang_vel_radps.length()
        );
        self.state.ship.vel_mps = nearest;
        self.state.ship.ang_vel_radps = DVec3::ZERO;
        self.jumps = self.jumps.wrapping_add(1);
        self.hyper_strain = (self.hyper_strain * 0.5).max(0.0);
    }

    /// The belt's live rocks for the shader: relative to the head, in the
    /// ship's frame; nothing when the ship is nowhere near the ring.
    fn belt_uniforms(&self, pose: &ViewPose) -> BeltUniforms {
        let ship_inv = self.state.ship.orient.inverse();
        let eye = self.eye_m(pose);
        let now = self.started.elapsed().as_secs_f32();
        let rocks: Vec<RockView> = self
            .belt
            .rocks
            .iter()
            .map(|r| RockView {
                centre: (ship_inv * (r.pos - eye)).as_vec3(),
                radius_m: r.radius_m as f32,
                seed: r.seed,
                phase: r.spin * now,
            })
            .collect();
        let sun_ship = (ship_inv * self.params.sun.dir).as_vec3();
        BeltUniforms::new(&pose.cam, pose.head, now, sun_ship, 1.0, &rocks)
    }

    /// Where the guns point, world frame: the gaze while looking, within
    /// the mounts' gimbal; the nose otherwise.
    fn aim_world(&self) -> DVec3 {
        self.state.ship.orient * self.aim_ship().0
    }

    /// Where the guns point in the ship's frame: the gaze within the
    /// gimbal, else the gimbal's edge nearest it — and whether it was
    /// held back.
    fn aim_ship(&self) -> (DVec3, bool) {
        let nose = DVec3::NEG_Z;
        let gaze = self.look.rotation().as_dquat() * DVec3::NEG_Z;
        let ang = gaze.dot(nose).clamp(-1.0, 1.0).acos();
        if ang <= GIMBAL_RAD {
            return (gaze, false);
        }
        let axis = nose.cross(gaze).normalize_or_zero();
        if axis == DVec3::ZERO {
            (nose, true)
        } else {
            (DQuat::from_axis_angle(axis, GIMBAL_RAD) * nose, true)
        }
    }

    /// The camera's head: the pilot's look with the helmet camera's
    /// shake on it. Every cockpit-frame pass takes this, so the cabin,
    /// the glass and the world all move together.
    fn head(&self) -> glam::Quat {
        // A headset is the head: no freelook, and no shake — a helmet
        // camera's jolt is a sickness in a headset.
        if let Some(vr) = &self.vr {
            return vr.eyes[self.vr_eye.min(1)].head;
        }
        self.look.rotation() * self.shake.rotation()
    }

    /// The rotation most "glass" elements — glass-style dials, the mini
    /// map, the design guide's anchors — are pinned against: the pilot's
    /// real look, whether that is mouse-driven freelook (flat) or a
    /// headset's own orientation (VR) — never `head()`'s shake, which
    /// would wobble an instrument reading. In flat flight this is
    /// exactly `self.look.rotation()`, as it always was; in VR it is the
    /// active eye's own head, so these elements stay cockpit-fixed
    /// instead of freezing at whatever the session started facing (see
    /// `text_screen_anchor` for the readout's own, different rule: it
    /// stays screen-fixed on purpose in flat flight, and only VR needs
    /// this same swap for it).
    fn glass_head(&self) -> glam::Quat {
        match &self.vr {
            Some(vr) => vr.eyes[self.vr_eye.min(1)].head,
            None => self.look.rotation(),
        }
    }

    /// The active eye's own seat, for `on_glass`'s parallax shift — zero
    /// in flat flight, always (that shift is a no-op there by
    /// construction). Deliberately the eye's own small IPD-scale offset
    /// alone, not `pose.eye_ship`: a glass overlay sits in front of the
    /// pilot's face regardless of which pose (cockpit, chase, EVA) the
    /// world is drawn from, never at a chase rig's own seat.
    fn glass_eye_pos(&self) -> Vec3 {
        match &self.vr {
            Some(vr) => vr.eyes[self.vr_eye.min(1)].pos,
            None => Vec3::ZERO,
        }
    }

    /// The gun sight this frame: off with a panel up or in design.
    fn sight_uniforms(&self, pose: &ViewPose) -> SightUniforms {
        let cam = &pose.cam;
        if self.panel_open() {
            return SightUniforms::none(cam);
        }
        // The mimics' markers ride this pass whatever the sight setting:
        // strength 0 hides the gun's reticle, never the way to a ship.
        let t = self.state.time_s;
        let ship_inv = self.state.ship.orient.inverse();
        let eye = self.eye_m(pose);
        let mut marks = [None; farfall_render::sight::MARKS];
        for (slot, (pos, kind)) in marks.iter_mut().zip(self.contacts(t)) {
            *slot = Some(((ship_inv * (pos - eye)).as_vec3(), kind));
        }
        let (aim, clamped) = self.aim_ship();
        let w = self.arms.selected;
        let mut barrels = [None; farfall_render::sight::BARRELS];
        let mounts: &[DVec3] = match w {
            arms::Weapon::Cannon => &[arms::WING_L, arms::WING_R],
            arms::Weapon::Rail => &[arms::NOSE],
        };
        for (b, m) in barrels.iter_mut().zip(mounts.iter()) {
            *b = Some(m.as_vec3());
        }
        SightUniforms::new(
            cam,
            pose.head,
            &SightScene {
                aim: aim.as_vec3(),
                gaze: (self.look.rotation() * glam::Vec3::NEG_Z),
                gimbal_rad: GIMBAL_RAD as f32,
                clamped,
                barrels,
                kind: w.kind(),
                heat: self.arms.heat_of(w),
                charge: if w == arms::Weapon::Rail {
                    self.arms.charge
                } else {
                    0.0
                },
                jammed: self.arms.jammed_of(w),
                empty: self.arms.ammo_of(w) == 0,
                strength: self.settings.arms_sight,
                marks,
            },
        )
    }

    /// The arms' line for the readout, while there is anything to say.
    fn arms_text(&self) -> Option<String> {
        let a = &self.arms;
        (a.heat.iter().any(|&h| h > 0.01) || a.charge > 0.0 || a.jammed.iter().any(|&j| j))
            .then(|| a.text())
    }

    /// The arms' light this frame: slugs and bursts relative to the head,
    /// the rocks along for occlusion.
    fn tracer_uniforms(&self, pose: &ViewPose) -> TracerUniforms {
        let cam = &pose.cam;
        let head = pose.head;
        if self.arms.slugs.is_empty() && self.arms.bursts.is_empty() && self.mimics.slugs.is_empty()
        {
            return TracerUniforms::none(cam, head);
        }
        let ship_inv = self.state.ship.orient.inverse();
        let ship_pos = self.state.ship.pos_m;
        let ship_vel = self.state.ship.vel_mps;
        let now = self.started.elapsed().as_secs_f32();
        let t = self.state.time_s;
        let to_ship = |p: DVec3| (ship_inv * (p - ship_pos)).as_vec3();
        let slugs: Vec<SlugView> = self
            .arms
            .slugs
            .iter()
            .map(|sl| {
                let age = (t - sl.born_s).max(0.0);
                let trail = (sl.vel - ship_vel) * age.min(TRACER_TRAIL_S);
                SlugView {
                    head: to_ship(sl.pos),
                    tail: to_ship(sl.pos - trail),
                    kind: sl.weapon.kind(),
                    age_s: age as f32,
                }
            })
            .chain(self.mimics.slugs.iter().map(|sl| {
                let age = (t - sl.born_s).max(0.0);
                let trail = (sl.vel - ship_vel) * age.min(TRACER_TRAIL_S);
                SlugView {
                    head: to_ship(sl.pos),
                    tail: to_ship(sl.pos - trail),
                    kind: 2,
                    age_s: age as f32,
                }
            }))
            .collect();
        let bursts: Vec<BurstView> = self
            .arms
            .bursts
            .iter()
            .map(|b| {
                let age = (t - b.at_s).max(0.0);
                BurstView {
                    at: to_ship(b.pos + b.vel * age),
                    age_s: age as f32,
                    kind: b.kind,
                    size: b.size,
                    seed: b.seed,
                }
            })
            .collect();
        let rocks: Vec<Occluder> = self
            .belt
            .rocks
            .iter()
            .map(|r| Occluder {
                centre: to_ship(r.pos),
                radius_m: r.radius_m as f32,
            })
            .collect();
        let sun_ship = (ship_inv * self.params.sun.dir).as_vec3();
        TracerUniforms::new(
            cam,
            head,
            now,
            self.settings.arms_glow,
            sun_ship,
            &TracerScene {
                slugs: &slugs,
                bursts: &bursts,
                rocks: &rocks,
            },
        )
    }

    /// The scars this frame, each on its rock, in the ship's frame.
    fn scar_uniforms(&self, pose: &ViewPose) -> ScarUniforms {
        let cam = &pose.cam;
        let head = pose.head;
        if self.arms.scars.is_empty() {
            return ScarUniforms::none(cam, head);
        }
        let ship_inv = self.state.ship.orient.inverse();
        let ship_pos = self.state.ship.pos_m;
        let t = self.state.time_s;
        let to_ship = |p: DVec3| (ship_inv * (p - ship_pos)).as_vec3();
        let scars: Vec<ScarView> = self
            .arms
            .scars
            .iter()
            .filter_map(|sc| {
                let rock = self.belt.rocks.iter().find(|r| r.id == sc.rock)?;
                Some(ScarView {
                    centre: to_ship(rock.pos),
                    radius_m: rock.radius_m as f32,
                    dir: (ship_inv * sc.dir).as_vec3(),
                    size_m: sc.size_m,
                    heat: scar_heat((t - sc.born_s) as f32, self.settings.arms_scar_cool),
                    seed: sc.seed,
                })
            })
            .collect();
        let rocks: Vec<farfall_render::scar::Occluder> = self
            .belt
            .rocks
            .iter()
            .map(|r| farfall_render::scar::Occluder {
                centre: to_ship(r.pos),
                radius_m: r.radius_m as f32,
            })
            .collect();
        ScarUniforms::new(
            cam,
            head,
            self.settings.arms_glow,
            &ScarScene {
                scars: &scars,
                rocks: &rocks,
            },
        )
    }

    /// The shards this frame, in the ship's frame, with the rocks that
    /// can hide them.
    /// The dust about the eye this frame: where the eye is on the world
    /// lattice, how thick the dust is here (the belt, a planet's air, a
    /// floor in deep space), what it rests in (the local orbit about the
    /// nearest body), what hides it (that body), and the cabin's light.
    fn dust_uniforms(&self, pose: &ViewPose, target_height_px: f32) -> DustUniforms {
        let t = self.state.time_s;
        let ship = &self.state.ship;
        let eye = self.eye_m(pose);
        let bodies = self.params.bodies(t);
        let vels = self.params.body_velocities(t);
        let mut near = 0;
        for (i, b) in bodies.iter().enumerate() {
            let alt = (b.centre - ship.pos_m).length() - b.radius_m;
            let best = (bodies[near].centre - ship.pos_m).length() - bodies[near].radius_m;
            if alt < best {
                near = i;
            }
        }
        let body = bodies[near];
        let rel = body.centre - ship.pos_m;
        let uranus = warp::Destination::Uranus.body(&self.params, t);
        let belt = belt::Belt::ring_density(&uranus, eye) as f32;
        let air = (sim::atmo_density(&self.params.planet, ship.pos_m.length())
            / self.params.planet.atmo_rho0.max(1e-12)) as f32;
        let density = farfall_render::dust::density(belt, air, self.hyper, self.settings.dust);
        let cabin = (pose.eye_ship == DVec3::ZERO && self.settings.cockpit_frame)
            .then_some((pose.head, self.settings.cockpit_glow));
        DustUniforms::new(
            &pose.cam,
            &DustScene {
                eye_m: eye,
                drift_mps: farfall_render::dust::drift(
                    rel,
                    body.mu,
                    vels[near],
                    ship.vel_mps,
                    self.air_ratio(),
                ),
                sun_dir: self.params.sun.dir.as_vec3(),
                density,
                setting: self.settings.dust,
                occluder_rel: (body.centre - eye).as_vec3(),
                occluder_radius_m: body.radius_m as f32,
                cabin,
                target_height_px,
            },
        )
    }

    /// The wind about the ship this frame: the sim's own field sampled at
    /// the ship and a gap above it (one source of truth — the shader only
    /// interpolates between the two samples), how visible the air is here,
    /// and the WIND setting.
    fn wind_uniforms(&self, pose: &ViewPose, target_height_px: f32) -> WindUniforms {
        let ship = &self.state.ship;
        let t = self.state.time_s;
        let r = ship.pos_m.length();
        let up = if r > 0.0 { ship.pos_m / r } else { DVec3::Y };
        let low = self.wind_now();
        let high = if self.bench_wind.is_some() {
            low
        } else {
            sim::wind_mps(
                &self.params,
                ship.pos_m + up * farfall_render::wind::SAMPLE_GAP_M,
                t,
            )
        };
        // Visibility follows the air on a gentle curve, (rho/rho0)^0.22,
        // so the ribbons still read in the thin fast air of the jet band
        // and are exactly nothing above the atmosphere (rho = 0 there).
        let rho = sim::atmo_density(&self.params.planet, r) / self.params.planet.atmo_rho0;
        let air = (rho.max(0.0) as f32).powf(0.22);
        WindUniforms::new(
            &pose.cam,
            &WindScene {
                eye_m: self.eye_m(pose),
                wind_low: low.as_vec3(),
                wind_high: high.as_vec3(),
                up: up.as_vec3(),
                air,
                setting: self.settings.wind,
                target_height_px,
            },
        )
    }

    /// The wind at the ship: the sim's field, or the bench's forced one.
    fn wind_now(&self) -> DVec3 {
        if let Some((mps, from_deg)) = self.bench_wind {
            let (nose_h, right_h, _) = self.horizontal_frame();
            let a = from_deg.to_radians();
            return -(nose_h * a.cos() + right_h * a.sin()) * mps;
        }
        sim::wind_mps(&self.params, self.state.ship.pos_m, self.state.time_s)
    }

    /// The pilot's compass on the ground plane: the nose and the right
    /// hand projected into the local horizontal, and the up they share.
    /// Nose-down, the ship's own up stands in so the frame never folds.
    fn horizontal_frame(&self) -> (DVec3, DVec3, DVec3) {
        let ship = &self.state.ship;
        let up = ship.pos_m.normalize_or_zero();
        let flat = |v: DVec3| v - up * v.dot(up);
        let mut nose_h = flat(ship.orient * DVec3::NEG_Z);
        if nose_h.length_squared() < 1e-12 {
            nose_h = flat(ship.orient * DVec3::Y);
        }
        let nose_h = nose_h.normalize_or_zero();
        (nose_h, nose_h.cross(up), up)
    }

    /// The WIND line: speed and the way the air goes, relative to the
    /// nose. None in vacuum (the field is exactly zero there) or a calm.
    fn wind_readout(&self) -> Option<(f32, &'static str)> {
        let w = self.wind_now();
        let speed = w.length();
        if speed < 0.5 {
            return None;
        }
        let (nose_h, right_h, _) = self.horizontal_frame();
        let ang = w.dot(right_h).atan2(w.dot(nose_h));
        Some((speed as f32, readout::arrow(ang as f32)))
    }

    fn debris_uniforms(&self, pose: &ViewPose) -> DebrisUniforms {
        let cam = &pose.cam;
        let head = pose.head;
        if self.arms.shards.is_empty() {
            return DebrisUniforms::none(cam, head);
        }
        let ship_inv = self.state.ship.orient.inverse();
        let ship_pos = self.state.ship.pos_m;
        let t = self.state.time_s;
        let to_ship = |p: DVec3| (ship_inv * (p - ship_pos)).as_vec3();
        let shards: Vec<ShardView> = self
            .arms
            .shards
            .iter()
            .map(|sh| {
                let age = (t - sh.born_s).max(0.0);
                ShardView {
                    at: to_ship(sh.pos),
                    size: sh.size,
                    axis: (ship_inv * sh.axis).as_vec3(),
                    angle: (sh.spin * age) as f32,
                    age01: (age / sh.life_s.max(0.01) as f64) as f32,
                    seed: sh.seed,
                }
            })
            .collect();
        let rocks: Vec<farfall_render::debris::Occluder> = self
            .belt
            .rocks
            .iter()
            .map(|r| farfall_render::debris::Occluder {
                centre: to_ship(r.pos),
                radius_m: r.radius_m as f32,
            })
            .collect();
        let sun_ship = (ship_inv * self.params.sun.dir).as_vec3();
        DebrisUniforms::new(
            cam,
            head,
            1.0,
            self.settings.arms_glow,
            sun_ship,
            &DebrisScene {
                shards: &shards,
                rocks: &rocks,
            },
        )
    }

    /// The after-image this frame, if one is still showing.
    fn ghost_uniforms(&self, pose: &ViewPose) -> GhostUniforms {
        let cam = &pose.cam;
        let head = pose.head;
        let Some(g) = self.ghost else {
            return GhostUniforms::none(cam, head);
        };
        let now = self.started.elapsed().as_secs_f32();
        let age = now - g.at_s;
        if !(0.0..GHOST_LIFE_S).contains(&age) {
            return GhostUniforms::none(cam, head);
        }
        let ship_inv = self.state.ship.orient.inverse();
        let dir_ship = (ship_inv * g.dir_world).as_vec3();
        let rot_rel = (ship_inv * g.orient).as_quat();
        GhostUniforms::new(cam, head, now, age, dir_ship, rot_rel, self.settings.shield)
            .with_eye(pose.eye_ship.as_vec3())
    }

    /// The ship itself, for an eye outside it; in the cockpit there is
    /// nothing to draw and the pass discards.
    fn jet_uniforms(&self, pose: &ViewPose) -> JetUniforms {
        let ship_inv = self.state.ship.orient.inverse();
        let sun_ship = (ship_inv * self.params.sun.dir).as_vec3();
        let thrust = self.thrust_look();
        let (body_dir, body_sin) = self.nearest_body_ship();
        let u = JetUniforms::new(
            &pose.cam,
            pose.head,
            pose.eye_ship.as_vec3(),
            sun_ship,
            thrust[0],
            self.hyper,
        )
        .with_rcs(thrust[1], thrust[2], thrust[3])
        .with_body_fill(body_dir, body_sin * body_sin)
        .with_fit(&bay::fit_views(&self.settings.mounts))
        .with_craft(self.settings.craft.kind());
        // In the cockpit there is nothing to draw; in a pad's helicopter
        // the heli pass draws the flown hull, not this one.
        if pose.eye_ship == DVec3::ZERO || self.helis.in_heli {
            u
        } else {
            u.shown()
        }
    }

    /// The holo3PP's miniature: the ship's neighbourhood in its own frame
    /// — the velocity relative to the nearest body, that body's bearing and
    /// angular size, the Sun's bearing, the engines.
    fn holo_uniforms(&self, pose: &ViewPose) -> HoloUniforms {
        let ship_inv = self.state.ship.orient.inverse();
        let t = self.state.time_s;
        let bodies = self.params.bodies(t);
        let vels = self.params.body_velocities(t);
        // The nearest body by altitude, as the altimeter picks it.
        let mut near: Option<(f64, DVec3, f64, DVec3)> = None;
        for (b, v) in bodies.iter().zip(vels) {
            let rel = b.centre - self.state.ship.pos_m;
            let alt = rel.length() - b.radius_m;
            if near.is_none_or(|n| alt < n.0) {
                near = Some((alt, rel, b.radius_m, v));
            }
        }
        let (body_dir, body_sin, vel_rel) = match near {
            Some((_, rel, r, v)) => (
                (ship_inv * rel).as_vec3(),
                (r / rel.length().max(1.0)).clamp(0.0, 1.0) as f32,
                self.state.ship.vel_mps - v,
            ),
            None => (Vec3::ZERO, 0.0, self.state.ship.vel_mps),
        };
        // The revealed mimics, relative to the ship in its frame: a mark
        // each at its true bearing.
        let mut marks = [None; farfall_render::holo::MARKS];
        for (slot, m) in marks
            .iter_mut()
            .zip(self.mimics.ships.iter().filter(|m| !m.shrouded(t)))
        {
            *slot = Some((
                (ship_inv * (m.pos - self.state.ship.pos_m)).as_vec3(),
                m.kind(),
            ));
        }
        let scene = HoloScene {
            vel_dir: (ship_inv * vel_rel).as_vec3(),
            speed_mps: vel_rel.length() as f32,
            body_dir,
            body_sin,
            sun_dir: (ship_inv * self.params.sun.dir).as_vec3(),
            effort: self.effort,
            hyper: self.hyper,
            range: self.settings.holo_range,
            marks,
            craft: self.settings.craft.kind(),
        };
        let tan_half = (pose.cam.fov_y * 0.5).tan();
        let radius = self.settings.holo_size * HOLO_RADIUS_M;
        let centre = holo_centre(self.settings.holo_anchor, tan_half, pose.cam.aspect, radius);
        HoloUniforms::new(
            &pose.cam,
            pose.head,
            centre,
            radius,
            &scene,
            self.holo_active(),
        )
    }

    /// Where a slipped drive drops the ship: some body, at a random
    /// distance of a few radii, going some way at a speed around circular
    /// — an orbit, but not one that stays. Attitude kept, spin killed.
    fn slip_jump(&mut self) {
        let t = self.state.time_s;
        let bodies = self.params.bodies(t);
        let vels = self.params.body_velocities(t);
        // Weighted: the near bodies more often than the Sun.
        let pick = self.next_unit();
        let i = if pick < 0.35 {
            0
        } else if pick < 0.62 {
            1
        } else if pick < 0.74 {
            2
        } else {
            3
        };
        let b = bodies[i];
        let (u1, u2, u3, u4, u5) = (
            self.next_unit(),
            self.next_unit(),
            self.next_unit(),
            self.next_unit(),
            self.next_unit(),
        );
        let dir = random_unit(u1, u2);
        let r = b.radius_m * (1.8 + 4.2 * u3 as f64);
        let pos = b.centre + dir * r;
        let v_circ = (b.mu / r).sqrt();
        let tangent = {
            let any = random_unit(u4, u5);
            let tnt = any - dir * any.dot(dir);
            if tnt.length() > 1e-6 {
                tnt.normalize()
            } else {
                dir.cross(DVec3::Y).normalize_or_zero()
            }
        };
        let lean = (self.next_unit() as f64 - 0.5) * 1.0;
        let gain = 0.5 + 0.9 * self.next_unit() as f64;
        let vel = vels[i] + (tangent * lean.cos() + dir * lean.sin()) * v_circ * gain;
        self.state.ship.pos_m = pos;
        self.state.ship.vel_mps = vel;
        self.state.ship.ang_vel_radps = DVec3::ZERO;
        self.jumps = self.jumps.wrapping_add(1);
        self.touchdown = None;
        log::warn!(
            "hyper: slipped to body {i} at {:.1} radii, {:.0} m/s ({:.2} of circular)",
            r / b.radius_m,
            (vel - vels[i]).length(),
            gain
        );
    }

    /// The jump itself, at the flip's peak: the ship is placed at the
    /// plan's arrival, attitude kept, and the world carries on from there.
    fn jump(&mut self) {
        let (pos, vel) =
            self.settings
                .plan
                .arrival(&self.params, &self.state.ship, self.state.time_s);
        self.state.ship.pos_m = pos;
        self.state.ship.vel_mps = vel;
        self.state.ship.ang_vel_radps = DVec3::ZERO;
        self.jumps = self.jumps.wrapping_add(1);
        let centre = self
            .settings
            .plan
            .dest
            .centre(&self.params, self.state.time_s);
        log::info!(
            "warp: arrived at {} — {:.0} km out, {:.0} m/s",
            self.settings.plan.dest.name(),
            (pos - centre).length() / 1000.0,
            vel.length()
        );
    }

    /// Advance the sim by wall time, in whole fixed steps (SPEC §7.2).
    fn tick(&mut self) {
        let now = Instant::now();
        let mut frame_dt = now.duration_since(self.last_frame).as_secs_f64();
        self.last_frame = now;
        self.frame_dt = frame_dt.min(0.25) as f32;

        // Instruments are presentation, not physics: the velocity hologram
        // keeps living even when the sim is frozen for a benchmark —
        // otherwise every perf capture shows an empty cockpit.
        self.gauge_fade.update(
            frame_dt.min(0.25) as f32,
            self.state.ship.vel_mps.length() as f32,
        );
        let (altitude, vspeed) = self.altitude_vspeed();
        self.alt_fade
            .update(frame_dt.min(0.25) as f32, altitude as f32, vspeed as f32);
        // Hologram inertia from body rotation rates, and the barrier flash
        // from the same supersonic edge that fires the audio boom.
        let w_body = self.state.ship.orient.conjugate() * self.state.ship.ang_vel_radps;
        self.holo_sway
            .update(frame_dt.min(0.25) as f32, w_body.x as f32, w_body.y as f32);
        self.mach_alert
            .update(frame_dt.min(0.25) as f32, self.is_supersonic());
        let target = if self.settings.layout.shown(Instrument::Trajectory) {
            1.0
        } else {
            0.0
        };
        let k = 1.0 - (-(frame_dt.min(0.25) as f32) / 0.18).exp();
        self.trajectory_vis += (target - self.trajectory_vis) * k;
        self.horizon_fade
            .update(frame_dt.min(0.25) as f32, altitude as f32);
        self.g_fade.update(frame_dt.min(0.25) as f32, self.felt_g);
        // The hyper field forms over a moment and collapses faster.
        {
            let want = if self.bench_hyper
                || (self.input.controls(self.assist).hyper
                    && !self.warp.active()
                    && !self.helis.in_heli)
            {
                1.0
            } else {
                0.0
            };
            let tau = if want > self.hyper { 0.6 } else { 0.25 };
            let k = 1.0 - (-(frame_dt.min(0.25) as f32) / tau).exp();
            self.hyper += (want - self.hyper) * k;
        }
        self.look.update(frame_dt.min(0.25) as f32);
        // The WARP LENGTH setting reaches the drive here; a running
        // sequence keeps the length it started with.
        self.warp.set_length(self.settings.warp_length);
        if self.warp.update(frame_dt.min(0.25) as f32) {
            if self.pending_slip {
                self.pending_slip = false;
                self.slip_jump();
            } else {
                self.jump();
            }
        }
        self.run_hyper_strain(frame_dt.min(0.25) as f32);

        // A frozen bench still shows the belt: the rocks come live, unmoved.
        if self.frozen {
            self.belt.step(
                &self.params,
                self.state.time_s,
                0.0,
                self.state.ship.pos_m,
                self.state.ship.vel_mps,
            );
        }
        // A pilot reading a panel is not flying: the world waits.
        if self.panel_open() {
            self.accumulator = 0.0;
            return;
        }

        // The camera on the head: pushed by the load, trembling under
        // thrust, settling — or parked, for a bench.
        self.shake.strength = self.settings.cam_shake;
        // The chaos drive jostles the SHIP; the helmet camera is held
        // while the field is up so the dash reads to the slip.
        self.shake.hyper_damp(self.hyper);
        self.shake
            .step(self.frame_dt, self.felt_g_body, self.effort);
        if let Some([y, p, r]) = self.bench_shake {
            self.shake.park(y, p, r);
        }

        if self.frozen {
            return;
        }
        // Death-spiral guard: never simulate more than 0.25 s per frame.
        frame_dt = frame_dt.min(0.25);
        self.accumulator += frame_dt;
        // Controls are sampled once per frame, not per step: every fixed step in
        // this frame sees the same input, which is what a networked client would
        // send upstream (SPEC §5.2).
        // Advance the input ramp on wall time, before sampling it.
        self.input.update(frame_dt);
        // Through a jump the stick is dead: the drive has the ship. On
        // foot every control is dead: the ship stays LANDED where its
        // pilot left it, and nothing can lift it off from under them.
        let mut controls = if self.warp.active() || self.eva.is_some() {
            sim::Controls {
                assist: self.assist,
                ..Default::default()
            }
        } else {
            self.input.controls(self.assist)
        };
        controls.hyper_level = self.hyper_level();
        // The CRAFT row is live: outside a pad's helicopter the sim flies
        // whatever airframe the SHIP page chose (SPEC §6.5c).
        if !self.helis.in_heli {
            self.params.ship = self.own_ship_params();
        }
        // In the helicopter the stick is the helicopter's: collective,
        // cyclic, pedals - and a pad hull's drives cannot come, while
        // the pilot's own FARFALL helicopter keeps its hyper field.
        if self.helis.in_heli {
            controls = heli::route_controls(controls);
        } else if self.settings.craft == bay::Craft::Helicopter {
            controls = heli::route_controls_farfall(controls);
        }

        // Fuelled by entropy: toward the slip the field shakes the ship —
        // the stick and the throttle jostled by the drive, a little, then
        // a lot, until it goes.
        let chaos = if controls.hyper {
            chaos_level(self.hyper_strain, self.slip_at) * self.settings.drive_shake.clamp(0.0, 2.0)
        } else {
            0.0
        };
        let base = controls;
        while self.accumulator >= sim::DT {
            if chaos > 0.0 {
                let j = |g: &mut Game| (g.next_unit() * 2.0 - 1.0) as f64;
                let (a, b, c) = (j(self), j(self), j(self));
                let (d, e, f) = (j(self), j(self), j(self));
                let k = chaos as f64;
                controls = base;
                controls.torque_body += DVec3::new(a, b, c) * 0.9 * k;
                controls.thrust_body += DVec3::new(d, e, f) * 0.5 * k;
                controls.torque_body = controls
                    .torque_body
                    .clamp(DVec3::splat(-1.0), DVec3::splat(1.0));
                controls.thrust_body = controls
                    .thrust_body
                    .clamp(DVec3::splat(-1.0), DVec3::splat(1.0));
            }
            let spacing = self.mark_spacing_m() as f64;
            let before = (self.odometer_m / spacing).floor();
            self.odometer_m += self.state.ship.vel_mps.length() * sim::DT;
            let after = (self.odometer_m / spacing).floor();
            // A hoop is a thing on the glass: unseen, or unwanted, it makes
            // no sound.
            let audible = self.trajectory_vis > 0.5
                && self.settings.layout.shown(Instrument::Hoops)
                && self.settings.layout.shown(Instrument::HoopSound);
            if after > before && audible {
                self.hoops_passed = self.hoops_passed.wrapping_add(1);
            }
            self.roll_for_strikes();
            let before = self.state.ship;
            let before_t = self.state.time_s;
            // HOLD: the computer's thrust replaces the pilot's on the way
            // in, its torque adds; the lock drops when the target is gone.
            let mut step_controls = controls;
            // LANDING ASSIST: on the way in, the flight computer holds the
            // hull level over the ground on any axis the pilot leaves alone.
            if self.landing
                && self.settings.landing_assist
                && self.touchdown.is_some()
                && !self.on_ground()
            {
                landing::assist(
                    &mut step_controls,
                    self.state.ship.orient,
                    self.state.ship.ang_vel_radps,
                    self.up_world(),
                );
            }
            if self.hold.engaged() {
                self.hold.gain = self.settings.hold_gain;
                self.hold.face = self.settings.hold_face;
                match self.hold.track(&self.belt, &self.mimics, &self.miners) {
                    Some(tracked) => {
                        let own = arms::Ship {
                            pos: self.state.ship.pos_m,
                            vel: self.state.ship.vel_mps,
                            orient: self.state.ship.orient,
                            aim: self.aim_world(),
                        };
                        self.hold.apply(
                            &mut step_controls,
                            &own,
                            self.state.ship.ang_vel_radps,
                            &tracked,
                            self.params.ship.max_thrust_mps2,
                            sim::DT,
                        );
                    }
                    None => {
                        self.hold.release();
                        self.mimics.line = Some(("HOLD LOST".to_string(), self.state.time_s + 2.0));
                        log::info!("hold: the target is gone");
                    }
                }
            }
            self.state = sim::step(&self.params, &self.state, step_controls);
            // The tick the ground is met is the one that says how the
            // touchdown went; the record stands until the next one.
            if let Some(record) =
                landing::Record::judge(&self.params, before_t, &before, &self.state.ship)
            {
                self.touchdown_record = Some(record);
                log::info!(
                    "touchdown: {} on {} — {:.1} m/s down, {:.1} m/s along",
                    record.verdict(),
                    landing::BODY_NAMES[record.body],
                    record.into_mps,
                    record.along_mps
                );
            }
            // The belt: rocks move, knock each other, and knock the ship
            // — an impulse on its state, a bump on the shield, grit in
            // the sound.
            let shove = self.belt.step(
                &self.params,
                self.state.time_s,
                sim::DT,
                self.state.ship.pos_m,
                self.state.ship.vel_mps,
            );
            if shove != DVec3::ZERO {
                self.state.ship.vel_mps += shove;
            }
            // The arms: slugs fly, land on rocks, and the guns kick.
            self.arms.power = self.settings.arms_power;
            self.arms.shards_per_break = self.settings.arms_shards;
            self.arms.shard_life_s = self.settings.arms_shard_life;
            self.arms.scar_size = self.settings.arms_scar_size;
            self.arms.scar_cool_s = self.settings.arms_scar_cool;
            self.arms.mounts = self.settings.mounts;
            self.mimics.chance = self.settings.mimics_chance;
            self.mimics.hostility = self.settings.mimics_hostility;
            self.mimics.size = self.settings.mimics_size;
            self.haul.yield_ = self.settings.arms_ore;
            self.miners.count = self.settings.miners_count;
            self.miners.growth = self.settings.miners_growth;
            let trigger = (self.fire_held || self.stick_fire)
                && !self.menu.open
                && !self.map_open()
                && !self.design;
            let kick = self.arms.step(
                self.state.time_s,
                sim::DT,
                &arms::Ship {
                    pos: self.state.ship.pos_m,
                    vel: self.state.ship.vel_mps,
                    orient: self.state.ship.orient,
                    aim: self.aim_world(),
                },
                trigger,
                &mut self.belt,
            );
            if kick != DVec3::ZERO {
                self.shake.kick(kick.length() as f32, self.arms.last_side);
                self.state.ship.vel_mps += kick;
            }
            self.step_mimics();
            self.step_eva();
            let hits: Vec<belt::Hit> = self.belt.hits.clone();
            for h in hits {
                let ship_inv = self.state.ship.orient.inverse();
                let dir = (ship_inv * h.from).as_vec3().normalize_or_zero();
                let size = ((h.radius_m / 60.0).sqrt() as f32).clamp(0.35, 1.0);
                self.strikes = self.strikes.wrapping_add(1);
                self.strike_size = size;
                self.impacts.insert(
                    0,
                    Impact {
                        dir,
                        at_s: self.started.elapsed().as_secs_f32(),
                        size,
                    },
                );
                self.impacts.truncate(farfall_render::shield::IMPACTS);
                log::info!(
                    "belt: a {:.0} m rock at {:.0} m/s",
                    h.radius_m * 2.0,
                    h.closing_mps
                );
            }
            let felt = sim::felt_acceleration(&self.params, before_t, &before, &self.state.ship);
            self.felt_g = (felt.length() / 9.81) as f32;
            // In the ship's frame: x right, y up, -z the nose — shown as
            // right, up, forward.
            let body = self.state.ship.orient.inverse() * felt / 9.81;
            self.felt_g_body = [body.x as f32, body.y as f32, -body.z as f32];
            self.accumulator -= sim::DT;
        }

        if self.landing {
            self.touchdown = landing::predict(&self.params, &self.state.ship, self.state.time_s);
        }

        // Camera response to the ship's own physics. Exponential smoothing,
        // framerate-independent: at 30 fps and at 240 fps the view opens up over
        // the same wall-clock time, so the ship's weight is a property of the
        // ship and not of the machine.
        let target = self
            .input
            .thrust_effort(self.params.ship.boost_multiplier)
            .max(self.hold_effort()) as f32;
        let alpha = 1.0 - (-(frame_dt as f32) / FOV_RESPONSE_S).exp();
        self.effort += (target - self.effort) * alpha;

        // Autosave: every 30 s of SIM time (not wall clock), so a crash
        // costs at most that much. Never reached while a panel is open or
        // the sim is frozen — both return earlier in this function, which
        // is exactly when this should not run anyway.
        if self.state.time_s >= self.next_save_s {
            self.next_save_s = self.state.time_s + AUTOSAVE_INTERVAL_S;
            self.maybe_store_world();
        }
    }

    /// How much atmosphere surrounds the hull, 0 (vacuum) to 1 (thick air).
    /// The single definition shared by the audio levels, the mach gate and
    /// the instruments — one border, agreed on by ear and eye.
    fn air_ratio(&self) -> f64 {
        let r = self.state.ship.pos_m.length();
        let rho = sim::atmo_density(&self.params.planet, r);
        (rho / self.params.planet.atmo_rho0 * 12.0).clamp(0.0, 1.0)
    }

    /// Supersonic IN the atmosphere — mach in vacuum is meaningless. The
    /// rising edge of this one expression fires both the sonic boom and the
    /// HUD's barrier flash, so they cannot drift apart.
    fn is_supersonic(&self) -> bool {
        self.air_ratio() > 0.65 && self.state.ship.vel_mps.length() > MACH1_MPS
    }

    /// Mach number for the instrument, or a negative number outside the
    /// atmosphere: the gauge hides a meaningless reading entirely.
    fn mach(&self) -> f32 {
        if self.air_ratio() > 0.10 {
            (self.state.ship.vel_mps.length() / MACH1_MPS) as f32
        } else {
            -1.0
        }
    }

    /// Altitude above the sphere and radial (climb) velocity, m and m/s.
    /// Altitude and vertical speed over the NEAREST body's surface —
    /// whichever ground is closest is the one that matters.
    fn altitude_vspeed(&self) -> (f64, f64) {
        let bodies = self.params.bodies(self.state.time_s);
        let vels = self.params.body_velocities(self.state.time_s);
        let mut best = (f64::INFINITY, 0.0);
        for (b, v) in bodies.iter().zip(vels) {
            let rel = self.state.ship.pos_m - b.centre;
            let r = rel.length();
            let alt = r - b.radius_m;
            if alt < best.0 {
                let up = rel / r.max(1.0);
                best = (alt, (self.state.ship.vel_mps - v).dot(up));
            }
        }
        best
    }

    /// Planet as the camera sees it. The world-space subtraction happens here,
    /// in f64, and only the *relative* offset is narrowed to f32 — which is the
    /// whole floating-origin discipline in one line (SPEC P3).
    fn planet_uniforms(&self, pose: &ViewPose) -> PlanetUniforms {
        let cam = &pose.cam;
        let eye = self.eye_m(pose);
        let centre_rel = (DVec3::ZERO - eye).as_vec3();
        let [_, moon, sun, _] = self.params.bodies(self.state.time_s);
        let rel = |b: &sim::Body| ((b.centre - eye).as_vec3(), b.radius_m as f32);
        PlanetUniforms::new(
            cam,
            centre_rel,
            self.params.planet.radius_m as f32,
            self.params.sun.dir.as_vec3(),
            &self.appearance,
            // Weather advances on sim time, so the sky is a function of the
            // world's clock rather than of how long the window has been open.
            self.state.time_s as f32 * 0.05,
        )
        .with_occluders([rel(&moon), rel(&sun)])
        .with_sky(self.settings.sky)
        .with_detail(
            self.settings.terrain_detail,
            self.settings.clouds,
            self.settings.city_lights,
        )
    }

    /// What the hull feels this frame: the wind in its own frame and the air
    /// it is cutting through. The heating itself is computed on the GPU
    /// (render::thermal); the CPU hands over physics and never a temperature.
    fn thermal_inputs(&self, dt: f32) -> ThermalInputs {
        let ship = &self.state.ship;
        let r = ship.pos_m.length();
        ThermalInputs {
            vel_ship_mps: farfall_render::thermal::ship_frame_velocity(
                ship.orient.as_quat(),
                ship.vel_mps.as_vec3(),
            ),
            rho: sim::atmo_density(&self.params.planet, r) as f32,
            rho0: self.params.planet.atmo_rho0 as f32,
            dt,
            reset: false,
        }
    }

    /// The world as the path predictor needs it: the laws and the state,
    /// camera-relative. Nose-on drag: the prediction assumes the pilot flies
    /// prograde, which is what a prediction is for.
    fn trajectory_world(&self, eye_m: DVec3) -> TrajectoryWorld {
        let planet = &self.params.planet;
        let ship = &self.state.ship;
        TrajectoryWorld {
            centre_rel: (DVec3::ZERO - eye_m).as_vec3(),
            radius_m: planet.radius_m as f32,
            mu: planet.mu as f32,
            rho0: planet.atmo_rho0 as f32,
            scale_height_m: planet.atmo_scale_height_m as f32,
            atmo_top_m: planet.atmo_top_m as f32,
            vel_world: ship.vel_mps.as_vec3(),
            cda_over_m: (self.params.ship.cd_area_m2 / self.params.ship.mass_kg) as f32,
        }
    }

    /// Step through the atmosphere presets. A stand-in for the settings panel:
    /// an alien world is not new code, it is different numbers.
    fn cycle_appearance(&mut self) {
        self.appearance_index = (self.appearance_index + 1) % PlanetAppearance::PRESETS.len();
        self.appearance = PlanetAppearance::PRESETS[self.appearance_index];
        log::info!(
            "atmosphere: {} (density {:.2}, cloud cover {:.0}%, deck {:.0} m)",
            self.appearance.name,
            self.appearance.atmosphere_density,
            self.appearance.cloud_coverage * 100.0,
            self.appearance.cloud_altitude_m,
        );
    }

    /// The ship's voice, from the sim's own physics (SPEC P2, in sound):
    /// wind is the actual dynamic pressure ½ρv², vacuum is the actual air
    /// density at altitude, load is the actual felt acceleration. Nothing
    /// here is a sound "event" — the audio is a continuous function of the
    /// world's state, so it cannot desync from what the pilot sees.
    fn audio_levels(&self) -> farfall_audio::Levels {
        let ship = &self.state.ship;
        let planet = &self.params.planet;
        let r = ship.pos_m.length();
        let rho = sim::atmo_density(planet, r);
        let rho_ratio = rho / planet.atmo_rho0;
        let speed = ship.vel_mps.length();

        // Dynamic pressure against a "full roar" reference: sea level at
        // 300 m/s pins the top of the wind's range.
        let q = 0.5 * rho * speed * speed;
        let q_ref = 0.5 * planet.atmo_rho0 * 300.0 * 300.0;

        let controls = self.input.controls(self.assist);

        // Entry intensity: the BUILD-UP of arrival. It needs three things at
        // once — thin air starting to bite (a wide, early-onset density ramp,
        // so the crackle grows through the descent the way the wind does
        // later), not yet full atmosphere (once the ship is properly inside,
        // the wind takes over and this collapses — which is the falling edge
        // the boom fires on), and actually diving at speed: sitting in orbit
        // over the same altitude is silent, and mach matters — a gentle sink
        // whispers, a fast plunge crackles like torn air.
        let air = self.air_ratio();
        let up = ship.pos_m / r.max(1.0);
        let dive = (-ship.vel_mps.dot(up) / 120.0).clamp(0.0, 1.0);
        let air_wide = (rho_ratio * 90.0).clamp(0.0, 1.0);
        let mach_bite = ((speed - 120.0) / 300.0).clamp(0.0, 1.0);
        let entry = (air_wide * (1.0 - air) * dive * mach_bite).clamp(0.0, 1.0);

        farfall_audio::Levels {
            effort: self
                .input
                .thrust_effort(self.params.ship.boost_multiplier)
                .max(self.hold_effort()) as f32,
            wind_q: ((q / q_ref) as f32).clamp(0.0, 1.0),
            vacuum: 1.0 - ((rho_ratio * 12.0) as f32).clamp(0.0, 1.0),
            brake: if controls.brake { 1.0 } else { 0.0 },
            // Attitude thrusters: the largest torque demand. Rolling is
            // flying, and a silent manoeuvre reads as a broken game.
            rcs: controls.torque_body.abs().max_element() as f32,
            entry: entry as f32,
            supersonic: if self.is_supersonic() { 1.0 } else { 0.0 },
            hoops: self.hoops_passed as f32,
            warp: self.warp_look().charge,
            jumps: self.jumps as f32,
            master: 0.8,
            stress: if self.settings.hull_sound {
                hull_stress(speed)
            } else {
                0.0
            },
            strikes: if self.settings.hull_sound {
                self.strikes as f32
            } else {
                0.0
            },
            strike_size: self.strike_size,
            shots: self.arms.shots as f32,
            shot_kind: self.arms.shot_kind as f32,
            bangs: self.arms.bangs as f32,
            bang_size: self.arms.bang_size,
            rail: if self.arms.selected == arms::Weapon::Rail {
                self.arms.charge
            } else {
                0.0
            },
            reveals: self.mimics.reveals as f32,
            hails: self.mimics.hails as f32,
        }
    }

    /// One sim step's chance of a grain of rock on the hull: space is not
    /// empty, and the faster the ship sweeps through it the more it meets —
    /// a Poisson process at the rate [`strike_rate_hz`] gives. The dice are
    /// the app's, not the sim's: a strike is a sound, not a force.
    fn roll_for_strikes(&mut self) {
        if !self.settings.hull_sound {
            return;
        }
        let speed = self.state.ship.vel_mps.length() as f32;
        let rate = strike_rate_hz(speed, self.air_ratio() as f32);
        let (u1, u2) = (self.next_unit(), self.next_unit());
        if u1 < rate * sim::DT as f32 {
            self.strikes = self.strikes.wrapping_add(1);
            self.strike_size = strike_size_from(u2);
            // Where it hit the shell: anywhere at a crawl; the faster the
            // ship, the more from ahead of the motion (ship frame).
            let ship_inv = self.state.ship.orient.inverse();
            let ahead = (ship_inv * self.state.ship.vel_mps.normalize_or_zero()).as_vec3();
            let scatter = random_unit(self.next_unit(), self.next_unit()).as_vec3();
            let bias = (speed / 2_000.0).clamp(0.0, 0.85);
            let dir = (ahead * bias + scatter * (1.0 - bias)).normalize_or_zero();
            self.impacts.insert(
                0,
                Impact {
                    dir,
                    at_s: self.started.elapsed().as_secs_f32(),
                    size: self.strike_size,
                },
            );
            self.impacts.truncate(farfall_render::shield::IMPACTS);
        }
    }

    fn next_unit(&mut self) -> f32 {
        let mut x = self.strike_rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.strike_rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Log why and where the session ended. Every exit path goes through this:
    /// a silent exit and a deliberate quit are indistinguishable in a log, and
    /// telling them apart is the difference between "the pilot stopped" and
    /// "the event loop died".
    fn log_exit(&self, reason: &str) {
        let ship = &self.state.ship;
        log::info!(
            "exit ({reason}): sim t={:.1}s alt={:.0}m speed={:.0}m/s spin={:.3}rad/s \
             assist={} hash={:#018x}",
            self.state.time_s,
            ship.pos_m.length() - self.params.planet.radius_m,
            ship.vel_mps.length(),
            ship.ang_vel_radps.length(),
            if self.assist { "on" } else { "off" },
            sim::state_hash(&self.state),
        );
        self.maybe_store_world();
    }

    /// Camera pose for this frame: ride the hull, look down the nose. The view
    /// is the ship's orientation, so steering turns the world rather than
    /// sliding a detached camera around it.
    /// Left button while looking: pick up the dial under the gaze, if one
    /// is within reach. Returns whether something was picked up.
    fn begin_drag(&mut self, cam: &CameraFrame, px: f32) -> bool {
        // The glass is laid out in the reference projection; the pointer
        // is the cursor in design mode, the gaze while looking.
        let Some(gaze) = self.pointer(cam) else {
            return false;
        };
        if self.card_open {
            return false;
        }
        // The block on screen: a flat card's width is over the aspect.
        let text_w = self.text_w(px) / cam.aspect;
        // A panel that is up is what the gaze can take: the map by its
        // pane, the settings by its text block.
        if self.map_open() {
            let [cx, cy, hw] = self.map_pane(cam.aspect);
            let hh = hw * cam.aspect;
            let inside = (gaze[0] - cx).abs() <= hw + 0.02 && (gaze[1] - cy).abs() <= hh + 0.02;
            let text = self.drive_text_anchor(cam.aspect, text_w);
            let on_text = gaze[0] >= text[0] - 0.02
                && gaze[0] <= text[0] + text_w + 0.02
                && gaze[1] <= text[1] + 0.02
                && gaze[1] >= text[1] - 0.5;
            if inside || on_text {
                let a = self.settings.map_anchor;
                self.drag = Some((Dragged::MapPanel, [a[0] - gaze[0], a[1] - gaze[1]]));
                log::info!("drag: picked up the map");
                return true;
            }
            return false;
        }
        if self.bay_open() {
            let [cx, cy, hw] = self.bay_pane();
            let hh = hw * cam.aspect;
            let inside = (gaze[0] - cx).abs() <= hw + 0.02 && (gaze[1] - cy).abs() <= hh + 0.02;
            let text = self.bay_text_anchor(cam.aspect, text_w);
            let on_text = gaze[0] >= text[0] - 0.02
                && gaze[0] <= text[0] + text_w + 0.02
                && gaze[1] <= text[1] + 0.02
                && gaze[1] >= text[1] - 0.5;
            if inside || on_text {
                let a = self.settings.bay_anchor;
                self.drag = Some((Dragged::BayPanel, [a[0] - gaze[0], a[1] - gaze[1]]));
                log::info!("drag: picked up the ship bay");
                return true;
            }
            return false;
        }
        if self.menu.open {
            let a = self.text_anchor(cam.aspect, px);
            let h = self.menu.extent().1 as f32 * px;
            let on_text = gaze[0] >= a[0] - 0.02
                && gaze[0] <= a[0] + text_w + 0.02
                && gaze[1] <= a[1] + 0.02
                && gaze[1] >= a[1] - h - 0.02;
            if on_text {
                self.drag = Some((Dragged::MenuPanel, [a[0] - gaze[0], a[1] - gaze[1]]));
                log::info!("drag: picked up the settings panel");
                return true;
            }
            return false;
        }
        // The holo3PP is glassware like the dials: take it by its
        // emitter's anchor and slide it along the dash.
        if self.holo_active() {
            let a = self.settings.holo_anchor;
            let r = self.settings.holo_size * 0.9;
            let inside = (gaze[0] - a[0]).abs() <= r && (gaze[1] - a[1]).abs() <= r;
            if inside {
                self.drag = Some((Dragged::HoloPanel, [a[0] - gaze[0], a[1] - gaze[1]]));
                log::info!("drag: picked up the hologram");
                return true;
            }
        }
        // The mini map is glassware too: taken by its pane, dropped
        // anywhere on the glass, its anchor kept on ui.map.
        if self.mini_map_shown() {
            let a = self.settings.layout.inset(self.mini_map_anchor());
            let [cx, cy, hw] = map::pane_rect_sized(cam.aspect, a, self.mini_map_half_h());
            let hh = hw * cam.aspect;
            let inside = (gaze[0] - cx).abs() <= hw + 0.02 && (gaze[1] - cy).abs() <= hh + 0.02;
            if inside {
                self.drag = Some((Dragged::MiniMap, [a[0] - gaze[0], a[1] - gaze[1]]));
                log::info!("drag: picked up the mini map");
                return true;
            }
        }
        // The readout's block is a glass element like a dial: the gaze
        // while looking, or the cursor in design mode, takes it too.
        {
            let a = self.settings.readout_anchor;
            let text_w = panel::block_ndc(PANEL_COLS, px);
            let on_text = gaze[0] >= a[0] - 0.02
                && gaze[0] <= a[0] + text_w + 0.02
                && gaze[1] <= a[1] + 0.02
                && gaze[1] >= a[1] - 0.45;
            if on_text {
                self.drag = Some((Dragged::Readout, [a[0] - gaze[0], a[1] - gaze[1]]));
                log::info!("drag: picked up the readout");
                return true;
            }
        }
        // The dials move only in DESIGN mode: looking round the cabin with
        // the button down is flying, and must not rearrange the dash.
        if !self.design {
            return false;
        }
        let layout = &self.settings.layout;
        let mut best: Option<(Instrument, f32, [f32; 2])> = None;
        for i in Instrument::ALL.iter().copied().filter(|i| i.slotted()) {
            if let Some(a) = layout.anchor(i) {
                let dx = (a[0] - gaze[0]) * cam.aspect;
                let dy = a[1] - gaze[1];
                let d = (dx * dx + dy * dy).sqrt();
                if d < DRAG_REACH && best.is_none_or(|b| d < b.1) {
                    best = Some((i, d, [a[0] - gaze[0], a[1] - gaze[1]]));
                }
            }
        }
        self.drag = best.map(|(i, _, off)| (Dragged::Dial(i), off));
        if let Some((d, _)) = self.drag {
            log::info!("drag: picked up {}", d.name());
        }
        self.drag.is_some()
    }

    /// Every frame while dragging: the dial follows the gaze, keeping the
    /// offset it was picked up with.
    fn update_drag(&mut self, cam: &CameraFrame) {
        let Some((i, off)) = self.drag else {
            return;
        };
        let Some(gaze) = self.pointer(cam) else {
            self.end_drag();
            return;
        };
        let at = [gaze[0] + off[0], gaze[1] + off[1]];
        // Panels stay on the screen; the readout is glass and may go anywhere
        // a dial may.
        let clamp = |a: [f32; 2]| [a[0].clamp(-0.95, 0.95), a[1].clamp(-0.95, 0.95)];
        let glass = |a: [f32; 2]| {
            [
                a[0].clamp(-cockpit::FREE_LIMIT, cockpit::FREE_LIMIT),
                a[1].clamp(-cockpit::FREE_LIMIT, cockpit::FREE_LIMIT),
            ]
        };
        match i {
            Dragged::Dial(i) => {
                let at = self.settings.layout.uninset(at);
                self.settings.layout.set_free(i, at);
            }
            Dragged::MenuPanel => self.settings.menu_anchor = clamp(at),
            Dragged::Readout => self.settings.readout_anchor = glass(at),
            Dragged::MapPanel => self.settings.map_anchor = clamp(at),
            Dragged::MiniMap => {
                let at = self.settings.layout.uninset(clamp(at));
                self.settings.layout.set_free(Instrument::Map, at);
            }
            Dragged::HoloPanel => self.settings.holo_anchor = clamp(at),
            Dragged::BayPanel => self.settings.bay_anchor = clamp(at),
        }
    }

    /// Drop it where it is, and keep that.
    fn end_drag(&mut self) {
        if let Some((d, _)) = self.drag.take() {
            self.settings.save();
            log::info!("drag: dropped {}", d.name());
        }
    }

    fn camera(&self, aspect: f32) -> CameraFrame {
        self.pose(aspect).cam
    }

    /// This frame's eye on the world: the walker's when someone is on
    /// foot, else the cockpit's, or the chase rig's when the CAMERA
    /// setting says so.
    fn pose(&self, aspect: f32) -> ViewPose {
        let mut pose = if let Some(w) = &self.eva {
            self.eva_pose(aspect, w)
        } else if self.chase_active() {
            self.chase_pose(aspect)
        } else {
            ViewPose {
                cam: self.cam_for(aspect, self.head()),
                head: self.head(),
                eye_ship: DVec3::ZERO,
            }
        };
        // A headset's fov is the runtime's alone — never settings.fov,
        // FARFALL_FOV, the thrust flare, a warp's fov_scale, or the
        // 2.9-radian clamp cam_for applies for every other view: a
        // changing fov in a headset is instant sickness. Every pose
        // variant gets this, not only the cockpit's — chase and EVA in
        // VR must see through the real eye too, or switching to either
        // would silently drop back to a flat, settings-driven fov.
        // eye.pos is added, not assigned: it is the eye's own small
        // offset from wherever this pose's own seat already is (zero in
        // the cockpit, CHASE_EYE_SHIP in chase, the walker's own eye on
        // foot), giving every pose its correct stereo parallax rather
        // than only the cockpit's.
        if let Some(vr) = &self.vr {
            let eye = vr.eyes[self.vr_eye.min(1)];
            let (fov_y, vr_aspect) = eye.symmetric();
            pose.cam.fov_y = fov_y;
            pose.cam.aspect = vr_aspect;
            pose.eye_ship += eye.pos.as_dvec3();
        }
        pose
    }

    /// The chase rig: an eye a few lengths back and a little above, pitched
    /// down so the ship centres. The pilot's freelook still turns it — a
    /// look around the ship, not out of the canopy.
    fn chase_pose(&self, aspect: f32) -> ViewPose {
        let head = self.head() * Quat::from_rotation_x(-CHASE_PITCH_RAD);
        ViewPose {
            cam: self.cam_for(aspect, head),
            head,
            eye_ship: CHASE_EYE_SHIP,
        }
    }

    /// The walker's eye (SPEC §6.5b): feet plus eye height, gaze leaning
    /// with the planet. Expressed in the ship's frame like every pose —
    /// exact while the ship is LANDED and still, which on foot it always
    /// is.
    fn eva_pose(&self, aspect: f32, w: &eva::Walker) -> ViewPose {
        let b = self.params.bodies(self.state.time_s)[w.body];
        let inv = self.state.ship.orient.inverse();
        let head = (inv * w.orientation()).as_quat();
        ViewPose {
            cam: self.cam_for(aspect, head),
            head,
            eye_ship: inv * (b.centre + w.eye_m() - self.state.ship.pos_m),
        }
    }

    /// The camera frame for a view turned `head` from the ship's own axes.
    /// The camera is the ship's orientation — both use the same
    /// right-handed frame with the nose at -Z, so no fix-up rotation is
    /// needed — times the view's turn. The head is a view, not a control:
    /// nothing downstream of the sim sees it.
    fn cam_for(&self, aspect: f32, head: Quat) -> CameraFrame {
        CameraFrame {
            orient: self.state.ship.orient.as_quat() * head,
            fov_y: ((self.settings.fov + FOV_THRUST_GAIN * self.effort)
                * self.warp_look().fov_scale)
                .to_radians()
                .min(2.9),
            aspect,
            time_s: self.started.elapsed().as_secs_f32(),
            exposure: 1.6,
        }
    }

    fn chase_active(&self) -> bool {
        self.settings.camera_chase
    }

    /// The eye is outside the ship — the chase rig's, or the walker's —
    /// so the hull shows and the cabin, dash and glass stay home.
    fn exterior_view(&self) -> bool {
        self.chase_active() || self.eva.is_some()
    }

    /// The cursor is grabbed while a look is driving it: the cockpit's
    /// freelook, or the walker's always-on gaze (freed while a panel is
    /// up, so the menu can be clicked).
    fn grabs_cursor(&self) -> bool {
        (self.eva.is_some() && !self.panel_open()) || self.look.engaged()
    }

    /// The key a named control answers to right now.
    fn bind(&self, n: Named) -> KeyCode {
        self.settings.bindings.named(n)
    }

    /// The holo3PP renders only when its panel is up and the main view is
    /// still the cockpit — in chase the whole screen already is the rig.
    fn holo_active(&self) -> bool {
        self.settings.holo_view && !self.exterior_view()
    }

    /// Where a pose's eye sits in the world, metres.
    fn eye_m(&self, pose: &ViewPose) -> DVec3 {
        self.state.ship.pos_m + self.state.ship.orient * pose.eye_ship
    }
}

impl Game {
    /// The helicopters this frame: the pads within draw range (hulls
    /// idling on their painted circles), any set down elsewhere, and -
    /// seen from the chase rig - the one being flown, its rotor by the
    /// collective.
    fn heli_uniforms(&self, pose: &ViewPose) -> HeliUniforms {
        let cam = &pose.cam;
        let head = pose.head;
        if !self.settings.helis {
            return HeliUniforms::none(cam, head);
        }
        let ship_inv = self.state.ship.orient.inverse();
        let eye = self.eye_m(pose);
        let to_ship = |p: DVec3| (ship_inv * (p - eye)).as_vec3();
        let now = self.started.elapsed().as_secs_f32();
        let mut views: Vec<(f64, HeliView)> = Vec::new();
        for i in 0..heli::PADS {
            let Some(st) = self.helis.heli_state(&self.params, i) else {
                continue;
            };
            let d = (st.pos_m - eye).length();
            if d > heli::DRAW_M {
                continue;
            }
            // A helicopter set down off its pad rests in a bare field.
            let on_pad = self.helis.displaced.iter().all(|(p, _)| *p != i);
            views.push((
                d,
                HeliView {
                    at: to_ship(st.pos_m),
                    rot: (ship_inv * st.orient).as_quat(),
                    rotor: 0.12,
                    seed: (i as f32 * 0.173).fract(),
                    pad: on_pad,
                },
            ));
        }
        if self.helis.in_heli && pose.eye_ship != DVec3::ZERO {
            views.push((
                0.0,
                HeliView {
                    at: to_ship(self.state.ship.pos_m),
                    rot: (ship_inv * self.state.ship.orient).as_quat(),
                    rotor: (0.35 + 0.65 * self.effort).clamp(0.0, 1.0),
                    seed: 0.5,
                    pad: false,
                },
            ));
        }
        if views.is_empty() {
            return HeliUniforms::none(cam, head);
        }
        views.sort_by(|a, b| a.0.total_cmp(&b.0));
        let views: Vec<HeliView> = views
            .into_iter()
            .map(|(_, v)| v)
            .take(farfall_render::heli::HELIS)
            .collect();
        // The planet underfoot hides what is past its limb.
        let occ = (to_ship(DVec3::ZERO), self.params.planet.radius_m as f32);
        let sun_ship = (ship_inv * self.params.sun.dir).as_vec3();
        HeliUniforms::new(cam, head, now, sun_ship, &views, occ)
    }
}

/// One eye on the world: the camera frame, plus where that eye actually
/// sits. Every world pass draws from a pose — the cockpit's (eye at the
/// ship's origin) or the chase rig's — so the chase view and the holo3PP
/// are the same code path as the canopy, never a special case.
#[derive(Debug, Clone, Copy)]
struct ViewPose {
    cam: CameraFrame,
    /// The view's rotation relative to the ship — the "head" every
    /// ship-frame pass takes.
    head: Quat,
    /// The eye's seat in the ship's frame, metres. ZERO in the cockpit.
    eye_ship: DVec3,
}

/// One of a headset's eyes this frame, in the seated reference space —
/// which is the ship's frame: right +X, up +Y, nose -Z.
#[derive(Debug, Clone, Copy)]
pub struct VrEye {
    /// The eye's orientation.
    pub head: Quat,
    /// The eye's seat, metres from the reference origin (the pilot's head
    /// at the start of the session).
    pub pos: Vec3,
    /// The frustum's tangents: left, right, up, down — all positive.
    pub tan: [f32; 4],
}

/// A headset on the frame: both eyes, as WebXR gave them.
#[derive(Debug, Clone, Copy)]
pub struct VrView {
    pub eyes: [VrEye; 2],
}

impl VrEye {
    /// A symmetric frustum wide enough to hold the eye's asymmetric one:
    /// the page maps the true frustum back out of it. Returns
    /// (fov_y, aspect) with aspect as tan(x)/tan(y).
    fn symmetric(&self) -> (f32, f32) {
        let tx = self.tan[0].max(self.tan[1]).max(1e-3);
        let ty = self.tan[2].max(self.tan[3]).max(1e-3);
        (2.0 * ty.atan(), tx / ty)
    }
}

/// The chase rig's seat: behind and above, in the ship's frame.
const CHASE_EYE_SHIP: DVec3 = DVec3::new(0.0, 3.2, 24.0);
/// Pitched down to put the ship in the middle of the frame.
const CHASE_PITCH_RAD: f32 = 0.13;

/// How far the nose is pitched down from prograde at spawn, degrees.
///
/// Near level, and it has to be: with the flight computer steering velocity
/// toward the nose, attitude *is* trajectory. Hold the horizon and the orbit
/// holds; pitch down and you descend. A steeply nose-down spawn would fly the
/// ship into the ground before the pilot touched anything.
const SPAWN_PITCH_DEG: f64 = 12.0;
/// Low enough that the horizon cuts across the frame while looking where you
/// are going — above roughly 14 km the planet drops entirely out of a
/// forward-facing view, because the surface is always 90 degrees off prograde.
/// Also close enough to make an approach, and eventually a collision, short.
const SPAWN_ALTITUDE_M: f64 = 12_000.0;

/// Under this altitude a FARFALL_BENCH_ALT run flies, it does not orbit:
/// the sim's circular orbit at 500 m is 787 m/s in sea-level air, which
/// lights the entry plasma (as it should — see thermal.wgsl) and fogs the
/// whole glass, so the ground and the air could never be looked at. The
/// stock 12 km scene is above this and unchanged.
const LOW_BENCH_CEILING_M: f64 = 8_000.0;
/// The airspeed of a low bench run, m/s: a fast cruise, a warm hull, no
/// sheath.
const LOW_BENCH_AIRSPEED_MPS: f64 = 250.0;
/// Where a low bench run is parked, latitude and longitude in degrees:
/// over a coast, so a capture shows land, sea and the line between them
/// (the orbit's own spot, 0°N 0°E, is open ocean to the horizon).
/// FARFALL_BENCH_ALT_AT=lat,lon overrides it.
const LOW_BENCH_LAT_LON_DEG: (f64, f64) = (10.0, 320.0);

/// Move a bench spawn from the orbit's spot to the low-flight spot at the
/// same altitude, keeping its attitude to the local horizon, at airspeed.
fn low_bench_flight(state: &mut sim::WorldState) {
    let (lat, lon) = std::env::var("FARFALL_BENCH_ALT_AT")
        .ok()
        .and_then(|v| {
            let p: Vec<f64> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            (p.len() == 2 && p[0].is_finite() && p[1].is_finite()).then(|| (p[0], p[1]))
        })
        .unwrap_or(LOW_BENCH_LAT_LON_DEG);
    let (lat, lon) = (lat.clamp(-89.0, 89.0).to_radians(), lon.to_radians());
    let up = DVec3::new(lat.cos() * lon.cos(), lat.sin(), lat.cos() * lon.sin());
    // The orbit spawns with +X up; take everything round to the new up.
    let rot = DQuat::from_rotation_arc(DVec3::X, up);
    let ship = &mut state.ship;
    ship.pos_m = rot * ship.pos_m;
    ship.vel_mps = rot * ship.vel_mps.normalize_or_zero() * LOW_BENCH_AIRSPEED_MPS;
    ship.orient = rot * ship.orient;
}

/// Base vertical field of view, degrees.
/// How much the view opens up under full boost, degrees. The camera reads the
/// ship's own thrust demand, so acceleration is *seen*, not just measured — the
/// cheapest honest speed cue there is, and it costs nothing but a lerp.
const FOV_THRUST_GAIN: f32 = 14.0;
/// Seconds for the view to catch up to a change in effort. Long enough that the
/// ship feels like it has mass, short enough that it still feels answerable.
const FOV_RESPONSE_S: f32 = 0.28;

/// Orbital attitude at spawn: nose prograde, belly toward the planet, pitched
/// down far enough that the disc is in frame immediately.
///
/// The preset leaves orientation at identity so the sim's golden hash stays a
/// property of the *orbit*, not of where the camera happens to be pointing —
/// choosing an attitude is the app's business, not the physics'.
/// FARFALL_CAPTURE=final: screenshots take the presented frame — post pass,
/// map, text and all — instead of the scene target.
fn capture_final() -> bool {
    std::env::var("FARFALL_CAPTURE").as_deref() == Ok("final")
}

/// "x,y" as two integers, for FARFALL_WINDOW_POS.
fn parse_vec2(s: &str) -> Option<(i32, i32)> {
    let mut it = s.split(',').map(|p| p.trim().parse::<i32>().ok());
    let x = it.next()??;
    let y = it.next()??;
    Some((x, y))
}
/// When a bench takes its capture, seconds on the frame clock: halfway
/// through the run (see `Gpu::bench_capture_due`). The bench knobs that
/// stage a moment — an after-image, strikes on the shield — are aged
/// against this, so they show in the capture whatever the run's length.
fn bench_capture_s() -> f32 {
    std::env::var("FARFALL_BENCH_SECONDS")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(20.0)
        * 0.5
}

/// "x,y,z" → vector, for the bench knobs.
fn parse_vec3(s: &str) -> Option<DVec3> {
    let mut it = s.split(',').map(|p| p.trim().parse::<f64>().ok());
    let v = DVec3::new(it.next()??, it.next()??, it.next()??);
    it.next().is_none().then_some(v)
}

/// Orientation with the ship's nose (-Z) along `dir`, rolled so `up` is as
/// close to the ship's +Y as the geometry allows.
fn look_at(dir: DVec3, up: DVec3) -> DQuat {
    let f = dir.normalize_or_zero();
    if f == DVec3::ZERO {
        return DQuat::IDENTITY;
    }
    let mut r = up.cross(-f);
    if r.length() < 1e-6 {
        r = DVec3::X.cross(-f);
    }
    let r = r.normalize();
    let u = (-f).cross(r);
    DQuat::from_mat3(&glam::DMat3::from_cols(r, u, -f))
}

fn spawn_attitude() -> DQuat {
    // The orbit starts at +X with velocity along -Z, so rolling -90 degrees
    // about the body Z axis puts the body's up (+Y) along world +X: radially
    // out, planet underfoot.
    let belly_down = DQuat::from_rotation_z(-std::f64::consts::FRAC_PI_2);
    // Then pitch the nose down toward the planet. In orbit the planet sits a
    // full 90 degrees off prograde — straight down — so a modest pitch does not
    // come close to reaching it: the disc has to be brought most of the way to
    // nadir before it enters the frame at all. `planet_is_in_view_at_spawn`
    // pins this down, because guessing at it got the framing wrong twice.
    belly_down * DQuat::from_rotation_x(-SPAWN_PITCH_DEG.to_radians())
}

/// What the adapter negotiation yields, on any platform.
struct GpuParts {
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    msaa_supported: Vec<u32>,
}

/// A device that arrived asynchronously (the web), waiting for the event
/// loop to pick it up and finish the renderer.
#[allow(dead_code)]
struct PendingGpu {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    settings: Settings,
    cfg: Config,
    parts: GpuParts,
}

/// The ordinary path: pick an adapter, open a device. Extracted so the VR
/// path (a device already handed to us by `xr::init`) and the flat path
/// (this) share the surface-configuration code below unchanged.
async fn request_flat_device(
    instance: &wgpu::Instance,
    surface: &wgpu::Surface<'static>,
) -> (wgpu::Adapter, wgpu::Device, wgpu::Queue) {
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(surface),
            ..Default::default()
        })
        .await
        .expect("no suitable GPU adapter");
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("farfall device"),
            // Lets the adapter's real sample-count support count,
            // rather than only the spec's guaranteed {1, 4}.
            required_features: adapter.features()
                & wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
        })
        .await
        .expect("request device");
    (adapter, device, queue)
}

/// Ask for the adapter and device, and configure the surface for it. In
/// VR, `vr_device` is already a device the OpenXR runtime approved
/// (`xr::init`): `request_adapter`/`request_device` are skipped, since
/// picking a *different* adapter than the one the headset was born on
/// would be a second GPU the runtime never agreed to.
async fn request_gpu(
    instance: &wgpu::Instance,
    surface: &wgpu::Surface<'static>,
    window: &Window,
    cfg: &Config,
    #[cfg(not(target_arch = "wasm32"))] vr_device: Option<(
        wgpu::Adapter,
        wgpu::Device,
        wgpu::Queue,
    )>,
) -> GpuParts {
    #[cfg(not(target_arch = "wasm32"))]
    let vr_active = vr_device.is_some();
    #[cfg(target_arch = "wasm32")]
    let vr_active = false;
    #[cfg(not(target_arch = "wasm32"))]
    let (adapter, device, queue) = match vr_device {
        Some((a, d, q)) => (a, d, q),
        None => request_flat_device(instance, surface).await,
    };
    #[cfg(target_arch = "wasm32")]
    let (adapter, device, queue) = request_flat_device(instance, surface).await;
    let size = window.inner_size();
    let mut config = surface
        .get_default_config(&adapter, size.width.max(1), size.height.max(1))
        .expect("surface unsupported by adapter");
    config.present_mode = if vr_active {
        // The monitor's own refresh must never pace the headset's: the
        // mirror is drawn whenever a VR frame lands, not on its own clock.
        wgpu::PresentMode::AutoNoVsync
    } else if cfg.vsync {
        wgpu::PresentMode::AutoVsync
    } else {
        wgpu::PresentMode::AutoNoVsync
    };
    // VR always needs to be able to capture the mirror (a VR bench's own
    // captures always come from it — xr_composite — regardless of
    // whether FARFALL_CAPTURE=final was also set for the flat path).
    if capture_final() || vr_active {
        config.usage |= wgpu::TextureUsages::COPY_SRC;
    }
    surface.configure(&device, &config);
    // Which sample counts this GPU can actually render at, for this
    // format. Metal on an M1 offers 1 and 4; asking for 2 or 8 is a
    // validation panic at pipeline creation, so the menu may only
    // offer what is here.
    let specific = device
        .features()
        .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES);
    let flags = adapter.get_texture_format_features(config.format).flags;
    let msaa_supported: Vec<u32> = settings::MSAA_CHOICES
        .iter()
        .copied()
        .filter(|&n| n == 1 || n == 4 || (specific && flags.sample_count_supported(n)))
        .collect();
    GpuParts {
        device,
        queue,
        config,
        msaa_supported,
    }
}

#[derive(Default)]
struct App {
    gpu: Option<Gpu>,
    game: Option<Game>,
    audio: Option<Audio>,
}

impl App {
    fn init_gpu(&mut self, event_loop: &ActiveEventLoop) {
        let settings = Settings::load();
        let cfg = Config::from_env(&settings);
        let title = if cfg.bench {
            "FARFALL — BENCHMARK (simulation frozen, controls inert)"
        } else {
            "FARFALL"
        };
        let mut attrs = Window::default_attributes().with_title(title);
        if let Some((w, h)) = cfg.bench_size {
            attrs = attrs
                .with_inner_size(winit::dpi::PhysicalSize::new(w, h))
                .with_decorations(false);
        }
        if cfg.bench {
            // A benchmark window is born unfocused and, if asked, on another
            // screen (FARFALL_WINDOW_POS=x,y in desktop pixels): it must never
            // take the keyboard or the mouse from whoever is working here.
            attrs = attrs.with_active(false);
            if let Some(pos) = std::env::var("FARFALL_WINDOW_POS")
                .ok()
                .and_then(|v| parse_vec2(&v))
            {
                attrs = attrs.with_position(winit::dpi::PhysicalPosition::new(pos.0, pos.1));
            }
        }
        if !cfg.windowed && cfg!(not(target_arch = "wasm32")) {
            // Borderless fullscreen on the current monitor: no mode switch, so
            // alt-tab stays instant and the resolution is the desktop's.
            attrs = attrs.with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
        }
        #[cfg(target_arch = "wasm32")]
        let attrs = web::with_canvas(attrs);
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));

        // VR HEADSET (SPEC §5.3): the Vulkan device must be born from the
        // OpenXR runtime, so this has to happen before wgpu's own instance
        // does — `xr::init` never launches a runtime that isn't already
        // running, and any failure here falls back to flat with a log line.
        // FARFALL_VR=synth skips this entirely: a synthetic headset needs
        // no runtime and no special-cased device, so it is built later,
        // after the ordinary flat device exists (see `finish_init`) — the
        // exact reason `cfg.vr_synth` never calls `xr::init` here.
        #[cfg(not(target_arch = "wasm32"))]
        let xr_init = (cfg.vr && !cfg.vr_synth)
            .then(|| xr::init(cfg.vr_scale))
            .flatten();
        // A VR bench that silently ran flat is a lie: the whole point of
        // FARFALL_BENCH=1 FARFALL_VR=1 is numbers and captures from
        // inside the OpenXR session, so a bench, unlike ordinary play,
        // never falls back — it fails the run outright. Never fires for
        // synth: `xr_init` is deliberately `None` there, and `finish_init`
        // builds the synthetic session unconditionally (it cannot fail
        // the way a real runtime's init can).
        #[cfg(not(target_arch = "wasm32"))]
        if cfg.bench && cfg.vr && !cfg.vr_synth && xr_init.is_none() {
            log::error!("VR bench: OpenXR init failed (see the warning above); exiting");
            std::process::exit(3);
        }

        let display_handle = event_loop.owned_display_handle();
        #[cfg(not(target_arch = "wasm32"))]
        let instance = match &xr_init {
            Some((vr, _, _)) => vr.instance.clone(),
            None => {
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
                    Box::new(display_handle),
                ))
            }
        };
        #[cfg(target_arch = "wasm32")]
        let instance = wgpu::Instance::new(
            wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(display_handle)),
        );
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");

        #[cfg(not(target_arch = "wasm32"))]
        {
            let (vr_device, xr) = match xr_init {
                Some((vr, session, format)) => (
                    Some((vr.adapter, vr.device, vr.queue)),
                    Some((session, format)),
                ),
                None => (None, None),
            };
            let parts =
                pollster::block_on(request_gpu(&instance, &surface, &window, &cfg, vr_device));
            self.finish_init(window, surface, settings, cfg, parts, xr);
        }
        #[cfg(target_arch = "wasm32")]
        {
            // The browser hands out the device asynchronously and there is
            // no blocking on the main thread: the parts land in a mailbox
            // and the redraw that follows finishes the job.
            wasm_bindgen_futures::spawn_local(async move {
                let parts = request_gpu(&instance, &surface, &window, &cfg).await;
                web::PENDING.with(|p| {
                    *p.borrow_mut() = Some(PendingGpu {
                        window: window.clone(),
                        surface,
                        settings,
                        cfg,
                        parts,
                    })
                });
                window.request_redraw();
            });
        }
    }

    /// A device the browser delivered since the last event: finish the
    /// renderer with it.
    #[cfg(target_arch = "wasm32")]
    fn pick_up_pending(&mut self) {
        if self.gpu.is_none() {
            if let Some(p) = web::PENDING.with(|p| p.borrow_mut().take()) {
                self.finish_init(p.window, p.surface, p.settings, p.cfg, p.parts);
            }
        }
    }

    /// Everything after the device: the passes, the game, the audio.
    fn finish_init(
        &mut self,
        window: Arc<Window>,
        surface: wgpu::Surface<'static>,
        settings: Settings,
        cfg: Config,
        parts: GpuParts,
        #[cfg(not(target_arch = "wasm32"))] xr: Option<(xr::XrSession, wgpu::TextureFormat)>,
    ) {
        let GpuParts {
            device,
            queue,
            config,
            msaa_supported,
        } = parts;
        // FARFALL_VR=synth: no real session came from `xr_init` (it was
        // never attempted — see `App::init_gpu`), so the synthetic
        // headset is built here instead, now that the ordinary flat
        // device exists. Cannot fail the way a real runtime's init can:
        // it is plain wgpu textures on a device already in hand.
        #[cfg(not(target_arch = "wasm32"))]
        let (xr_session, xr_format) = match xr {
            Some((s, f)) => (Some(s), Some(f)),
            None if cfg.vr_synth => {
                let (s, f) = xr::init_synth(
                    &device,
                    cfg.vr_scale,
                    cfg.vr_script,
                    cfg.bench_seconds as f32,
                );
                (Some(s), Some(f))
            }
            None => (None, None),
        };
        let mut cfg = cfg;
        if !msaa_supported.contains(&cfg.msaa) {
            let fallback = msaa_supported
                .iter()
                .copied()
                .filter(|&n| n <= cfg.msaa)
                .max()
                .unwrap_or(1);
            log::warn!(
                "MSAA {}x unsupported here (have {:?}); using {}x",
                cfg.msaa,
                msaa_supported,
                fallback
            );
            cfg.msaa = fallback;
        }
        let msaa_in_use = cfg.msaa;

        log::info!(
            "renderer: {}x MSAA, vsync {}, gpu_sync {}, {:?}",
            cfg.msaa,
            if cfg.vsync { "on" } else { "off" },
            cfg.gpu_sync,
            config.format
        );
        let scene = SceneTarget::new(cfg.msaa, config.format, cfg.scale);
        let blit = BlitPass::new(&device, config.format);
        let post = PostPass::new(&device, config.format, cfg.msaa);
        // Bake the static world fields before the first frame. Everything the
        // planet pass reads per pixel is generated here, by shader, once.
        let baked = BakedMaps::bake(&device, &queue);
        let mut nebula = NebulaBake::new(&device);
        // Baked properly once the game (and its bench knobs) exists, below.
        nebula.bake(&device, &queue, nebula_params(&settings));
        let passes = Passes::new(
            &device,
            config.format,
            cfg.msaa,
            &baked,
            &nebula.view,
            settings.cockpit_res,
        );
        // The HUD draws straight onto the swapchain, after the upscale, so it
        // is always native resolution and single-sampled however low the scene
        // scale goes (P1: the readout must never soften).
        let hud = HudPass::new(&device, config.format, 1);
        let map = InstrumentPass::new_pane_sized(
            &device,
            config.format,
            1,
            "map",
            farfall_render::shaders::MAP,
            map::UNIFORM_BYTES,
        );
        let hologram = hologram_pass(&device, config.format, 1);
        let pointer = pointer_pass(&device, config.format, 1);

        window.request_redraw();
        // Audio: live synthesis, muted for benchmarks (a frozen sim droning
        // at full volume helps nobody) and by FARFALL_MUTE.
        let mute = cfg.bench
            || matches!(
                std::env::var("FARFALL_MUTE").as_deref(),
                Ok("1" | "on" | "true")
            );
        if !mute {
            self.audio = Audio::start();
            if self.audio.is_none() {
                log::warn!("audio: no output device, running silent");
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        let vr_pair = xr_session.as_ref().zip(xr_format).map(|(session, fmt)| {
            // xrblit.wgsl is a bare passthrough (no manual gamma math): it
            // relies entirely on the sample view and the store view both
            // being correctly tagged sRGB or not. The pair render target
            // is built at the window's own surface format (matching the
            // flat-mode scene target, SceneTarget::new(.., config.format,
            // ..) — same tonemap pass, same target-format contract); the
            // XR swapchain is *always* one of SWAPCHAIN_FORMATS' sRGB
            // variants (xr::try_init hard-errors otherwise). If those two
            // formats ever disagree, the headset leg either double- or
            // never-gamma-corrects — log it plainly so a dim headset and
            // a correct-looking mirror point straight here.
            if config.format != fmt {
                log::warn!(
                    "VR: window surface format {:?} != XR swapchain format {fmt:?} — \
                     the headset image may be gamma-mismatched even though the desktop \
                     mirror (drawn in the window's own format) looks right",
                    config.format
                );
            }
            VrPair::new(&device, config.format, fmt, session.eye_size())
        });
        self.gpu = Some(Gpu {
            #[cfg(not(target_arch = "wasm32"))]
            xr: xr_session,
            window,
            device,
            queue,
            surface,
            config,
            scene,
            #[cfg(not(target_arch = "wasm32"))]
            vr_pair,
            #[cfg(not(target_arch = "wasm32"))]
            eye_order_checked: false,
            #[cfg(not(target_arch = "wasm32"))]
            overlay_depth_logged: std::cell::Cell::new(false),
            post,
            blit,
            passes,
            baked,
            nebula,
            hud,
            map,
            hologram,
            pointer,
            dial_rects: std::cell::Cell::new([None; 5]),
            text: TextBitmap::new(),
            cfg,
            perf: Perf::new(),
            auto_scale: 1.0,
            auto_scale_at: Instant::now(),
            capture_requested: false,
            bench_captured: false,
            bench_spin_taken: 0,
        });
        let mut game = Game::new();
        let mut settings = settings;
        settings.msaa = msaa_in_use;
        // FARFALL_FOV=deg: the field of view for this run, over the file's
        // graphics.fov — a graphics knob like FARFALL_SCALE, not bench-only.
        if let Some(f) = std::env::var("FARFALL_FOV")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|f| f.is_finite())
        {
            settings.fov = f.clamp(settings::FOV_MIN, settings::FOV_MAX);
        }
        game.apply_settings(settings);
        // RESUME: pick up the last quit's world, unless this is a bench
        // (a scene capture must stay reproducible — it must never see a
        // resumed world) or the pilot/profiler has turned it off.
        if resume_allowed(
            game.settings.resume,
            game.frozen,
            env_resume().as_deref(),
            bench_spawn_env_present(),
        ) {
            if let Some(s) = save::load() {
                game.restore(&s);
            }
        }
        // FARFALL_BENCH_RESUME=<path>: bench-only, for e2e verification —
        // load a save from an EXPLICIT path (never ~/.farfall) through
        // the same parse/seal check as a real resume, then keep the bench
        // frozen (restore itself always clears `frozen`, since a real
        // resume can only ever happen when it was already false). A
        // tampered or unreadable file is refused and logged; the bench
        // simply stays at the stock orbit, same as any other refusal.
        let bench_resume_path = std::env::var_os("FARFALL_BENCH_RESUME");
        if bench_path_action_allowed(game.frozen, bench_resume_path.as_deref()) {
            let bench_resume_path = std::path::PathBuf::from(bench_resume_path.unwrap());
            match save::load_from(&bench_resume_path) {
                Some(s) => {
                    game.restore(&s);
                    game.frozen = true;
                    log::info!("bench: resumed from {}", bench_resume_path.display());
                }
                None => log::warn!(
                    "bench: FARFALL_BENCH_RESUME {} refused (missing, unreadable, \
                     or failed validation)",
                    bench_resume_path.display()
                ),
            }
        }
        game.menu.set_msaa_supported(&msaa_supported);
        // The CONTROLS card: on the first run (no file yet), or at every
        // start if asked; never in a bench unless a capture wants it.
        if (!game.frozen && (!Settings::file_exists() || game.settings.controls_card))
            || (game.frozen && std::env::var("FARFALL_BENCH_CARD").is_ok())
        {
            game.open_card();
        }
        if game.frozen && std::env::var("FARFALL_BENCH_MAP").is_ok() {
            game.toggle_map();
        }
        if let Some(style) = std::env::var("FARFALL_BENCH_STYLE")
            .ok()
            .filter(|_| game.frozen)
            .and_then(|v| settings::GaugeStyle::from_key(v.trim()))
        {
            // The cockpit's style, over any per-dial choice in the file.
            game.settings.gauge_style = style;
            for d in game.settings.dials.iter_mut() {
                d.style = None;
            }
        }
        if let Ok(v) = std::env::var("FARFALL_BENCH_FIT") {
            if game.frozen {
                for (slot, k) in game.settings.mounts.iter_mut().zip(v.split(',')) {
                    if let Some(m) = bay::Mount::from_key(k.trim()) {
                        *slot = m;
                    }
                }
                game.arms.mounts = game.settings.mounts;
            }
        }
        if game.frozen && std::env::var("FARFALL_BENCH_SHIP").is_ok() {
            game.toggle_bay();
            game.bay_dropdown = Some(0);
            // A pointer in the picture, over the hologram.
            let (w, h) = self.gpu.as_ref().map_or((1600.0, 1200.0), |g| {
                (g.config.width as f32, g.config.height as f32)
            });
            game.window_size = (w, h);
            game.cursor = Some((w * 0.62, h * 0.36));
        }
        if game.frozen && std::env::var("FARFALL_BENCH_DESIGN").is_ok() {
            game.toggle_design();
        }
        if game.frozen && std::env::var("FARFALL_BENCH_CRAFT").is_ok_and(|v| v.trim() == "heli") {
            game.settings.craft = bay::Craft::Helicopter;
            game.params.ship = game.own_ship_params();
        }
        if game.frozen && std::env::var("FARFALL_BENCH_CHASE").is_ok() {
            game.settings.camera_chase = true;
        }
        if game.frozen && std::env::var("FARFALL_BENCH_HOLO").is_ok() {
            game.settings.holo_view = true;
        }
        // The dust on its own: a bench row for the motes.
        if let Ok(v) = std::env::var("FARFALL_BENCH_DUST") {
            if game.frozen {
                if let Ok(f) = v.trim().parse::<f32>() {
                    if f.is_finite() {
                        game.settings.dust = f.clamp(0.0, 2.0);
                    }
                }
            }
        }
        // The nebula: "off" for a baseline, anything else a full sky of it —
        // the stock glow doubled, every cloud, spread wide — so a capture
        // sees it whichever way it looks (hues and seed from the settings).
        if let Ok(v) = std::env::var("FARFALL_BENCH_NEBULA") {
            if game.frozen {
                if v.trim() == "off" {
                    game.settings.nebula = 0.0;
                } else {
                    game.settings.nebula = 2.0;
                    game.settings.nebula_clouds = 8;
                    game.settings.nebula_spread = 3.0;
                }
            }
        }
        if game.frozen && std::env::var("FARFALL_BENCH_HYPER").is_ok() {
            game.hyper = 1.0;
            game.bench_hyper = true;
        }
        if let Some(age) = std::env::var("FARFALL_BENCH_GHOST")
            .ok()
            .filter(|_| game.frozen)
            .and_then(|v| v.trim().parse::<f32>().ok())
        {
            // The capture lands halfway through the bench: an image that
            // old then, banked a little, gone down the nose.
            let o = game.state.ship.orient;
            game.ghost = Some(Ghost {
                orient: o * DQuat::from_rotation_z(0.35) * DQuat::from_rotation_x(-0.15),
                dir_world: o * DVec3::new(0.15, 0.05, -1.0).normalize(),
                at_s: bench_capture_s() - age,
            });
        }
        let bench_arms = std::env::var("FARFALL_BENCH_ARMS").unwrap_or_default();
        let bench_mimic = std::env::var("FARFALL_BENCH_MIMIC").unwrap_or_default();
        if game.frozen && !bench_mimic.is_empty() {
            // A ship out of a rock a little way ahead and left, turned
            // three-quarters to us: mid-reveal (hologram over hardening
            // hull), hailing, attacking (with its fire in the air and a
            // hit on the shell), or a wreck.
            let o = game.state.ship.orient;
            let p = game.state.ship.pos_m;
            let v = game.state.ship.vel_mps;
            let t = game.state.time_s;
            let at = p + o * DVec3::new(-9.0, 2.5, -40.0);
            let orient = o * DQuat::from_rotation_y(1.15) * DQuat::from_rotation_x(0.10);
            use mimic::{Mood, Phase, REVEAL_S};
            let (born, phase, mood) = match bench_mimic.as_str() {
                "hail" => (t - REVEAL_S - 1.0, Phase::Hailing, Mood::Hail),
                "attack" => (t - REVEAL_S - 1.0, Phase::Attacking, Mood::Hostile),
                "wreck" => (t - REVEAL_S - 1.0, Phase::Wreck, Mood::Hostile),
                _ => (t - REVEAL_S * 0.55, Phase::Revealing, Mood::Hostile),
            };
            let mut m = mimic::Mimic::planted(at, v, orient, born, phase, mood, 0.37);
            match phase {
                Phase::Hailing => {
                    m.effort = 0.2;
                    game.mimics.line = Some((mimic::hail_text(0.37).to_string(), t + 30.0));
                }
                Phase::Attacking => {
                    m.effort = 0.85;
                    game.mimics.line = Some(("HULL 93%  UNDER FIRE".to_string(), t + 30.0));
                    for i in 0..4 {
                        let dir =
                            (p + o * DVec3::new(0.6 * i as f64 - 1.0, -0.4, 0.0) - at).normalize();
                        let dist = 12.0 + 15.0 * i as f64;
                        game.mimics.slugs.push(mimic::FoeSlug {
                            pos: at + dir * dist,
                            vel: v + dir * arms::Weapon::Cannon.muzzle_mps(),
                            born_s: t - dist / arms::Weapon::Cannon.muzzle_mps(),
                        });
                    }
                    game.impacts.insert(
                        0,
                        Impact {
                            dir: Vec3::new(-0.2, 0.1, -1.0).normalize(),
                            at_s: game.started.elapsed().as_secs_f32() - 0.4,
                            size: 0.55,
                        },
                    );
                }
                Phase::Wreck => {
                    m.wound_j = mimic::MIMIC_TOUGH_J;
                }
                _ => {}
            }
            game.mimics.ships.push(m);
        }
        // FARFALL_BENCH_MINERS=tier[,mine|fight]: a miner of that tier
        // ahead-left, its beam on a rock planted ahead of it (mine, the
        // default) or come about with its fire in the air (fight); and a
        // far speck of a second one.
        let bench_miners = std::env::var("FARFALL_BENCH_MINERS").unwrap_or_default();
        if game.frozen && !bench_miners.is_empty() {
            let mut parts = bench_miners.split(',');
            let tier: usize = parts
                .next()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            let tier = tier.min(miner::TIERS - 1);
            let stage = parts.next().unwrap_or("mine").trim().to_string();
            let o = game.state.ship.orient;
            let p = game.state.ship.pos_m;
            let v = game.state.ship.vel_mps;
            let t = game.state.time_s;
            let haul_t = miner::TIER_T[tier] + 5.0;
            let size = miner::TIER_SIZE[tier];
            // Close enough to read at any tier: a tier-0 miner where the
            // mimic bench puts its ship, the big ones a little further out
            // but still filling the glass more than a fighter does.
            let at = p + o * DVec3::new(-3.0 * size, 1.2 * size, -28.0 - 11.0 * size);
            if stage == "fight" {
                let orient = o * DQuat::from_rotation_y(2.6) * DQuat::from_rotation_x(0.08);
                let mut m = miner::Miner::planted(
                    at,
                    v,
                    orient,
                    haul_t,
                    miner::Phase::Attacking,
                    miner::Temper::Hostile,
                    0.61,
                );
                m.effort = 0.9;
                m.sheen = if tier >= 2 { 0.8 } else { 0.0 };
                m.wound_j = m.tough_j() * 0.3;
                game.mimics.line = Some(("HULL 88%  UNDER FIRE".to_string(), t + 30.0));
                for i in 0..5 {
                    let dir =
                        (p + o * DVec3::new(0.7 * i as f64 - 1.4, -0.3, 0.0) - at).normalize();
                    let dist = 10.0 * size + 14.0 * i as f64;
                    game.mimics.slugs.push(mimic::FoeSlug {
                        pos: at + dir * dist,
                        vel: v + dir * arms::Weapon::Cannon.muzzle_mps(),
                        born_s: t - dist / arms::Weapon::Cannon.muzzle_mps(),
                    });
                }
                game.impacts.insert(
                    0,
                    Impact {
                        dir: Vec3::new(-0.3, 0.1, -1.0).normalize(),
                        at_s: game.started.elapsed().as_secs_f32() - 0.4,
                        size: 0.6,
                    },
                );
                game.miners.ships.push(m);
            } else {
                // A rock ahead of the miner, the beam on its near face.
                // The rock off to the miner's right, so the hull is seen
                // side-on and the beam runs across the glass.
                let rock_at = at + o * DVec3::new(18.0 + 14.0 * size, -3.0, -14.0 * size);
                let radius = 12.0 + 5.0 * size;
                let rock = belt::Rock {
                    id: (0, 0, 0, 253),
                    pos: rock_at,
                    vel: v,
                    radius_m: radius,
                    seed: 0.62,
                    spin: 0.0,
                };
                game.belt.rocks.insert(0, rock);
                let mut m = miner::Miner::planted(
                    at,
                    v,
                    mimic::look_at(rock_at - at, o * DVec3::Y),
                    haul_t,
                    miner::Phase::Mining,
                    miner::Temper::Neutral,
                    0.27,
                );
                m.claim = Some(rock.id);
                m.effort = 0.25;
                game.mimics.line = Some((miner::hail_text(0.27, tier).to_string(), t + 30.0));
                game.miners.ships.push(m);
            }
            // A second miner a long way off: a speck with a marker.
            let far = p + o * DVec3::new(900.0, 260.0, -2_600.0);
            let mut m2 = miner::Miner::planted(
                far,
                v,
                o,
                miner::TIER_T[1] + 5.0,
                miner::Phase::Transit,
                miner::Temper::Neutral,
                0.8,
            );
            m2.effort = 0.7;
            game.miners.ships.push(m2);
            game.miners.placed = true;
        }
        if game.frozen && bench_arms == "sight" {
            game.arms.heat[0] = 0.55;
        }
        if game.frozen && bench_arms == "nosight" {
            // The same frame without the sight, for the scene test's
            // baseline.
            game.settings.arms_sight = 0.0;
        }
        if game.frozen && bench_arms == "scars" {
            // A rock of our own a little way ahead, wearing three craters
            // on the face toward us: one just struck, one cooling, one
            // nearly cold.
            let o = game.state.ship.orient;
            let p = game.state.ship.pos_m;
            let v = game.state.ship.vel_mps;
            let t = game.state.time_s;
            let rock = belt::Rock {
                id: (0, 0, 0, 250),
                pos: p + o * DVec3::new(6.0, -4.0, -140.0),
                vel: v,
                radius_m: 30.0,
                seed: 0.6,
                spin: 0.0,
            };
            game.belt.rocks.push(rock);
            let toward = (p - rock.pos).normalize_or_zero();
            let side = toward.cross(DVec3::Y).normalize_or_zero();
            let up = side.cross(toward);
            for (k, (age, ox, oy)) in [(0.3, -0.55, 0.1), (4.0, 0.05, -0.35), (10.0, 0.5, 0.3)]
                .iter()
                .enumerate()
            {
                game.arms.scars.push(arms::Scar {
                    rock: rock.id,
                    dir: (toward + side * *ox + up * *oy).normalize_or_zero(),
                    born_s: t - *age,
                    size_m: 5.0,
                    seed: 0.2 + 0.3 * k as f32,
                });
            }
        }
        if game.frozen && bench_arms == "debris" {
            // A rock just broken ahead: its shards spread out in a cloud,
            // the freshest still glowing, a few chips nearer; the break's
            // own burst behind them.
            let o = game.state.ship.orient;
            let p = game.state.ship.pos_m;
            let v = game.state.ship.vel_mps;
            let t = game.state.time_s;
            let mut h = 0.37f32;
            let mut unit = || {
                h = (h * 9.731 + 0.173).fract();
                h
            };
            for i in 0..48 {
                let age = 0.05 + 2.4 * unit();
                let dir = DVec3::new(
                    unit() as f64 - 0.5,
                    unit() as f64 - 0.5,
                    unit() as f64 - 0.5,
                )
                .normalize_or_zero();
                let speed = 4.0 + 14.0 * unit() as f64;
                let at = DVec3::new(0.0, -1.0, -70.0) + dir * speed * age as f64;
                let axis = DVec3::new(
                    unit() as f64 - 0.5,
                    unit() as f64 - 0.5,
                    unit() as f64 - 0.5,
                )
                .normalize_or_zero();
                let spin = 0.5 + 3.0 * unit() as f64;
                let size = if i % 8 == 0 { 3.5 } else { 0.6 + 1.6 * unit() };
                let seed = unit();
                game.arms.shards.push(arms::Shard {
                    pos: p + o * at,
                    vel: v + o * dir * speed,
                    axis,
                    spin,
                    size,
                    born_s: t - age as f64,
                    life_s: 5.0,
                    seed,
                });
            }
            game.arms.bursts.push(arms::Burst {
                pos: p + o * DVec3::new(0.0, -1.0, -70.0),
                vel: v,
                at_s: t - 0.9,
                kind: 2,
                size: 1.2,
                seed: 0.4,
            });
        }
        if game.frozen && (bench_arms == "1" || bench_arms == "guns") {
            // Both guns in the air and every kind of burst at once, ahead:
            // cannon tracers from both wings a little way out, a rail slug
            // further, a muzzle flash on the right wing, hits and a rock
            // breaking down the nose.
            let o = game.state.ship.orient;
            let p = game.state.ship.pos_m;
            let v = game.state.ship.vel_mps;
            let t = game.state.time_s;
            for i in 0..6 {
                let side = if i % 2 == 0 {
                    arms::WING_L
                } else {
                    arms::WING_R
                };
                let dist = 30.0 + 90.0 * i as f64;
                let dir =
                    o * DVec3::new(0.012 * (i as f64 - 2.5), 0.004 * i as f64, -1.0).normalize();
                game.arms.slugs.push(arms::Slug {
                    pos: p + o * side + dir * dist,
                    vel: v + dir * arms::Weapon::Cannon.muzzle_mps(),
                    born_s: t - dist / arms::Weapon::Cannon.muzzle_mps(),
                    weapon: arms::Weapon::Cannon,
                });
            }
            let dir = o * DVec3::new(-0.05, 0.02, -1.0).normalize();
            game.arms.slugs.push(arms::Slug {
                pos: p + o * arms::NOSE + dir * 700.0,
                vel: v + dir * arms::Weapon::Rail.muzzle_mps(),
                born_s: t - 0.12,
                weapon: arms::Weapon::Rail,
            });
            let burst = |at: DVec3, age: f64, kind: u8, size: f32, seed: f32| arms::Burst {
                pos: p + o * at,
                vel: v,
                at_s: t - age,
                kind,
                size,
                seed,
            };
            game.arms
                .bursts
                .push(burst(arms::WING_R, 0.02, 0, 1.0, 0.2));
            game.arms
                .bursts
                .push(burst(DVec3::new(6.0, 2.0, -260.0), 0.15, 1, 0.8, 0.5));
            game.arms
                .bursts
                .push(burst(DVec3::new(-40.0, 10.0, -500.0), 0.3, 3, 1.2, 0.7));
            game.arms
                .bursts
                .push(burst(DVec3::new(25.0, -8.0, -380.0), 0.5, 2, 1.0, 0.9));
            game.arms.heat[0] = 0.4;
        }
        if let Some(k) = std::env::var("FARFALL_BENCH_CLOUDS")
            .ok()
            .filter(|_| game.frozen)
            .and_then(|v| v.trim().parse::<f32>().ok())
        {
            game.settings.clouds = k.clamp(0.0, 2.0);
        }
        if let Some(n) = std::env::var("FARFALL_BENCH_STRIKES")
            .ok()
            .filter(|_| game.frozen)
            .and_then(|v| v.trim().parse::<usize>().ok())
        {
            // Staggered over the last second, spread round the nose: each
            // ripple is at a different age in the capture, which lands
            // halfway through the bench.
            let cap = bench_capture_s();
            for i in 0..n.min(farfall_render::shield::IMPACTS) {
                let spread = random_unit(0.3 + 0.1 * i as f32, 0.13 * i as f32).as_vec3();
                game.impacts.push(Impact {
                    dir: (glam::Vec3::NEG_Z + spread * 0.8).normalize(),
                    at_s: cap - 0.15 - 0.3 * i as f32,
                    size: 0.3 + 0.25 * (i % 4) as f32,
                });
            }
        }
        if let Some(g) = std::env::var("FARFALL_BENCH_G")
            .ok()
            .filter(|_| game.frozen)
            .and_then(|v| {
                let p: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                (p.len() == 3).then(|| [p[0], p[1], p[2]])
            })
        {
            game.felt_g_body = g;
            game.felt_g = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt();
        }
        if let Some(pose) = std::env::var("FARFALL_BENCH_SHAKE")
            .ok()
            .filter(|_| game.frozen)
            .and_then(|v| {
                let p: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                (p.len() == 3).then(|| [p[0], p[1], p[2]])
            })
        {
            game.bench_shake = Some([
                pose[0].to_radians(),
                pose[1].to_radians(),
                pose[2].to_radians(),
            ]);
        }
        if let Some(pages) = std::env::var("FARFALL_BENCH_MENU")
            .ok()
            .filter(|_| game.frozen)
            .and_then(|v| v.trim().parse::<usize>().ok())
        {
            game.menu.toggle();
            for _ in 0..pages {
                game.menu.key(KeyCode::Tab, &mut game.settings);
            }
        }
        // FARFALL_BENCH_PROFILE=reforger: the stick wears the Reforger
        // helicopter-pilot map, and an open menu walks its cursor to the
        // PROFILE row so the worn value and its line show.
        if std::env::var("FARFALL_BENCH_PROFILE").is_ok_and(|v| v.trim() == "reforger")
            && game.frozen
        {
            game.settings.stick = stick::StickMap::reforger_heli();
            if game.menu.open {
                for _ in 0..3 {
                    game.menu.key(KeyCode::ArrowDown, &mut game.settings);
                }
            }
        }
        if let Some(step) = std::env::var("FARFALL_BENCH_STICK")
            .ok()
            .filter(|_| game.frozen)
            .and_then(|v| v.trim().parse::<usize>().ok())
        {
            if !game.menu.open {
                game.menu.toggle();
            }
            let mut w = stick::Wizard::at_step(step);
            w.bench_detect();
            game.wizard = Some(w);
        }
        // FARFALL_BENCH_DEMAND=p,r,y,t: a parked control demand, for a
        // capture of the console's stick and lever answering it.
        if let Some([p, r, y, t]) = std::env::var("FARFALL_BENCH_DEMAND")
            .ok()
            .filter(|_| game.frozen)
            .and_then(|v| {
                let n: Vec<f64> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                <[f64; 4]>::try_from(n).ok()
            })
        {
            // The mirror's senses: +p pitch up (+x torque), +r roll right
            // (-z), +y yaw right (-y), +t thrust ahead (-z). It stands for
            // the whole run: the bench never polls a real stick.
            game.input.set_stick([0.0, 0.0, -t, p, -y, -r]);
        }
        if game.frozen && std::env::var("FARFALL_BENCH_LAND").is_ok() {
            game.toggle_landing();
            game.touchdown = landing::predict(&game.params, &game.state.ship, game.state.time_s);
        }
        if game.frozen && std::env::var("FARFALL_BENCH_DISEMBARK").is_ok() {
            game.disembark();
        }
        if game.frozen && std::env::var("FARFALL_BENCH_EVA").is_ok() {
            // On foot for the capture: walked out a dozen metres, turned
            // to look back at the parked ship.
            game.disembark();
            game.stage_eva_bench();
        }
        if let Some(head) = std::env::var("FARFALL_BENCH_HEAD")
            .ok()
            .filter(|_| game.frozen)
            .and_then(|v| {
                let (a, b) = v.split_once(',')?;
                Some((a.trim().parse::<f32>().ok()?, b.trim().parse::<f32>().ok()?))
            })
        {
            game.look.aim(head.0.to_radians(), head.1.to_radians());
        }
        // The nebula, from the game's settings: a no-op unless a bench knob
        // moved them.
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.nebula
                .bake(&gpu.device, &gpu.queue, nebula_params(&game.settings));
        }
        self.game = Some(game);
    }
}

/// What [`xr_begin_frame`] found: whether the rest of `redraw` should
/// draw this call at all.
#[cfg(not(target_arch = "wasm32"))]
enum XrGate {
    /// No native session (never asked for one, or it isn't up yet): the
    /// flat/WebXR path runs exactly as it always has.
    NoVr,
    /// A VR frame is open and `game.vr` holds this frame's eyes.
    Rendering,
    /// Nothing to draw this call — the runtime isn't ready for a frame,
    /// or asked for one it doesn't want rendered.
    SkipFrame,
}

/// Poll and open this frame's native VR session, if one is up. Must run
/// before `game.tick()`, so a located pose drives the frame it was
/// predicted for.
#[cfg(not(target_arch = "wasm32"))]
fn xr_begin_frame(gpu: &mut Gpu, game: &mut Game) -> XrGate {
    let Some(session) = gpu.xr.as_mut() else {
        return XrGate::NoVr;
    };
    match session.begin_frame(gpu.cfg.vr_force_render) {
        xr::Frame::Idle => {
            game.vr = None;
            XrGate::SkipFrame
        }
        xr::Frame::Lost => {
            log::warn!("VR: the session is gone; falling back to the flat view");
            gpu.xr = None;
            gpu.vr_pair = None;
            game.vr = None;
            XrGate::NoVr
        }
        xr::Frame::Open {
            should_render: false,
            ..
        } => {
            session.skip_frame();
            game.vr = None;
            XrGate::SkipFrame
        }
        xr::Frame::Open {
            should_render: true,
            eyes,
        } => {
            if game.vr_recentre {
                game.vr_recentre = false;
                session.recentre(eyes[0]);
            }
            game.vr = Some(VrView { eyes });
            XrGate::Rendering
        }
    }
}

/// Read a small patch of `texture` (a `wgpu::Bgra8UnormSrgb`-shaped
/// image) back to the CPU as tightly-packed BGRA8 rows (the
/// `COPY_BYTES_PER_ROW_ALIGNMENT` padding stripped) — the eye-order
/// self-check's one readback, shared by its ink-presence and glyph-
/// shape checks. `None` on any device or mapping failure. Blocks
/// (`device.poll`); only ever called from that one-shot self-check.
#[cfg(not(target_arch = "wasm32"))]
fn readback_patch(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    origin: (u32, u32),
    size: (u32, u32),
) -> Option<Vec<u8>> {
    let unpadded = size.0 * 4;
    let bytes_per_row =
        unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("eye order self-check readback"),
        size: (bytes_per_row * size.1) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("eye order self-check copy"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: origin.0,
                y: origin.1,
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(size.1),
            },
        },
        wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    if device.poll(wgpu::PollType::wait_indefinitely()).is_err() {
        return None;
    }
    let Ok(Ok(())) = rx.recv() else {
        return None;
    };
    let Ok(mapped) = slice.get_mapped_range() else {
        return None;
    };
    let mut out = Vec::with_capacity((size.0 * size.1 * 4) as usize);
    for row in 0..size.1 {
        let start = (row * bytes_per_row) as usize;
        let end = start + (size.0 * 4) as usize;
        out.extend_from_slice(&mapped[start..end]);
    }
    drop(mapped);
    buffer.unmap();
    Some(out)
}

/// Bright hologram cyan ([0.45, 0.92, 1.0, 0.96], hud.wgsl) — green and
/// blue well above red — is distinct enough from dash metal or a
/// starfield that a readback patch needs no exact colour match to tell
/// ink from background.
#[cfg(not(target_arch = "wasm32"))]
fn is_ink(bgra: &[u8]) -> bool {
    let (b, g, r) = (bgra[0] as u32, bgra[1] as u32, bgra[2] as u32);
    g > 140 && b > 140 && r + 40 < g
}

/// The fraction of a tightly-packed BGRA8 patch (from [`readback_patch`])
/// that reads as ink — "ink is present here," not a glyph classifier.
#[cfg(not(target_arch = "wasm32"))]
fn ink_fraction(bgra: &[u8]) -> f32 {
    let total = bgra.len() / 4;
    if total == 0 {
        return 0.0;
    }
    let lit = bgra.chunks_exact(4).filter(|px| is_ink(px)).count();
    lit as f32 / total as f32
}

/// Downsamples a tightly-packed BGRA8 patch into a `GLYPH_W x GLYPH_H`
/// grid of "is this cell mostly ink" bools, in the same bit layout
/// `farfall_render::text::glyph` returns (bit 0 = leftmost column) — a
/// coarse box downsample, not a geometrically exact resample (the patch
/// is not precisely registered to the glyph's own cell grid, only
/// dominated by it), which [`glyph_match_score`] compares against the
/// font's own reference shapes.
#[cfg(not(target_arch = "wasm32"))]
fn glyph_shape(bgra: &[u8], size: (u32, u32)) -> [u8; farfall_render::text::GLYPH_H] {
    use farfall_render::text::{GLYPH_H, GLYPH_W};
    let mut out = [0u8; GLYPH_H];
    if size.0 == 0 || size.1 == 0 {
        return out;
    }
    let cell_w = (size.0 as f32 / GLYPH_W as f32).max(1.0);
    let cell_h = (size.1 as f32 / GLYPH_H as f32).max(1.0);
    for (row, cell) in out.iter_mut().enumerate() {
        let y0 = (row as f32 * cell_h) as u32;
        let y1 = (((row + 1) as f32 * cell_h).ceil() as u32)
            .min(size.1)
            .max(y0 + 1);
        for col in 0..GLYPH_W {
            let x0 = (col as f32 * cell_w) as u32;
            let x1 = (((col + 1) as f32 * cell_w).ceil() as u32)
                .min(size.0)
                .max(x0 + 1);
            let mut lit = 0u32;
            let mut total = 0u32;
            for y in y0..y1 {
                let row_start = (y * size.0 * 4) as usize;
                for x in x0..x1 {
                    let idx = row_start + (x * 4) as usize;
                    if idx + 4 <= bgra.len() {
                        total += 1;
                        if is_ink(&bgra[idx..idx + 4]) {
                            lit += 1;
                        }
                    }
                }
            }
            if total > 0 && (lit as f32 / total as f32) > 0.15 {
                *cell |= 1 << col;
            }
        }
    }
    out
}

/// How many of the `GLYPH_W * GLYPH_H` cells agree (both lit or both
/// unlit) between an observed shape ([`glyph_shape`]) and a reference
/// one (`farfall_render::text::glyph`) — an exact match scores
/// `GLYPH_W * GLYPH_H`; a exact mismatch on every cell scores 0.
#[cfg(not(target_arch = "wasm32"))]
fn glyph_match_score(
    observed: [u8; farfall_render::text::GLYPH_H],
    reference: [u8; farfall_render::text::GLYPH_H],
) -> u32 {
    observed
        .iter()
        .zip(reference.iter())
        .map(|(o, r)| {
            (0..farfall_render::text::GLYPH_W as u32)
                .filter(|c| (o >> c) & 1 == (r >> c) & 1)
                .count() as u32
        })
        .sum()
}

/// FARFALL_VR_LABEL=1: on the first labelled composite, read back both
/// of each eye's corners (its own outer one, and the inner one where
/// the OTHER eye's mark would land if the two ever got crossed) and
/// confirm: ink in the outer corner, none in the inner one, and the
/// outer corner's own shape reads as the right letter — L for eye 0, R
/// for eye 1 — not merely "some cyan ink somewhere near here," which
/// passed a synth capture (e80e9af) that actually showed the SAME
/// oversized "R" landing in both eyes at eye 1's own position (a
/// uniform-buffer race this branch also fixes — see `VrPair::
/// label_hud`). Runs identically for a real or synthetic headset,
/// which is the whole point: this class of bug is now caught by a
/// bench row, on any machine, before it reaches a human. Logs "VR: eye
/// order self-check OK" or FAILED, and exactly what failed; under
/// `FARFALL_BENCH=1` a failure exits 9.
#[cfg(not(target_arch = "wasm32"))]
fn eye_order_self_check(gpu: &Gpu, eyes: &[VrEye; 2]) {
    let Some(session) = gpu.xr.as_ref() else {
        return;
    };
    // A validation error in here (a usage-flag mismatch this class has
    // now hit twice: the mirror-pair crash, then this self-check's own
    // first real-runtime run) must read as a failed self-check, not
    // crash the session it exists to protect — wgpu panics on an
    // unhandled validation error, so every device call this function
    // makes is inside this scope.
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let eye_size = session.eye_size();
    // Plain screen-space geometry stands in for the canopy warp: a
    // patch generously larger than the mark's own rectangle (margin
    // 1.6x per axis) absorbs that approximation's own error and the
    // per-eye parallax shift without needing to invert the warp.
    let to_px = |nx: f32, ny: f32| -> (i64, i64) {
        (
            ((nx + 1.0) / 2.0 * eye_size.0 as f32) as i64,
            ((1.0 - ny) / 2.0 * eye_size.1 as f32) as i64,
        )
    };
    let patch_for = |eye: usize, corner_ndc: [f32; 2], px: f32| -> ((u32, u32), (u32, u32)) {
        let width_ndc = farfall_render::text::ADVANCE as f32 * px;
        let height_ndc = farfall_render::text::GLYPH_H as f32 * px;
        let cx = corner_ndc[0] + width_ndc / 2.0;
        let cy = corner_ndc[1] - height_ndc / 2.0;
        let margin = 1.6;
        let (x0, y0) = to_px(
            cx - width_ndc / 2.0 * margin,
            cy + height_ndc / 2.0 * margin,
        );
        let (x1, y1) = to_px(
            cx + width_ndc / 2.0 * margin,
            cy - height_ndc / 2.0 * margin,
        );
        let clamp_x = |v: i64| v.clamp(0, eye_size.0 as i64 - 1) as u32;
        let clamp_y = |v: i64| v.clamp(0, eye_size.1 as i64 - 1) as u32;
        let (ox, oy) = (clamp_x(x0.min(x1)), clamp_y(y0.min(y1)));
        let (ex, ey) = (clamp_x(x0.max(x1)), clamp_y(y0.max(y1)));
        let _ = eye; // corner_ndc already carries the eye-specific geometry
        ((ox, oy), ((ex - ox).max(1), (ey - oy).max(1)))
    };
    let mut ok = true;
    for eye in 0..2 {
        let Some(texture) = session.acquired_eye_texture(eye) else {
            ok = false;
            continue;
        };
        let (outer_anchor, px) = label_geometry(eye, eyes);
        // The inner corner: where the OTHER eye's own mark would land
        // in THIS eye's image if the two were ever crossed — mirroring
        // label_geometry's own eye-1-from-eye-0 formula.
        let width_ndc = farfall_render::text::ADVANCE as f32 * px;
        let inner_anchor = [-outer_anchor[0] - width_ndc, outer_anchor[1]];
        let expected = if eye == 0 { 'L' } else { 'R' };
        let wrong = if eye == 0 { 'R' } else { 'L' };

        let (origin, size) = patch_for(eye, outer_anchor, px);
        match readback_patch(&gpu.device, &gpu.queue, texture, origin, size) {
            Some(bytes) => {
                let fraction = ink_fraction(&bytes);
                if fraction < 0.01 {
                    ok = false;
                    log::error!(
                        "VR: eye order self-check: eye {eye}'s own outer corner has no ink \
                         (lit fraction {fraction:.4})"
                    );
                } else {
                    let shape = glyph_shape(&bytes, size);
                    let want = glyph_match_score(shape, farfall_render::text::glyph(expected));
                    let dont = glyph_match_score(shape, farfall_render::text::glyph(wrong));
                    if want <= dont {
                        ok = false;
                        log::error!(
                            "VR: eye order self-check: eye {eye}'s outer corner reads as \
                             '{wrong}' (score {dont}), not '{expected}' (score {want})"
                        );
                    }
                }
            }
            None => {
                ok = false;
                log::error!("VR: eye order self-check: eye {eye}'s outer corner readback failed");
            }
        }

        let (origin, size) = patch_for(eye, inner_anchor, px);
        if let Some(bytes) = readback_patch(&gpu.device, &gpu.queue, texture, origin, size) {
            let fraction = ink_fraction(&bytes);
            if fraction > 0.01 {
                ok = false;
                log::error!(
                    "VR: eye order self-check: eye {eye}'s INNER corner has ink \
                     (lit fraction {fraction:.4}) — the other eye's mark may have \
                     landed here too"
                );
            }
        }
    }
    if let Some(e) = pollster::block_on(scope.pop()) {
        ok = false;
        log::error!("VR: eye order self-check: a device validation error, not a swapped eye: {e}");
    }
    if ok {
        log::info!("VR: eye order self-check OK");
    } else {
        log::error!("VR: eye order self-check FAILED");
        if gpu.cfg.bench {
            std::process::exit(9);
        }
    }
}

/// The crop-and-mirror step native VR needs and WebXR gets for free from
/// the browser's own compositor (`web/xr.js`): cut each eye's true
/// asymmetric field back out of the wide symmetric pair just rendered
/// (`xr::cutout_uv`) into that eye's OpenXR swapchain image, then mirror
/// the left eye's half — letterboxed to the window's own shape — into
/// `window_view`, and end the runtime's frame. Panics only on invariants
/// this module itself maintains (`gpu.xr`/`gpu.vr_pair`/`game.vr` are
/// all set together, by `xr_begin_frame`).
#[cfg(not(target_arch = "wasm32"))]
fn xr_composite(gpu: &mut Gpu, game: &Game, window_view: &wgpu::TextureView) {
    let label = gpu.cfg.vr_label;
    let mirror_pair = gpu.cfg.vr_mirror_pair;
    let (ww, wh) = (gpu.config.width, gpu.config.height);
    // The swapchain's own per-eye size — what MIRROR=pair actually
    // shows, since it sources from the (post-crop) swapchain images
    // themselves, not the wider render. Distinct from the render's own
    // per-eye size (the default single-eye mirror's own letterbox,
    // since that mode shows the render's un-cropped half): the render is
    // inflated by the hull-vs-true ratio (eye_render_size) and can have
    // a different aspect ratio than the swapchain outright.
    let swapchain_eye_size = gpu
        .xr
        .as_ref()
        .expect("xr_composite implies gpu.xr")
        .eye_size();
    let pair = gpu
        .vr_pair
        .as_mut()
        .expect("xr_composite is only called with vr_pair set");
    let render_eye_size = (pair.size.0 / 2, pair.size.1);
    let eyes = game
        .vr
        .as_ref()
        .expect("xr_composite is only called with game.vr set")
        .eyes;
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("xr composite"),
        });
    let mut swapchain_views = Vec::with_capacity(2);
    {
        let session = gpu.xr.as_mut().expect("xr_composite implies gpu.xr");
        for eye in 0..2 {
            swapchain_views.push(session.acquire_eye(eye).clone());
        }
    }
    for (eye, target) in swapchain_views.iter().enumerate() {
        let rect = xr::pair_source_rect(eye, &eyes);
        pair.to_swapchain.update(
            &gpu.queue,
            &farfall_render::blit_xr::XrBlitUniforms::new(rect),
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("xr eye crop"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pair.to_swapchain.draw(&mut pass);
        // FARFALL_VR_LABEL=1: stamped into the same pass, onto the same
        // swapchain image the headset itself will show — proves the eye
        // order on the headset side, not only the desktop mirror. A small
        // opaque glyph in the eye's own upper-outer corner, at the same
        // ~1 m glass depth as every other overlay (VR_HUD_DISTANCE_M):
        // no plate, no full-eye tint, and a genuine per-eye parallax
        // shift, not a coincidence of matching screen position — a
        // zero-disparity full-eye label is itself the "close obscuring
        // plane" this exists to rule out.
        if label {
            pair.label_bitmap.clear();
            pair.label_bitmap
                .draw(0, 0, if eye == 0 { "L" } else { "R" });
            let aspect = swapchain_eye_size.0 as f32 / swapchain_eye_size.1.max(1) as f32;
            let (anchor, px) = label_geometry(eye, &eyes);
            let mut block = farfall_render::hud::HudBlock::glass(
                anchor,
                px,
                aspect,
                swapchain_eye_size.1 as f32,
            );
            block.no_backdrop = true; // ink on the glass, no plate behind it
            pair.label_hud[eye].update(&gpu.queue, &pair.label_bitmap, &block);
            pair.label_hud[eye].draw(&mut pass);
        }
    }
    if mirror_pair {
        // Sourced from the swapchain images themselves — post-crop, and
        // post-label if FARFALL_VR_LABEL is on — so the desktop mirror
        // shows exactly what each eye's display will, provable before
        // anyone wears the headset (SPEC §5.3).
        let half_w = (ww / 2).max(1);
        for (eye, target) in swapchain_views.iter().enumerate() {
            pair.mirror_swap.rebind(&gpu.device, target);
            pair.mirror_swap.update(
                &gpu.queue,
                &farfall_render::blit_xr::XrBlitUniforms::new([0.0, 0.0, 1.0, 1.0]),
            );
            let (x, y, w, h) = xr::letterbox((half_w, wh), swapchain_eye_size);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xr mirror pair"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: window_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if eye == 0 {
                            wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_viewport(
                (eye as u32 * half_w + x) as f32,
                y as f32,
                w as f32,
                h as f32,
                0.0,
                1.0,
            );
            pair.mirror_swap.draw(&mut pass);
        }
    } else {
        // No further crop: the left eye's half, stretched to fill a
        // letterboxed viewport of its own aspect within the window.
        pair.to_window.update(
            &gpu.queue,
            &farfall_render::blit_xr::XrBlitUniforms::new([0.0, 0.0, 0.5, 1.0]),
        );
        let (x, y, w, h) = xr::letterbox((ww, wh), render_eye_size);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("xr mirror"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: window_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
        pair.to_window.draw(&mut pass);
    }
    gpu.queue.submit([encoder.finish()]);
    if label && !gpu.eye_order_checked {
        gpu.eye_order_checked = true;
        eye_order_self_check(gpu, &eyes);
    }
    // Present (below, in redraw) must never block the XR frame: end_frame
    // — the runtime's own submission — always runs here, before redraw
    // reaches gpu.queue.present(frame).
    gpu.xr
        .as_mut()
        .expect("xr_composite implies gpu.xr")
        .end_frame();
}

/// One frame: tick the sim, draw the world into the scene target, upscale
/// and lay the HUD over it, present. In VR the pair is drawn side by side
/// into one surface, left eye first, each from its own eye.
fn redraw(
    gpu: &mut Gpu,
    game: &mut Game,
    audio: Option<&Audio>,
    event_loop: Option<&ActiveEventLoop>,
) {
    let tick_start = Instant::now();
    if gpu.cfg.bench && gpu.cfg.bench_spin > 0 {
        // The spin: a full turn of the head over the bench, so
        // the captures look every way round the cabin.
        let t = game.started.elapsed().as_secs_f64();
        let yaw = std::f64::consts::TAU * t / gpu.cfg.bench_seconds.max(0.1);
        game.look.aim_free(yaw as f32, 0.0);
    }
    if let Some(at) = gpu.cfg.bench_warp_at {
        if gpu.cfg.bench && game.started.elapsed().as_secs_f64() >= at {
            gpu.cfg.bench_warp_at = None;
            game.engage_warp();
        }
    }
    // Native VR (SPEC §5.3): wait for and open the runtime's frame before
    // the sim ticks, so the predicted pose it hands back drives this
    // frame — the same reason the WebXR bridge sets `game.vr` before
    // `redraw` is called at all.
    #[cfg(not(target_arch = "wasm32"))]
    if matches!(xr_begin_frame(gpu, game), XrGate::SkipFrame) {
        gpu.window.request_redraw();
        return;
    }
    game.tick();
    if let Some(audio) = &audio {
        audio.set(&game.audio_levels());
    }

    // Acquiring a swapchain image BLOCKS until one is free, which
    // when the GPU is the bottleneck means blocking for roughly a
    // GPU frame. Timing it as CPU work made the readout claim 37 ms
    // of CPU against a real figure of half a millisecond — the
    // renderer's own instrument accusing the wrong half of the
    // machine. It is measured separately and reported as WAIT.
    let sim_seconds = tick_start.elapsed().as_secs_f64();
    let acquire_start = Instant::now();
    let frame = match gpu.surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(frame) => frame,
        wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
            // Benchmarks must neither hang nor lie when occluded:
            // the window being buried is no reason to skip the
            // capture, because the scene target is offscreen and
            // needs no swapchain. Render it headless and save.
            // (This is the seed of the golden-image harness: a
            // frame produced with no visible window at all.)
            if gpu.cfg.bench {
                let t = game.started.elapsed().as_secs_f64();
                if gpu.bench_capture_due(t) {
                    if gpu
                        .scene
                        .ensure(&gpu.device, gpu.config.width, gpu.config.height)
                    {
                        gpu.rebind_scene();
                    }
                    let aspect = gpu.config.width as f32 / gpu.config.height as f32;
                    let pose = game.pose(aspect);
                    let cam = pose.cam;
                    gpu.passes.starfield.update(
                        &gpu.queue,
                        &FrameUniforms::from_camera(&cam)
                            .with_occluder(
                                (DVec3::ZERO - game.eye_m(&pose)).as_vec3(),
                                game.params.planet.radius_m as f32,
                            )
                            .with_star_stretch(game.speed_look()),
                    );
                    gpu.passes
                        .planet
                        .update(&gpu.queue, &game.planet_uniforms(&pose));
                    gpu.passes.bodies.update(
                        &gpu.queue,
                        &game.bodies_uniforms(&pose, gpu.scene.size().1 as f32),
                    );
                    gpu.passes
                        .belt
                        .update(&gpu.queue, &game.belt_uniforms(&pose));
                    gpu.passes
                        .mimic
                        .update(&gpu.queue, &game.mimic_uniforms(&pose));
                    gpu.passes
                        .heli
                        .update(&gpu.queue, &game.heli_uniforms(&pose));
                    gpu.passes
                        .tracer
                        .update(&gpu.queue, &game.tracer_uniforms(&pose));
                    gpu.passes
                        .debris
                        .update(&gpu.queue, &game.debris_uniforms(&pose));
                    gpu.passes
                        .scar
                        .update(&gpu.queue, &game.scar_uniforms(&pose));
                    gpu.passes
                        .sight
                        .update(&gpu.queue, &game.sight_uniforms(&pose));
                    let (altitude_m, _) = game.altitude_vspeed();
                    gpu.update_instruments(
                        game,
                        &cam,
                        aspect,
                        altitude_m as f32,
                        pose.eye_ship.as_vec3(),
                    );
                    // The capture should show what the pilot
                    // sees, text included. The HUD pipeline is
                    // single-sample (it draws in the present
                    // pass), so it can only join a 1x scene.
                    let capture_text = gpu.cfg.msaa == 1;
                    if capture_text && (game.map_open() || game.mini_map_shown()) {
                        gpu.map
                            .update(&gpu.queue, &game.map_uniforms(aspect, cam.time_s));
                    }
                    if capture_text && game.design {
                        gpu.text.clear();
                        for (row, line) in game.design_text(aspect).iter().enumerate() {
                            gpu.text.draw_line(0, row, line);
                        }
                    } else if capture_text && game.map_open() {
                        game.map_panel.render(&mut gpu.text, &game.settings);
                    } else if capture_text && game.bay_open() {
                        game.render_bay_card(&mut gpu.text);
                        let sh = gpu.scene.size().1 as f32;
                        let px = panel::px_canopy(sh) * game.text_fov_scale(&cam);
                        let anchor = game.bay_text_anchor(aspect, panel::block_ndc(PANEL_COLS, px));
                        gpu.hologram.update(
                            &gpu.queue,
                            &game.hologram_uniforms(aspect, cam.time_s, sh, (anchor, px)),
                        );
                        gpu.pointer
                            .update(&gpu.queue, &game.pointer_uniforms(aspect, cam.time_s));
                    } else if capture_text && game.card_open {
                        card::render(&mut gpu.text, &game.settings.bindings);
                    } else if capture_text && game.menu.open {
                        game.render_menu(&mut gpu.text);
                    } else if capture_text {
                        gpu.text.clear();
                        gpu.text.draw(0, 0, "HEADLESS CAPTURE");
                        gpu.text.draw_line(
                            0,
                            1,
                            &format!(
                                "ALT {}",
                                farfall_render::gauge::length_text(altitude_m as f32)
                            ),
                        );
                        gpu.text.draw_line(
                            0,
                            2,
                            &format!("VEL {:.0}M/S", game.state.ship.vel_mps.length()),
                        );
                    }
                    if capture_text {
                        // Whatever text this frame has — a panel,
                        // the design card or the bench readout —
                        // goes to the GPU the same way.
                        let (_, sh) = gpu.scene.size();
                        let px_canopy = panel::px_canopy(sh as f32) * game.text_fov_scale(&cam);
                        gpu.hud.update(
                            &gpu.queue,
                            &gpu.text,
                            &game.hud_block(&cam, px_canopy, sh as f32),
                        );
                    }
                    gpu.update_holo(game, aspect);
                    let mut encoder =
                        gpu.device
                            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                label: Some("headless"),
                            });
                    let thermal_in = game.thermal_inputs(game.frame_dt);
                    gpu.passes.plasma.update(
                        &gpu.queue,
                        &PlasmaUniforms::new(&cam, thermal_in.vel_ship_mps, game.head()),
                    );
                    gpu.passes
                        .thermal
                        .step(&gpu.queue, &mut encoder, &thermal_in);
                    {
                        let (sw, sh) = gpu.scene.size();
                        gpu.passes.cabin.ensure(&gpu.device, sw, sh);
                        let (cu, bu) = game.cabin_uniforms(&cam);
                        gpu.passes.cabin.update(&gpu.queue, &mut encoder, &cu, &bu);
                    }
                    gpu.passes.trajectory.update(
                        &gpu.queue,
                        &TrajectoryUniforms::new(
                            &cam,
                            &game.trajectory_world(game.eye_m(&pose)),
                            TRAJECTORY_HORIZON_S,
                            game.trajectory_vis,
                            gpu.scene.size().1 as f32,
                            game.marks(game.eye_m(&pose)),
                        ),
                    );
                    gpu.passes
                        .shield
                        .update(&gpu.queue, &game.shield_uniforms(&pose));
                    gpu.passes
                        .ghost
                        .update(&gpu.queue, &game.ghost_uniforms(&pose));
                    gpu.passes.jet.update(&gpu.queue, &game.jet_uniforms(&pose));
                    gpu.update_post(game, aspect, cam.time_s);
                    let du = game.dust_uniforms(&pose, gpu.scene.size().1 as f32);
                    gpu.passes.dust.update(&gpu.queue, &du);
                    let wu = game.wind_uniforms(&pose, gpu.scene.size().1 as f32);
                    gpu.passes.wind.update(&gpu.queue, &wu);
                    {
                        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("scene headless"),
                            color_attachments: &[Some(gpu.scene.world_attachment())],
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        });
                        gpu.passes.starfield.draw(&mut pass);
                        gpu.passes.bodies.draw(&mut pass);
                        gpu.passes.planet.draw(&mut pass);
                        gpu.passes.belt.draw(&mut pass);
                        gpu.passes.mimic.draw(&mut pass);
                        gpu.passes.scar.draw(&mut pass);
                        gpu.passes.debris.draw(&mut pass);
                        gpu.passes.tracer.draw(&mut pass);
                        gpu.passes.dust.draw_space(&mut pass, &du);
                        gpu.passes.wind.draw(&mut pass, &wu);
                        gpu.passes.jet.draw(&mut pass);
                        if !game.exterior_view() {
                            gpu.passes.plasma.draw(&mut pass, &gpu.passes.thermal);
                        }
                        gpu.passes.trajectory.draw(&mut pass);
                        gpu.passes.shield.draw(&mut pass);
                        gpu.passes.ghost.draw(&mut pass);
                    }
                    {
                        // The picture, then the ship over it.
                        let mut pass = gpu.post.begin_ship_pass(&mut encoder, &gpu.scene, true);
                        if !game.exterior_view() {
                            // The horizon and its ladder are at infinity:
                            // the dash hides what falls below its sill.
                            gpu.passes.horizon.draw(&mut pass);
                            gpu.passes.cabin.draw(&mut pass);
                            gpu.passes.dust.draw_cabin(&mut pass, &du);
                            gpu.passes.gauge.draw_within(
                                &mut pass,
                                gpu.dial_rects.get()[0],
                                gpu.scene.size(),
                            );
                            gpu.passes.alt_gauge.draw_within(
                                &mut pass,
                                gpu.dial_rects.get()[1],
                                gpu.scene.size(),
                            );
                            gpu.passes.g_gauge.draw_within(
                                &mut pass,
                                gpu.dial_rects.get()[2],
                                gpu.scene.size(),
                            );
                            gpu.passes.gvec.draw_within(
                                &mut pass,
                                gpu.dial_rects.get()[3],
                                gpu.scene.size(),
                            );
                            gpu.passes.gyro.draw_within(
                                &mut pass,
                                gpu.dial_rects.get()[4],
                                gpu.scene.size(),
                            );
                            gpu.passes.guide.draw(&mut pass);
                            gpu.passes.guide.draw(&mut pass);
                            gpu.passes.sight.draw(&mut pass);
                        }
                        gpu.passes.holo.draw(&mut pass);
                        if capture_text && (game.map_open() || game.mini_map_shown()) {
                            gpu.map.draw(&mut pass);
                        }
                        if capture_text && game.bay_open() {
                            gpu.hologram.draw(&mut pass);
                        }
                        if capture_text {
                            gpu.hud.draw(&mut pass);
                            gpu.pointer.draw(&mut pass);
                        }
                    }
                    let capture = gpu.scene.colour_texture().map(|tex| {
                        let path =
                            std::env::temp_dir().join(format!("farfall-{:.0}.png", t * 1000.0));
                        Capture::record(&gpu.device, &mut encoder, tex, path)
                    });
                    gpu.queue.submit([encoder.finish()]);
                    if let Some(capture) = capture {
                        let bgra = matches!(
                            gpu.scene.format(),
                            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
                        );
                        match capture.save(&gpu.device, bgra) {
                            Ok(path) => {
                                log::info!(
                                    "screenshot: {} ({:.1} fps at capture)",
                                    path.display(),
                                    gpu.perf.stats.smoothed_fps()
                                )
                            }
                            Err(e) => log::warn!("headless capture failed: {e}"),
                        }
                    }
                }
                if t > gpu.cfg.bench_seconds {
                    if let Some(el) = event_loop {
                        bench_save_world(game, gpu.cfg.bench);
                        el.exit();
                    }
                }
            }
            gpu.window.request_redraw();
            return;
        }
        other => {
            log::warn!("surface: {other:?}, reconfiguring");
            gpu.surface.configure(&gpu.device, &gpu.config);
            gpu.window.request_redraw();
            return;
        }
    };
    let wait_seconds = acquire_start.elapsed().as_secs_f64();
    let frame_view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    // Native VR draws the pair offscreen (its own eye size, decoupled
    // from the mirror window's) and crops+mirrors it afterward; flat and
    // WebXR draw straight into the surface, as they always have.
    // `xr_eye_size`/`vr_pair_view` are `None` on every platform without a
    // native session (always, on the web build), so this needs no cfg of
    // its own.
    let native_vr = gpu.xr_eye_size().is_some() && game.vr.is_some();
    // In VR the surface is the stereo pair, side by side; each eye is a
    // full draw of its own, from its own seat, into its own half. The
    // render itself is sized bigger than the runtime's own recommended
    // size (eye_render_size, SPEC §5.3: canted lenses mean the symmetric
    // hull this engine actually renders is wider than the true field,
    // so undersizing it here would soften everything on the crop) —
    // ensure() only actually reallocates the one time this changes,
    // normally once, the first frame real tangents arrive.
    let eyes: u32 = if game.vr.is_some() { 2 } else { 1 };
    let (ew, eh) = if native_vr {
        let tans = {
            let e = &game.vr.as_ref().expect("native_vr implies game.vr").eyes;
            [e[0].tan, e[1].tan]
        };
        gpu.ensure_vr_render_size(tans)
            .expect("native_vr implies ensure_vr_render_size")
    } else {
        (gpu.config.width / eyes, gpu.config.height)
    };
    let view = if native_vr {
        gpu.vr_pair_view()
            .expect("vr_pair is created alongside gpu.xr")
            .clone()
    } else {
        frame_view.clone()
    };
    let mut cpu_seconds = sim_seconds;
    let mut captured: Option<Capture> = None;
    for eye in 0..eyes {
        game.vr_eye = eye as usize;
        let encode_start = Instant::now();
        if gpu.scene.ensure(&gpu.device, ew, eh) {
            // The scene textures were recreated; a bind group still
            // pointing at the old view would sample a destroyed
            // resource.
            gpu.rebind_scene();
            gpu.perf.stats.skip_next_frame();
        }

        let aspect = ew as f32 / eh as f32;
        let pose = game.pose(aspect);
        let cam = pose.cam;
        game.update_drag(&cam);
        gpu.passes.starfield.update(
            &gpu.queue,
            &FrameUniforms::from_camera(&cam)
                .with_occluder(
                    (DVec3::ZERO - game.eye_m(&pose)).as_vec3(),
                    game.params.planet.radius_m as f32,
                )
                .with_star_stretch(game.speed_look()),
        );
        gpu.passes
            .planet
            .update(&gpu.queue, &game.planet_uniforms(&pose));
        gpu.passes.bodies.update(
            &gpu.queue,
            &game.bodies_uniforms(&pose, gpu.scene.size().1 as f32),
        );
        gpu.passes
            .belt
            .update(&gpu.queue, &game.belt_uniforms(&pose));
        gpu.passes
            .mimic
            .update(&gpu.queue, &game.mimic_uniforms(&pose));
        gpu.passes
            .heli
            .update(&gpu.queue, &game.heli_uniforms(&pose));
        gpu.passes
            .tracer
            .update(&gpu.queue, &game.tracer_uniforms(&pose));
        gpu.passes
            .debris
            .update(&gpu.queue, &game.debris_uniforms(&pose));
        gpu.passes
            .scar
            .update(&gpu.queue, &game.scar_uniforms(&pose));
        gpu.passes
            .sight
            .update(&gpu.queue, &game.sight_uniforms(&pose));
        let thermal_in = game.thermal_inputs(game.frame_dt);
        gpu.passes.plasma.update(
            &gpu.queue,
            &PlasmaUniforms::new(&cam, thermal_in.vel_ship_mps, game.head()),
        );
        gpu.passes.trajectory.update(
            &gpu.queue,
            &TrajectoryUniforms::new(
                &cam,
                &game.trajectory_world(game.eye_m(&pose)),
                TRAJECTORY_HORIZON_S,
                game.trajectory_vis,
                gpu.scene.size().1 as f32,
                game.marks(game.eye_m(&pose)),
            ),
        );
        gpu.passes
            .shield
            .update(&gpu.queue, &game.shield_uniforms(&pose));
        gpu.passes
            .ghost
            .update(&gpu.queue, &game.ghost_uniforms(&pose));
        gpu.passes.jet.update(&gpu.queue, &game.jet_uniforms(&pose));
        let du = game.dust_uniforms(&pose, gpu.scene.size().1 as f32);
        gpu.passes.dust.update(&gpu.queue, &du);
        let wu = game.wind_uniforms(&pose, gpu.scene.size().1 as f32);
        gpu.passes.wind.update(&gpu.queue, &wu);
        let (altitude_m, _) = game.altitude_vspeed();
        gpu.update_instruments(
            game,
            &cam,
            aspect,
            altitude_m as f32,
            pose.eye_ship.as_vec3(),
        );

        // Scale the readout with the surface so it keeps the same
        // apparent size on a retina fullscreen and a small window;
        // the size is chosen in pixels and expressed in canopy units.
        let px_canopy = panel::px_canopy(eh as f32) * game.text_fov_scale(&cam);
        {
            gpu.update_post(game, aspect, cam.time_s);
            gpu.map
                .update(&gpu.queue, &game.map_uniforms(aspect, cam.time_s));
        }
        game.press_flash = (game.press_flash - game.frame_dt * 5.0).max(0.0);
        if game.bay_open() {
            // The bay turns by itself when the hand is off it.
            game.bay_view.tick(game.frame_dt, game.settings.bay_spin);
            let sh = eh as f32;
            let px = panel::px_canopy(sh) * game.text_fov_scale(&cam);
            let anchor = game.bay_text_anchor(aspect, panel::block_ndc(PANEL_COLS, px));
            gpu.hologram.update(
                &gpu.queue,
                &game.hologram_uniforms(aspect, cam.time_s, sh, (anchor, px)),
            );
        }
        gpu.pointer
            .update(&gpu.queue, &game.pointer_uniforms(aspect, cam.time_s));
        gpu.hud.update(
            &gpu.queue,
            &gpu.text,
            &game.hud_block(&cam, px_canopy, eh as f32),
        );

        gpu.update_holo(game, aspect);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        // Pass 0: advance the hull heat field (64x64, on the GPU).
        gpu.passes
            .thermal
            .step(&gpu.queue, &mut encoder, &thermal_in);
        // Pass 0b: the cabin, at its own size.
        if gpu.cfg.draws("cockpit") && game.settings.cockpit_frame {
            let (sw, sh) = gpu.scene.size();
            gpu.passes.cabin.ensure(&gpu.device, sw, sh);
            let (cu, bu) = game.cabin_uniforms(&cam);
            let cu = cu.with_eye(pose.eye_ship.as_vec3());
            gpu.passes.cabin.update(&gpu.queue, &mut encoder, &cu, &bu);
        }
        {
            // Pass 1: the expensive world, at whatever scale is set, in
            // radiance.
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(gpu.scene.world_attachment())],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if gpu.cfg.draws("starfield") {
                gpu.passes.starfield.draw(&mut pass);
            }
            if gpu.cfg.draws("bodies") {
                gpu.passes.bodies.draw(&mut pass);
            }
            if gpu.cfg.draws("planet") {
                gpu.passes.planet.draw(&mut pass);
            }
            if gpu.cfg.draws("belt") {
                gpu.passes.belt.draw(&mut pass);
            }
            if gpu.cfg.draws("mimic") {
                gpu.passes.mimic.draw(&mut pass);
            }
            if gpu.cfg.draws("heli") {
                gpu.passes.heli.draw(&mut pass);
            }
            if gpu.cfg.draws("scar") {
                gpu.passes.scar.draw(&mut pass);
            }
            if gpu.cfg.draws("debris") {
                gpu.passes.debris.draw(&mut pass);
            }
            if gpu.cfg.draws("tracer") {
                gpu.passes.tracer.draw(&mut pass);
            }
            if gpu.cfg.draws("dust") {
                gpu.passes.dust.draw_space(&mut pass, &du);
            }
            if gpu.cfg.draws("wind") {
                gpu.passes.wind.draw(&mut pass, &wu);
            }
            if gpu.cfg.draws("jet") {
                gpu.passes.jet.draw(&mut pass);
            }
            if gpu.cfg.draws("plasma") && !game.exterior_view() {
                gpu.passes.plasma.draw(&mut pass, &gpu.passes.thermal);
            }
            if gpu.cfg.draws("trajectory") {
                gpu.passes.trajectory.draw(&mut pass);
            }
            if gpu.cfg.draws("shield") {
                gpu.passes.shield.draw(&mut pass);
            }
            if gpu.cfg.draws("ghost") {
                gpu.passes.ghost.draw(&mut pass);
            }
        }
        {
            // Pass 1b: the picture — bloom, exposure, tonemap and the
            // drive's distortion, done to the world — then the ship drawn
            // over it, so the dash and the dials never warp or bloom.
            let mut pass =
                gpu.post
                    .begin_ship_pass(&mut encoder, &gpu.scene, gpu.cfg.draws("bloom"));
            if gpu.cfg.draws("gauge") && !game.exterior_view() {
                // At infinity, so under the dash: the cabin covers what
                // falls below its sill. On the ship side, so it never blooms.
                gpu.passes.horizon.draw(&mut pass);
            }
            if gpu.cfg.draws("cockpit") && !game.exterior_view() {
                gpu.passes.cabin.draw(&mut pass);
                if gpu.cfg.draws("dust") {
                    gpu.passes.dust.draw_cabin(&mut pass, &du);
                }
            }
            if gpu.cfg.draws("gauge") && !game.exterior_view() {
                gpu.passes
                    .gauge
                    .draw_within(&mut pass, gpu.dial_rects.get()[0], gpu.scene.size());
                gpu.passes.alt_gauge.draw_within(
                    &mut pass,
                    gpu.dial_rects.get()[1],
                    gpu.scene.size(),
                );
                gpu.passes.g_gauge.draw_within(
                    &mut pass,
                    gpu.dial_rects.get()[2],
                    gpu.scene.size(),
                );
                gpu.passes
                    .gvec
                    .draw_within(&mut pass, gpu.dial_rects.get()[3], gpu.scene.size());
                gpu.passes
                    .gyro
                    .draw_within(&mut pass, gpu.dial_rects.get()[4], gpu.scene.size());
                gpu.passes.guide.draw(&mut pass);
                gpu.passes.sight.draw(&mut pass);
            }
            if gpu.cfg.draws("holo") {
                gpu.passes.holo.draw(&mut pass);
            }
        }
        {
            // Pass 2: upscale, then the HUD at native resolution.
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("present"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // The second eye lands beside the first, not over it.
                        load: if eye == 0 {
                            wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if eyes > 1 {
                pass.set_viewport((eye * ew) as f32, 0.0, ew as f32, eh as f32, 0.0, 1.0);
            }
            if gpu.cfg.draws("blit") {
                gpu.blit.draw(&mut pass);
            }
            if game.map_open() || game.mini_map_shown() {
                gpu.map.draw(&mut pass);
            }
            if game.bay_open() {
                gpu.hologram.draw(&mut pass);
            }
            if gpu.cfg.draws("hud") {
                gpu.hud.draw(&mut pass);
            }
            gpu.pointer.draw(&mut pass);
        }
        // Screenshot: recorded into the same command buffer, so it
        // captures exactly the frame that was just drawn. In native VR,
        // `frame` (the mirror window's own surface) is not drawn into
        // here at all — that happens in xr_composite, after this whole
        // per-eye loop — so a capture request is left standing and
        // handled there instead, once the labelled pair actually exists.
        let pending = if gpu.capture_requested && !native_vr {
            gpu.capture_requested = false;
            if gpu.scene.colour_texture().is_none() {
                log::warn!("capture skipped: scene target has no colour texture");
            }
            let path = std::env::temp_dir().join(format!(
                "farfall-{:.0}.png",
                game.started.elapsed().as_secs_f64() * 1000.0
            ));
            if capture_final() {
                Some(Capture::record(
                    &gpu.device,
                    &mut encoder,
                    &frame.texture,
                    path,
                ))
            } else {
                gpu.scene
                    .colour_texture()
                    .map(|tex| Capture::record(&gpu.device, &mut encoder, tex, path))
            }
        } else {
            None
        };

        gpu.queue.submit([encoder.finish()]);
        // Genuine CPU work: simulation, uniform packing, encoding.
        cpu_seconds += encode_start.elapsed().as_secs_f64();
        if pending.is_some() {
            captured = pending;
        }
    }

    // Native VR (SPEC §5.3): the pair just rendered above is a symmetric,
    // wider-than-needed view of each eye; crop each eye's true asymmetric
    // field back out of it into that eye's OpenXR swapchain image, then
    // mirror the left eye's half (letterboxed) into the actual window
    // surface, and hand the frame back to the runtime.
    #[cfg(not(target_arch = "wasm32"))]
    if native_vr {
        xr_composite(gpu, game, &frame_view);
    }
    // The VR capture xr_composite's own mirror-pair draw made possible:
    // the labelled pair (both eyes cropped, FARFALL_VR_LABEL honoured),
    // not a raw scene target — frame_view now genuinely holds it, unlike
    // at the point capture_requested is normally consumed above.
    #[cfg(not(target_arch = "wasm32"))]
    if native_vr && gpu.capture_requested {
        gpu.capture_requested = false;
        let path = std::env::temp_dir().join(format!(
            "farfall-{:.0}.png",
            game.started.elapsed().as_secs_f64() * 1000.0
        ));
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("vr capture"),
            });
        let capture = Capture::record(&gpu.device, &mut encoder, &frame.texture, path);
        gpu.queue.submit([encoder.finish()]);
        captured = Some(capture);
    }

    if let Some(capture) = captured {
        let bgra = matches!(
            gpu.scene.format(),
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        // The rate at this moment (smoothed over the recent frames), not the
        // run's average: a spin's eight captures each get their own number.
        // Per-frame render_ms=/headroom_ms= for the VR bench stamp (same
        // smoothed convention as the fps just above, not this exact
        // frame's own instantaneous value).
        let vr_frame_stamp = native_vr.then(|| {
            let render_fps = gpu.perf.render.smoothed_fps();
            let render_ms = if render_fps > 0.0 {
                1000.0 / render_fps
            } else {
                0.0
            };
            let headroom_ms = gpu
                .xr_display_refresh_hz()
                .map_or(0.0, |hz| 1000.0 / hz as f64 - render_ms);
            format!(" render_ms={render_ms:.3} headroom_ms={headroom_ms:.3}")
        });
        match capture.save(&gpu.device, bgra) {
            Ok(path) => log::info!(
                "screenshot: {} ({:.1} fps at capture{})",
                path.display(),
                gpu.perf.stats.smoothed_fps(),
                vr_frame_stamp.unwrap_or_default(),
            ),
            Err(e) => log::warn!("screenshot failed: {e}"),
        }
        // The readback blocks on the GPU; that frame's timing says
        // nothing about the renderer.
        gpu.perf.stats.skip_next_frame();
    }

    gpu.queue.present(frame);
    // render_ms (the VR bench stamp): CPU encode plus real GPU frame
    // time, measured by forcing the block gpu_sync already offers as an
    // opt-in profiling knob, timed here — chosen over new timestamp-query
    // infrastructure this session could not verify against real hardware,
    // and gpu_sync's blocking poll is already a proven, if normally
    // opt-in, way to make a CPU-side clock GPU-honest.
    let gpu_wait_seconds = if gpu.cfg.gpu_sync || native_vr {
        let start = Instant::now();
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        start.elapsed().as_secs_f64()
    } else {
        0.0
    };
    // A benchmark stops itself. Left running, it is a frozen window
    // that looks exactly like the game and answers no controls.
    if gpu.cfg.bench {
        let t = game.started.elapsed().as_secs_f64();
        // One automatic capture partway through, so a headless
        // benchmark leaves a picture of what it measured.
        if gpu.bench_capture_due(t) {
            gpu.capture_requested = true;
            log::info!("benchmark capture requested at t={t:.1}s");
        }
        if t > gpu.cfg.bench_seconds {
            log::info!("benchmark complete, exiting");
            if let Some(el) = event_loop {
                bench_save_world(game, gpu.cfg.bench);
                el.exit();
            }
        }
    }
    gpu.govern_scale(&game.settings, gpu.vr_fps_floor(game.settings.fps_floor));
    let render_seconds = native_vr.then_some(cpu_seconds + gpu_wait_seconds);
    gpu.frame_timing(
        cpu_seconds,
        wait_seconds,
        render_seconds,
        game.settings.fps_floor,
        &Readout {
            altitude_m: game.altitude_vspeed().0,
            speed_mps: game.state.ship.vel_mps.length(),
            assist: game.assist,
            show: game.settings.layout.shown(Instrument::Readout)
                || game.landing
                || game.on_ground(),
            wind: game.wind_readout(),
            collective: game.flying_heli().then(|| {
                heli::route_controls(game.input.controls(game.assist))
                    .thrust_body
                    .y
            }),
            landing: game
                .eva_text()
                .or_else(|| game.landing_text())
                .or_else(|| game.hold_text())
                .or_else(|| game.mimic_text())
                .or_else(|| game.strain_text())
                .or_else(|| game.arms_text())
                .or_else(|| game.haul_text()),
        },
    );
    if game.design {
        gpu.text.clear();
        let aspect = gpu.config.width as f32 / gpu.config.height as f32;
        for (row, line) in game.design_text(aspect).iter().enumerate() {
            gpu.text.draw_line(0, row, line);
        }
    } else if game.map_open() {
        gpu.text.clear();
        game.map_panel.render(&mut gpu.text, &game.settings);
    } else if game.bay_open() {
        game.render_bay_card(&mut gpu.text);
    } else if game.card_open {
        card::render(&mut gpu.text, &game.settings.bindings);
    } else if game.menu.open {
        gpu.text.clear();
        game.render_menu(&mut gpu.text);
    }
    gpu.window.request_redraw();
}

impl App {
    /// The stick, once a frame: its axes into the input, its buttons
    /// through [`stick::StickMap::pilot`] — their named binds in flight
    /// (shifted while SHIFT is held), the arrows/ENTER/ESC of whatever
    /// panel is up, the wizard's navigation while plain presses stay its
    /// data, or a KEYS row's waiting bind. The stick is a complete
    /// parallel path to the keyboard; stick.rs's module doc is the map.
    fn poll_stick(&mut self, event_loop: &ActiveEventLoop) {
        // The bench is hermetic: the pilot's plugged-in stick is not part
        // of the scene — a full-rail throttle once slam-fired the chaos
        // drive mid-bench and warp-streaked a whole sweep's captures. A
        // parked FARFALL_BENCH_DEMAND (set once at staging) is the only
        // stick a bench flies with, and this return leaves it standing.
        if self.gpu.as_ref().is_some_and(|g| g.cfg.bench) {
            return;
        }
        let Some(game) = self.game.as_mut() else {
            return;
        };
        let Some(sample) = game.stick.poll() else {
            game.menu.set_stick(None);
            game.input.set_stick([0.0; 6]);
            game.input.set_stick_held(false, false);
            game.stick_gestures.reset();
            game.stick_fire = false;
            return;
        };
        game.menu
            .set_stick(game.stick.device.as_ref().map(|d| (d.vid, d.pid)));
        // The device names the raw indices: a HOTAS 4 says TRIGGER and
        // ROCKER, anything else says B0 and AXIS 4.
        if let Some(d) = game.stick.device.as_ref() {
            let layout =
                if stick::Device::known_name(d.vid, d.pid).is_some_and(|n| n.contains("HOTAS 4")) {
                    stick::Layout::Hotas4
                } else {
                    stick::Layout::Generic
                };
            if game.settings.stick.layout != layout {
                game.settings.stick.layout = layout;
                game.settings.save();
            }
        }
        let (pressed, released) = game.stick.edges(sample);
        let map = game.settings.stick;
        let shift_held = map.shift.is_some_and(|b| sample.button(b));
        // What is up decides what a button means. The order is
        // key_input's: the card over everything, then the wizard.
        let surface = if game.card_open {
            stick::Surface::Panel
        } else if let Some(w) = game.wizard.as_mut() {
            // The wizard reads the stick as data — except while SHIFT
            // is held, which reserves everything for navigation.
            if !shift_held {
                w.feed(sample, &map);
            }
            stick::Surface::Wizard {
                listening: w.listening(),
            }
        } else if game.design {
            stick::Surface::Design
        } else if game.menu.open || game.map_panel.open || game.bay_panel.open {
            stick::Surface::Panel
        } else {
            stick::Surface::Flight
        };
        let wizard_up = matches!(surface, stick::Surface::Wizard { .. });
        if wizard_up || !map.enabled {
            game.input.set_stick([0.0; 6]);
            game.input.set_stick_held(false, false);
            game.stick_gestures.reset();
            game.stick_fire = false;
            // Disabled means ignored entirely; the wizard still reads
            // and pilots below, so it can be set up with the map off.
            if !wizard_up {
                return;
            }
        } else {
            let axes = map.body_axes(&sample);
            game.input.set_stick(axes);
            game.stick_fire = surface == stick::Surface::Flight
                && !shift_held
                && map.fire.is_some_and(|b| sample.button(b));
            // The throttle's gestures: the lever hard back holds the air
            // brake; a slam forward is two seconds of chaos drive.
            let t = game.started.elapsed().as_secs_f64();
            let (gesture_brake, gesture_hyper) = game.stick_gestures.step(&map, &sample, t);
            game.input.set_stick_held(gesture_brake, gesture_hyper);
            // A flight-log line while the stick is doing something, a
            // second apart: the evidence that the map is the right way up.
            if game.stick_log > 0 {
                game.stick_log -= 1;
            } else if axes.iter().any(|v| v.abs() > 0.3) || sample.buttons != 0 {
                let f = map.flight(&sample);
                log::info!(
                    "stick: pitch {:+.2} yaw {:+.2} roll {:+.2} throttle {:+.2} strafe {:+.2} lift {:+.2} buttons {:#x} -> thrust [{:+.2} {:+.2} {:+.2}] torque [{:+.2} {:+.2} {:+.2}]",
                    f[0], f[1], f[2], f[3], f[4], f[5], sample.buttons,
                    axes[0], axes[1], axes[2], axes[3], axes[4], axes[5]
                );
                game.stick_log = 60;
            }
        }
        // Button edges, outside the borrow: each press through the pilot
        // table, each release as whatever key its press sent.
        let mut edges: Vec<(u8, bool)> = Vec::new();
        for b in 0..stick::MAX_BUTTONS {
            let bit = 1u32 << b;
            if pressed & bit != 0 {
                edges.push((b, true));
            } else if released & bit != 0 {
                edges.push((b, false));
            }
        }
        for (b, down) in edges {
            let (Some(gpu), Some(game)) = (self.gpu.as_mut(), self.game.as_mut()) else {
                return;
            };
            log::info!(
                "stick: {} {}",
                game.settings.stick.button_name(Some(b)),
                if down { "down" } else { "up" }
            );
            if down && game.menu.open && game.menu.rebinding() {
                let map = game.settings.stick;
                if map.back == Some(b) {
                    // BACK is ESC here too: the wait is cancelled.
                    let ev = game.menu.key(KeyCode::Escape, &mut game.settings);
                    apply_menu_event(game, gpu, event_loop, ev);
                } else if map.shift != Some(b) {
                    let ev = game.menu.stick_button(b, shift_held, &mut game.settings);
                    apply_menu_event(game, gpu, event_loop, ev);
                }
                continue;
            }
            let code = if down {
                let code = match game.settings.stick.pilot(b, shift_held, surface) {
                    stick::Pilot::None => None,
                    stick::Pilot::Key(c) => Some(c),
                    stick::Pilot::Named(n) => Some(game.settings.bindings.named(n)),
                };
                game.stick_sent[b as usize] = code;
                code
            } else {
                game.stick_sent[b as usize].take()
            };
            if let Some(code) = code {
                self.key_input(event_loop, code, down, false);
            }
        }
    }

    /// A key, pressed or released — from the keyboard, or from a stick
    /// button standing in for the key its control is bound to (so a
    /// button bound to BOOST is exactly the BOOST key). The wizard, when
    /// it is up, takes every key first.
    fn key_input(
        &mut self,
        event_loop: &ActiveEventLoop,
        code: KeyCode,
        pressed: bool,
        repeat: bool,
    ) {
        let (Some(gpu), Some(game)) = (self.gpu.as_mut(), self.game.as_mut()) else {
            return;
        };
        // The CONTROLS card: any key puts it away; F1 brings it back.
        if game.card_open {
            if pressed && !repeat {
                game.close_card();
            }
            return;
        }
        if pressed && !repeat && code == KeyCode::F1 && !game.design {
            game.open_card();
            return;
        }
        // The stick wizard, over the open menu: every key is its.
        if let Some(w) = game.wizard.as_mut() {
            if pressed && !repeat {
                match w.key(code, &mut game.settings.stick) {
                    stick::WizardEvent::Done => {
                        game.wizard = None;
                        game.settings.save();
                    }
                    stick::WizardEvent::Changed => game.settings.save(),
                    stick::WizardEvent::Nothing => {}
                }
            }
            return;
        }
        if game.design {
            if pressed && !repeat {
                let aspect = gpu.config.width as f32 / gpu.config.height as f32;
                match code {
                    KeyCode::KeyK | KeyCode::Escape => {
                        game.toggle_design();
                        gpu.set_look_cursor(game.look.engaged());
                    }
                    other => game.design_key(other, aspect),
                }
            }
            return;
        }
        if game.panel_open() {
            // M closes the map from anywhere in it; B the bay.
            if pressed && !repeat && code == game.bind(Named::Map) {
                game.toggle_map();
                return;
            }
            if pressed && !repeat && code == game.bind(Named::Bay) {
                game.toggle_bay();
                return;
            }
            // Pane zoom from the keyboard, for a mouse with no wheel.
            if pressed && game.pane_open() {
                let notches = match code {
                    KeyCode::Equal | KeyCode::NumpadAdd => 1.0,
                    KeyCode::Minus | KeyCode::NumpadSubtract => -1.0,
                    _ => 0.0,
                };
                if game.map_open() {
                    game.map_view.zoom_by(notches);
                } else {
                    game.bay_view.zoom_by(notches);
                }
            }
            if pressed && !repeat {
                let panel = if game.map_open() {
                    &mut game.map_panel
                } else if game.bay_open() {
                    &mut game.bay_panel
                } else {
                    &mut game.menu
                };
                let ev = panel.key(code, &mut game.settings);
                apply_menu_event(game, gpu, event_loop, ev);
                // Leaving the panel on foot, the walker's gaze takes the
                // cursor back.
                gpu.set_look_cursor(game.grabs_cursor());
            }
            return;
        }
        // On foot the keyboard is the walker's (SPEC §6.5b): the
        // translation binds walk, BOOST runs, BRAKE jumps, DISEMBARK
        // boards at the hull; ESC still menus, the capture key still
        // captures. Everything of the ship's is out of reach.
        if game.eva_active() {
            match code {
                KeyCode::Escape if pressed && !repeat => {
                    game.toggle_menu();
                    gpu.set_look_cursor(game.grabs_cursor());
                }
                c if pressed && !repeat && c == game.bind(Named::Disembark) => {
                    game.try_board();
                    gpu.set_look_cursor(game.grabs_cursor());
                }
                c if pressed
                    && !repeat
                    && (c == game.bind(Named::Capture) || c == KeyCode::F12) =>
                {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        gpu.capture_requested = true;
                    }
                }
                _ => game.eva_key(code, pressed),
            }
            return;
        }
        match code {
            // Whatever was held is released when a panel opens: the
            // world pauses, the keys must not carry a thrust demand
            // across.
            KeyCode::Escape if pressed && !repeat => game.toggle_menu(),
            // Every named control below reads its binding — the
            // KEYS page lists all of them, so what the menu shows
            // is what the keyboard does. Edge-triggered, and
            // `repeat` is filtered: holding a key must not strobe
            // a toggle.
            c if pressed && !repeat && c == game.bind(Named::Bay) => game.toggle_bay(),
            c if pressed && !repeat && c == game.bind(Named::Hold) => game.toggle_hold(),
            c if pressed && !repeat && c == game.bind(Named::Map) => game.toggle_map(),
            c if pressed && !repeat && c == game.bind(Named::Appearance) => game.cycle_appearance(),
            c if pressed && !repeat && (c == game.bind(Named::Capture) || c == KeyCode::F12) => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    gpu.capture_requested = true;
                }
            }
            c if pressed
                && !repeat
                && (c == game.bind(Named::ScaleDown) || c == game.bind(Named::ScaleUp)) =>
            {
                let step = if c == game.bind(Named::ScaleUp) {
                    0.1
                } else {
                    -0.1
                };
                let next = gpu.scene.scale() + step;
                gpu.scene.set_scale(next);
                log::info!("render scale {:.0}%", gpu.scene.scale() * 100.0);
            }
            c if pressed && !repeat && c == game.bind(Named::Engage) => game.engage_warp(),
            c if pressed && !repeat && c == game.bind(Named::WarpStop) => game.warp_stop(),
            c if pressed && !repeat && c == game.bind(Named::Landing) => game.toggle_landing(),
            c if pressed && !repeat && c == game.bind(Named::Disembark) => {
                game.disembark();
                // Walked out: the gaze takes the cursor at once.
                gpu.set_look_cursor(game.grabs_cursor());
            }
            c if pressed && !repeat && c == game.bind(Named::Design) => {
                game.toggle_design();
                gpu.set_look_cursor(game.look.engaged() && !game.design);
            }
            c if pressed && !repeat && c == game.bind(Named::LookLock) => {
                game.look.toggle_lock();
                gpu.set_look_cursor(game.look.engaged());
            }
            c if pressed && !repeat && c == game.bind(Named::Trajectory) => {
                game.settings.layout.cycle(Instrument::Trajectory, true);
                game.settings.save();
                log::info!(
                    "trajectory {}",
                    game.settings.layout.get(Instrument::Trajectory).name()
                );
            }
            c if pressed && !repeat && c == game.bind(Named::Chase) => {
                game.settings.camera_chase = !game.settings.camera_chase;
                game.settings.save();
                log::info!(
                    "camera {}",
                    if game.settings.camera_chase {
                        "CHASE"
                    } else {
                        "FIRST PERSON"
                    }
                );
            }
            c if pressed && !repeat && c == game.bind(Named::Holo) => {
                game.settings.holo_view = !game.settings.holo_view;
                game.settings.save();
                log::info!(
                    "holo3PP {}",
                    if game.settings.holo_view { "ON" } else { "OFF" }
                );
            }
            c if pressed && !repeat && c == game.bind(Named::VrRecentre) => {
                game.vr_recentre = true;
            }
            c if pressed && !repeat && c == game.bind(Named::Weapon1) => {
                game.arms.select(arms::Weapon::Cannon);
                log::info!("arms: {}", game.arms.selected.name());
            }
            c if pressed && !repeat && c == game.bind(Named::Weapon2) => {
                game.arms.select(arms::Weapon::Rail);
                log::info!("arms: {}", game.arms.selected.name());
            }
            c if pressed && !repeat && c == game.bind(Named::NextWeapon) => {
                game.arms.next_weapon();
                log::info!("arms: {}", game.arms.selected.name());
            }
            c if pressed && !repeat && c == game.bind(Named::Assist) => {
                game.assist = !game.assist;
                log::info!("flight assist {}", if game.assist { "ON" } else { "OFF" });
            }
            c if pressed && (c == game.bind(Named::HoloOut) || c == game.bind(Named::HoloIn)) => {
                game.zoom_holo(if c == game.bind(Named::HoloOut) {
                    1.0
                } else {
                    -1.0
                });
            }
            _ => game.input.set(code, pressed),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_none() {
            self.init_gpu(event_loop);
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if self.gpu.as_ref().is_some_and(|g| g.cfg.bench) {
            return;
        }
        // Raw motion, not cursor position: the cursor is grabbed while
        // looking, and raw counts keep coming at the screen edge.
        if let DeviceEvent::MouseMotion { delta } = event {
            if let Some(game) = self.game.as_mut() {
                // On foot the mouse is the walker's gaze, always engaged —
                // except under a panel, where it is the menu's again.
                if game.eva_active() {
                    if !game.panel_open() {
                        let sens = f64::from(game.settings.look_sensitivity);
                        if let Some(w) = game.eva.as_mut() {
                            w.look(delta.0, delta.1, sens);
                        }
                    }
                } else {
                    game.look.motion(delta.0 as f32, delta.1 as f32);
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        #[cfg(target_arch = "wasm32")]
        self.pick_up_pending();
        let (Some(gpu), Some(game)) = (self.gpu.as_mut(), self.game.as_mut()) else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => {
                game.log_exit("window closed");
                event_loop.exit();
            }
            // A benchmark is deaf: no key, mouse or wheel reaches the game,
            // so a stray keypress on the other screen never changes what is
            // being measured or steals a freelook.
            WindowEvent::KeyboardInput { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::CursorMoved { .. }
            | WindowEvent::Focused(_)
                if gpu.cfg.bench => {}
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                let pressed = event.state == ElementState::Pressed;
                self.key_input(event_loop, code, pressed, event.repeat);
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Right,
                ..
            } => {
                game.look.set_held(state == ElementState::Pressed);
                gpu.set_look_cursor(game.grabs_cursor());
                if !game.look.engaged() {
                    game.end_drag();
                }
            }
            // Left button: on the map, drag it round; while looking, drag
            // the dial under the gaze.
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                game.left_down = state == ElementState::Pressed;
                // The CONTROLS card: a click puts it away like any key.
                if game.card_open {
                    if game.left_down {
                        game.close_card();
                    }
                    return;
                }
                if game.left_down {
                    let aspect = gpu.config.width as f32 / gpu.config.height as f32;
                    let cam = game.camera(aspect);
                    let px = panel::px_canopy(gpu.config.height as f32) * game.text_fov_scale(&cam);
                    let text_w = game.text_w(px);
                    game.press_flash = 1.0;
                    // The pointer first: a row of the bay's card or a pip,
                    // or a row of a text panel, is a click.
                    if let Some(at) = game.cursor_screen() {
                        if game.bay_open() && game.bay_click(at, aspect, text_w, px) {
                            game.left_down = false;
                            return;
                        }
                        if game.menu.open || game.map_open() {
                            let anchor = game.text_anchor(aspect, px);
                            let on = at[0] >= anchor[0] - 0.01
                                && at[0] <= anchor[0] + text_w / aspect + 0.01
                                && at[1] <= anchor[1];
                            if on {
                                let row =
                                    ((anchor[1] - at[1]) / (LINE as f32 * px)).floor() as usize;
                                let col = ((at[0] - anchor[0]) * aspect
                                    / (farfall_render::text::ADVANCE as f32 * px))
                                    .floor()
                                    .max(0.0) as usize;
                                let ev = if game.map_open() {
                                    game.map_panel.click(row, col, &mut game.settings)
                                } else {
                                    game.menu.click(row, col, &mut game.settings)
                                };
                                if ev != MenuEvent::Nothing {
                                    apply_menu_event(game, gpu, event_loop, ev);
                                    game.left_down = false;
                                    return;
                                }
                            }
                        }
                    }
                    // The glass first: a dial or a panel under the gaze is
                    // picked up. Nothing there, and the button is the
                    // trigger.
                    let picked = game.begin_drag(&cam, panel::px_canopy(gpu.config.height as f32));
                    game.fire_held =
                        !picked && !game.menu.open && !game.pane_open() && !game.design;
                } else {
                    game.fire_held = false;
                    game.end_drag();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let now = (position.x as f32, position.y as f32);
                game.window_size = (gpu.config.width as f32, gpu.config.height as f32);
                if let Some(last) = game.cursor {
                    // Turning the map with the mouse, unless the gaze has
                    // hold of the pane itself.
                    if game.left_down && game.map_open() && game.drag.is_none() {
                        game.map_view.drag(now.0 - last.0, now.1 - last.1);
                    }
                    if game.left_down && game.bay_open() && game.drag.is_none() {
                        game.bay_view.drag(now.0 - last.0, now.1 - last.1);
                    }
                }
                game.cursor = Some(now);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if game.map_open() {
                    let notches = match delta {
                        winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                        winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                    };
                    game.map_view.zoom_by(notches);
                }
                if game.bay_open() {
                    let notches = match delta {
                        winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                        winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                    };
                    game.bay_view.zoom_by(notches);
                }
                // In flight the wheel zooms the hologram: up is closer.
                if !game.panel_open() && game.holo_active() {
                    let notches = match delta {
                        winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                        winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                    };
                    game.zoom_holo(-notches);
                }
            }
            // A key held while the window loses focus never sees its release
            // event; without this the ship keeps thrusting unattended.
            WindowEvent::Focused(false) => {
                game.input.release_all();
                game.eva_keys = eva::Keys::default();
                game.fire_held = false;
                game.stick_fire = false;
                game.look.set_held(false);
                gpu.set_look_cursor(game.grabs_cursor());
                game.end_drag();
            }
            WindowEvent::Resized(size) => {
                gpu.config.width = size.width.max(1);
                gpu.config.height = size.height.max(1);
                gpu.surface.configure(&gpu.device, &gpu.config);
                // Reconfiguring the swapchain stalls; that frame is not the
                // renderer's fault and must not pollute the worst-frame stat.
                gpu.perf.stats.skip_next_frame();
                gpu.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                // The stick is polled once a frame, here, before the frame
                // samples the controls.
                self.poll_stick(event_loop);
                let (Some(gpu), Some(game)) = (self.gpu.as_mut(), self.game.as_mut()) else {
                    return;
                };
                redraw(gpu, game, self.audio.as_ref(), Some(event_loop));
            }
            _ => {}
        }
    }
}

/// Run the game: the native entry point, and the body of the web one.
#[cfg(not(target_arch = "wasm32"))]
pub fn run() {
    env_logger::init();
    let event_loop = winit::event_loop::EventLoop::new().expect("event loop");
    event_loop.run_app(&mut App::default()).expect("run");
}

#[cfg(target_arch = "wasm32")]
pub use web::run;

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{DQuat, DVec3};
    use input::{Action, InputState};
    use winit::keyboard::KeyCode;

    /// The whole EVA loop (SPEC §6.5b): DISEMBARK while LANDED walks out,
    /// the sim's ship never moves (the golden hash's state is untouched),
    /// the readout swaps to the suit's lines, boarding needs the hull in
    /// reach, and the same key walks back in.
    #[test]
    fn disembark_walks_out_and_the_same_key_boards_back() {
        let mut game = Game::new();
        game.state.ship = landing::parked(&game.params, 0);
        let before = game.state;
        let hash = sim::state_hash(&game.state);
        game.disembark();
        assert!(game.eva_active(), "on foot");
        assert_eq!(sim::state_hash(&game.state), hash, "the sim never felt it");
        assert_eq!(game.state.ship.pos_m, before.ship.pos_m, "the ship stays");
        // The suit's readout, with the boarding offer: the walk-out
        // stands within reach of the hull.
        let text = game.eva_text().expect("the suit's lines");
        assert!(text.contains("EVA"), "{text}");
        assert!(text.contains("BOARD"), "{text}");
        // The eye is outside the ship: the hull shows, the cabin stays home.
        assert!(game.exterior_view());
        let pose = game.pose(1.5);
        assert!(pose.eye_ship != DVec3::ZERO, "the eye left the ship");
        let d = game.eva_ship_m().unwrap();
        assert!((d - eva::EXIT_M).abs() < 3.0, "beside the ship: {d}");
        // Too far from the hull, the key does not board.
        let b = game.params.bodies(game.state.time_s)[0];
        if let Some(w) = game.eva.as_mut() {
            let t = w.up().any_orthonormal_vector();
            w.feet_m = (w.feet_m + t * 300.0).normalize() * b.radius_m;
        }
        game.try_board();
        assert!(game.eva_active(), "still walking back");
        // Beside it again, the same key boards.
        game.stage_eva_bench();
        game.try_board();
        assert!(!game.eva_active(), "back in the seat");
        assert_eq!(sim::state_hash(&game.state), hash, "and the sim never knew");
        assert_eq!(game.disembark_notice.map(|(n, _)| n), Some("BOARDED SHIP"));
    }

    /// The readout is diagnostics, not glassware: turning the head must
    /// not move it (it once drifted mid-sky through a spinning bench
    /// capture). The design card still rides the glass with the look.
    #[test]
    fn the_readout_keeps_its_screen_place_through_a_head_turn() {
        let mut game = Game::new();
        let cam = game.camera(1.5);
        game.look.aim(0.0, 0.0);
        let centred = game.text_screen_anchor(&cam, 0.002);
        game.look.aim(1.2, -0.4);
        let turned = game.text_screen_anchor(&game.camera(1.5), 0.002);
        assert_eq!(centred, turned, "the readout moved with the head");
        game.design = true;
        let designing = game.text_screen_anchor(&game.camera(1.5), 0.002);
        assert_ne!(centred, designing, "the design card is glass, and swings");
    }

    /// SPEC §5.3: a real headset capture (a9869ff) showed the readout at
    /// roughly 10 degrees a glyph, a tenth of the vertical field — the
    /// old VR_TEXT_SCALE=6.0 estimate was wrong by close to an order of
    /// magnitude. This measures what the app's own code actually
    /// produces, for an Index-like eye (2740 px tall, ~110 degrees
    /// vertical), against the 1.2-degree target — a number from the
    /// real formula, not a hand estimate re-guessed a second time.
    #[test]
    fn the_vr_readout_measures_about_1_2_degrees_a_glyph() {
        let mut game = Game::new();
        game.vr = Some(vr_view_facing(Quat::IDENTITY));
        let fov_y_deg = 110.0f32;
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: fov_y_deg.to_radians(),
            aspect: 2468.0 / 2740.0,
            time_s: 0.0,
            exposure: 1.0,
        };
        let px = panel::px_canopy(2740.0) * game.text_fov_scale(&cam);
        let glyph_ndc_height = farfall_render::text::GLYPH_H as f32 * px;
        // The same linear pixel-fraction-times-total-fov conversion the
        // capture review measured degrees-per-glyph with (glyph height
        // as a fraction of the full vertical field, times that field's
        // own degrees) — not the exact tangent-perspective conversion,
        // so this number is directly comparable to a screenshot
        // measurement, which is what VR_TEXT_SCALE_DEFAULT was tuned
        // against.
        let degrees = (glyph_ndc_height / 2.0) * fov_y_deg;
        assert!(
            (degrees - 1.2).abs() < 0.3,
            "{degrees} degrees per glyph, want 1.2 +/- 0.3"
        );
    }

    /// SPEC §5.3: the L/R corner mark's own angular size must not
    /// depend on which headset it is drawn for — an e80e9af synth
    /// capture showed a fixed px_canopy=0.09 filling ~35% of the eye's
    /// own width, unmistakable for the wrong reason. Same linear
    /// convention as the readout's own measurement above.
    #[test]
    fn the_label_measures_about_its_own_target_degrees() {
        let fov_y_deg = 110.0f32;
        let ty = (fov_y_deg.to_radians() * 0.5).tan();
        let px = label_px_canopy(ty);
        let glyph_ndc_height = farfall_render::text::GLYPH_H as f32 * px;
        let degrees = (glyph_ndc_height / 2.0) * fov_y_deg;
        assert!(
            (degrees - LABEL_TARGET_DEG).abs() < 0.3,
            "{degrees} degrees, want {LABEL_TARGET_DEG} +/- 0.3"
        );
        // A much narrower headset gets a bigger canopy fraction for the
        // same degrees — the whole point of computing from ty.
        let narrow_ty = (60f32.to_radians() * 0.5).tan();
        assert!(
            label_px_canopy(narrow_ty) > px,
            "a narrower fov should need more canopy per degree"
        );
    }

    /// The eye-order self-check's own shape identification: painting a
    /// patch that exactly reproduces one glyph's bit pattern must score
    /// that glyph over the other one — the machine check for "is this
    /// really an L, not an R," not just "is there ink somewhere."
    #[test]
    fn glyph_shape_readback_identifies_the_letter_it_was_painted_as() {
        use farfall_render::text::{glyph, GLYPH_H, GLYPH_W};
        for &c in &['L', 'R'] {
            let bits = glyph(c);
            // One BGRA8 texel per font cell — glyph_shape's own box
            // downsample degenerates to an exact read at 1:1.
            let mut bgra = vec![0u8; GLYPH_W * GLYPH_H * 4];
            for (row, &bits_row) in bits.iter().enumerate() {
                for col in 0..GLYPH_W {
                    let lit = (bits_row >> col) & 1 != 0;
                    let idx = (row * GLYPH_W + col) * 4;
                    if lit {
                        // Bright cyan: matches is_ink's own threshold.
                        bgra[idx..idx + 4].copy_from_slice(&[255, 255, 0, 255]);
                    }
                }
            }
            let shape = glyph_shape(&bgra, (GLYPH_W as u32, GLYPH_H as u32));
            let own = glyph_match_score(shape, glyph(c));
            let other = glyph_match_score(shape, glyph(if c == 'L' { 'R' } else { 'L' }));
            assert!(
                own > other,
                "{c}: own score {own} did not beat the other letter's {other}"
            );
            assert_eq!(
                own,
                (GLYPH_W * GLYPH_H) as u32,
                "{c}: an exact painting should score a perfect match"
            );
        }
    }

    /// A headset for the given eye alone, level-eyed and centred (no
    /// asymmetric FOV — the tests below care about rotation, not lens
    /// geometry) turned by `head`.
    fn vr_view_facing(head: Quat) -> VrView {
        let eye = VrEye {
            head,
            pos: Vec3::ZERO,
            tan: [1.0, 1.0, 1.0, 1.0],
        };
        VrView { eyes: [eye, eye] }
    }

    /// SPEC §5.3, canted lenses: an eye whose asymmetric tangents could
    /// never arise from any flat fov setting, so a test against it can
    /// tell "the eye's own fov" from "a flat fov that happens to match".
    fn canted_vr_view() -> VrView {
        let eye = VrEye {
            head: Quat::IDENTITY,
            pos: Vec3::new(-0.03, 0.0, 0.0),
            tan: [1.4, 0.7, 0.6, 0.5],
        };
        VrView { eyes: [eye, eye] }
    }

    /// A headset's fov is the runtime's alone (SPEC §5.3): every pose
    /// variant — cockpit, chase, and (by the same code path) EVA — must
    /// take the eye's own symmetric hull, never settings.fov, the
    /// thrust flare, or a warp's fov_scale. A changing fov in a headset
    /// is instant sickness, so this is checked with those knobs pushed
    /// to their most extreme, most-likely-to-leak values.
    #[test]
    fn vr_fov_is_the_eyes_own_regardless_of_flat_fov_knobs_cockpit_and_chase() {
        let mut game = Game::new();
        game.vr = Some(canted_vr_view());
        game.settings.fov = 179.0; // the widest flat setting allows
        game.effort = 1.0; // full thrust flare
        let (expected_fov_y, expected_aspect) = game.vr.as_ref().unwrap().eyes[0].symmetric();

        let cockpit = game.pose(1.7);
        assert!(
            (cockpit.cam.fov_y - expected_fov_y).abs() < 1e-5,
            "cockpit fov_y leaked a flat value: {} vs {expected_fov_y}",
            cockpit.cam.fov_y
        );
        assert!(
            (cockpit.cam.aspect - expected_aspect).abs() < 1e-5,
            "cockpit aspect leaked a flat value: {} vs {expected_aspect}",
            cockpit.cam.aspect
        );

        game.settings.camera_chase = true;
        let chase = game.pose(1.7);
        assert!(
            (chase.cam.fov_y - expected_fov_y).abs() < 1e-5,
            "chase fov_y leaked a flat value: {} vs {expected_fov_y}",
            chase.cam.fov_y
        );
        assert!(
            (chase.cam.aspect - expected_aspect).abs() < 1e-5,
            "chase aspect leaked a flat value: {} vs {expected_aspect}",
            chase.cam.aspect
        );
        // The eye's own IPD offset must still reach chase's seat, added
        // to the chase rig's own fixed seat rather than replacing it.
        assert_ne!(
            chase.eye_ship, CHASE_EYE_SHIP,
            "the eye offset never reached chase"
        );
        assert!((chase.eye_ship - (CHASE_EYE_SHIP + DVec3::new(-0.03, 0.0, 0.0))).length() < 1e-6);
    }

    /// SPEC §5.3, readability: a headset's narrower vertical fov (the
    /// Index's ~55 degrees against the flat reference's 70) needs the
    /// readout scaled past the flat clamp's own ceiling to stay legible,
    /// without changing anything about flat flight's own sizing — but
    /// only past it, not by the old VR_TEXT_SCALE=6.0 estimate's own
    /// wild margin: a real capture (a9869ff) showed that put a glyph at
    /// roughly 10 degrees against a 1.2-degree target, so
    /// VR_TEXT_SCALE_DEFAULT is a measured ~1.27, not a guessed 6.0 —
    /// see `the_vr_readout_measures_about_1_2_degrees_a_glyph`.
    #[test]
    fn vr_scales_the_readout_up_past_flats_own_fov_clamp() {
        let mut game = Game::new();
        let cam = game.camera(1.5);
        let flat_scale = game.text_fov_scale(&cam);

        game.vr = Some(canted_vr_view());
        let vr_cam = game.pose(1.5).cam;
        let vr_scale = game.text_fov_scale(&vr_cam);

        assert!(
            vr_scale > flat_scale * 1.3,
            "vr={vr_scale} flat={flat_scale}: VR should still read larger"
        );
    }

    /// SPEC §5.3: in flat flight the readout is deliberately screen-fixed
    /// (see the test above) — a monitor does not move when the mouse
    /// looks around. A headset's "screen" is the pilot's own face, so
    /// the same rule there would glue the readout to it; VR must instead
    /// keep the readout in a fixed cockpit spot, exactly like a dash
    /// dial, as the pilot's real head turns.
    #[test]
    fn in_vr_the_readout_is_cockpit_fixed_and_moves_as_the_real_head_turns() {
        let mut game = Game::new();
        game.vr = Some(vr_view_facing(Quat::IDENTITY));
        let centred = game.text_screen_anchor(&game.camera(1.5), 0.002);
        game.vr = Some(vr_view_facing(Quat::from_rotation_y(0.6)));
        let turned = game.text_screen_anchor(&game.camera(1.5), 0.002);
        assert_ne!(
            centred, turned,
            "a VR readout frozen at the flat identity would not move \
             with the real headset — it would be stuck to the pilot's face"
        );
    }

    /// SPEC §5.3, 6-DoF: a headset's per-eye *position* (not only its
    /// rotation) has to reach the cabin, or leaning toward the dash
    /// would not bring it any closer — exactly the same pipeline the
    /// chase rig's own fixed seat (`CHASE_EYE_SHIP`) already proves
    /// works. Two eyes at the same orientation but different seats must
    /// produce different `CabinUniforms` (their `eye` lane).
    #[test]
    fn two_vr_eyes_at_different_seats_produce_different_cabin_eye_uniforms() {
        let mut game = Game::new();
        let cam = game.camera(1.5);
        let left = VrEye {
            head: Quat::IDENTITY,
            pos: Vec3::new(-0.032, 0.0, 0.0),
            tan: [1.0, 1.0, 1.0, 1.0],
        };
        let right = VrEye {
            head: Quat::IDENTITY,
            pos: Vec3::new(0.032, 0.0, 0.0),
            tan: [1.0, 1.0, 1.0, 1.0],
        };
        game.vr = Some(VrView {
            eyes: [left, right],
        });
        let (cabin, _) = game.cabin_uniforms(&cam);
        game.vr_eye = 0;
        let left_pose = game.pose(1.5);
        game.vr_eye = 1;
        let right_pose = game.pose(1.5);
        assert_ne!(
            left_pose.eye_ship, right_pose.eye_ship,
            "the two eyes' own seats must differ"
        );
        let left_cabin = cabin.with_eye(left_pose.eye_ship.as_vec3());
        let right_cabin = cabin.with_eye(right_pose.eye_ship.as_vec3());
        assert_ne!(
            left_cabin, right_cabin,
            "two eyes at different seats must march the cabin from \
             different origins, or leaning toward the dash does nothing"
        );
    }

    /// The same fix, for every glass-style dial anchor (`slot_of`) and
    /// the mini map: `Game::glass_head` must return the active eye's own
    /// head in VR, not the mouse-driven `Look` (which VR never touches
    /// and which would otherwise freeze these at whatever the session
    /// happened to start facing).
    #[test]
    fn glass_head_follows_the_headset_not_the_untouched_mouse_look() {
        let mut game = Game::new();
        game.look.aim(0.9, -0.2); // a mouse look VR never drives
        let vr_head = Quat::from_rotation_y(-1.1) * Quat::from_rotation_x(0.3);
        game.vr = Some(vr_view_facing(vr_head));
        let got = game.glass_head();
        assert!(
            got.angle_between(vr_head) < 1e-5,
            "glass_head should be the headset's own orientation, got {got:?}"
        );
        assert!(
            got.angle_between(game.look.rotation()) > 1e-3,
            "glass_head must not fall back to the untouched mouse look in VR"
        );
    }

    /// The gyro ball is a real sphere cast in the dash: near the rim of
    /// a far-turned view its projection blows out and once filled a
    /// corner scissor patch with magnified globe. With its anchor off
    /// the screen the ball is culled; head centred, it is back.
    #[test]
    fn the_gyro_ball_is_culled_once_its_anchor_leaves_the_screen() {
        let mut game = Game::new();
        game.settings.gauge_style = settings::GaugeStyle::Warthog;
        game.look.aim(0.0, 0.0);
        let cam = game.camera(1.5);
        let tw = game.dial_tweak(Instrument::Gyro);
        assert!(
            game.gyro_ball(&cam, tw, glam::Vec3::ZERO).is_some(),
            "centred: the ball shows"
        );
        game.look.aim(1.2, 0.0);
        let cam = game.camera(1.5);
        assert!(
            game.gyro_ball(&cam, tw, glam::Vec3::ZERO).is_none(),
            "turned 69\u{b0} right: the ball's anchor is off screen"
        );
    }

    /// The camera basis must agree with the ship's own axes exactly. If these
    /// ever diverge, the world is mirrored or rolled relative to the hull.
    #[test]
    fn camera_basis_matches_ship_axes() {
        let mut game = Game::new();
        game.state.ship.orient = DQuat::from_euler(glam::EulerRot::YXZ, 0.7, -0.3, 0.2).normalize();
        let (right, up, forward) = game.camera(1.0).basis();
        let ship = game.state.ship.orient;
        for (got, want, name) in [
            (right, ship * DVec3::X, "right"),
            (up, ship * DVec3::Y, "up"),
            (forward, ship * DVec3::NEG_Z, "forward"),
        ] {
            assert!(
                (got - want.as_vec3()).length() < 1e-5,
                "camera {name} {got:?} != ship {name} {want:?}"
            );
        }
    }

    /// Steering the ship must swing the view by the same angle.
    #[test]
    fn camera_follows_ship_orientation() {
        let mut game = Game::new();
        game.state.ship.orient = DQuat::IDENTITY;
        let before = game.camera(1.0).basis().2;
        game.state.ship.orient = DQuat::from_rotation_y(std::f64::consts::FRAC_PI_2);
        let after = game.camera(1.0).basis().2;
        let angle = before.dot(after).clamp(-1.0, 1.0).acos();
        assert!(
            (angle - std::f32::consts::FRAC_PI_2).abs() < 1e-3,
            "view swung {angle} rad, expected pi/2"
        );
    }

    /// The chase rig: the eye sits behind and above the ship in its own
    /// frame, the view pitches down to centre it, and every world builder
    /// measures from that eye — not from the pilot's seat.
    /// A dial's scissor: a patch around its screen anchor, wider than
    /// tall by the aspect, clamped to the target; hidden dials get none.
    #[test]
    fn a_dial_draws_only_in_its_own_patch() {
        let r = dial_scissor([0.0, 0.0], 1.0, 1.0, 2.0, (2000, 1000)).unwrap();
        // Centred: symmetric about the middle.
        assert_eq!(r[0] + r[2] / 2, 1000);
        assert_eq!(r[1] + r[3] / 2, 500);
        // Canopy x runs over the aspect: the patch is as wide in pixels as
        // it is tall.
        assert!((r[2] as i32 - r[3] as i32).abs() <= 2, "{r:?}");
        assert!(r[3] < 1000 && r[3] > 200);
        // A corner dial is clamped to the screen, never negative.
        let c = dial_scissor([-0.98, -0.98], 1.0, 1.0, 2.0, (2000, 1000)).unwrap();
        assert_eq!(c[0], 0);
        assert!(c[1] + c[3] <= 1000);
        assert!(dial_scissor([0.0, 0.0], 0.0, 1.0, 2.0, (2000, 1000)).is_none());
        // Off-screen entirely: nothing.
        assert!(dial_scissor([3.0, 3.0], 1.0, 1.0, 2.0, (2000, 1000)).is_none());
    }

    #[test]
    fn the_chase_eye_sits_behind_the_ship_and_the_world_measures_from_it() {
        let mut game = Game::new();
        game.state.ship.orient = DQuat::from_rotation_y(1.1);
        game.state.ship.pos_m = DVec3::new(1.0e6, -2.0e6, 3.0e6);
        // First person: the eye is the ship.
        let fp = game.pose(1.5);
        assert_eq!(game.eye_m(&fp), game.state.ship.pos_m);
        // Chase: behind (+Z of the ship) and above (+Y), world frame.
        game.settings.camera_chase = true;
        let ch = game.pose(1.5);
        let off = game.eye_m(&ch) - game.state.ship.pos_m;
        let back = game.state.ship.orient * DVec3::Z;
        let up = game.state.ship.orient * DVec3::Y;
        assert!(off.dot(back) > 20.0, "the eye is lengths behind: {off:?}");
        assert!(off.dot(up) > 2.0, "and above: {off:?}");
        assert!(
            (off.length() - CHASE_EYE_SHIP.length()).abs() < 1e-9,
            "rigid rig"
        );
        // The view pitches down toward the ship, and the ship is in frame:
        // the eye-to-ship line is close to the view's forward axis.
        let fwd = ch.cam.basis().2;
        let to_ship = (-off).normalize().as_vec3();
        assert!(
            fwd.dot(to_ship) > 0.99,
            "the ship centres in the chase view: {}",
            fwd.dot(to_ship)
        );
        // The jet draws in chase and never in the cockpit.
        assert!(game.chase_active());
        game.settings.camera_chase = false;
        assert!(!game.exterior_view());
    }

    fn fly(keys: &[KeyCode], secs: u64) -> sim::ShipState {
        let params = sim::presets::earth_compact();
        let altitude = std::env::var("FARFALL_BENCH_ALT")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(SPAWN_ALTITUDE_M);
        // In vacuum: these tests are about the control mapping, and a hull
        // falling through air picks up real aerodynamic torques of its own
        // (a broadside wind on a tail-heavy-of-pressure ship yaws it). The
        // air's behaviour has its own tests in the sim crate.
        let altitude = altitude.max(params.planet.atmo_top_m + 1_000.0);
        let mut state = sim::presets::circular_orbit(&params, altitude);
        state.ship.orient = DQuat::IDENTITY;
        state.ship.vel_mps = DVec3::ZERO; // isolate control response from orbit
        let mut input = InputState::default();
        for k in keys {
            input.set(*k, true);
        }
        let controls = input.controls_immediate(false);
        for _ in 0..(secs * 120) {
            state = sim::step(&params, &state, controls);
        }
        state.ship
    }

    /// Every control direction, asserted against the real integrator from the
    /// pilot's point of view. Comments lie; this is what caught the frame being
    /// declared left-handed while the rotation math was right-handed, which
    /// silently mirrored yaw, roll, and strafe.
    #[test]
    fn the_dials_are_only_picked_up_in_design_mode() {
        let mut game = Game::new();
        let cam = game.camera(1.5);
        let text_w = 0.4;
        // The readout out of the way; aim the head at the speed dial.
        game.settings.readout_anchor = [-0.9, 0.9];
        let a = game.settings.layout.anchor(Instrument::Speed).unwrap();
        let t = game.ref_tan();
        game.look
            .aim((a[0] * t * 1.5_f32).atan(), (a[1] * t).atan());
        assert!(!game.begin_drag(&cam, text_w), "freelook leaves the dials");
        assert!(game.drag.is_none());
        // In DESIGN mode the pointer is the cursor: head straight, the
        // cursor on the dial through the live FOV.
        game.design = true;
        game.look.aim(0.0, 0.0);
        game.window_size = (1500.0, 1000.0);
        let k = t / (cam.fov_y * 0.5).tan();
        game.cursor = Some(((a[0] * k + 1.0) * 750.0, (1.0 - a[1] * k) * 500.0));
        assert!(game.begin_drag(&cam, text_w), "design mode picks it up");
        assert!(matches!(
            game.drag,
            Some((Dragged::Dial(Instrument::Speed), _))
        ));
    }

    /// The DESIGN keys turn a dial all three ways — tilt (,/.), sideways
    /// lean (;/'), in-plane rotation (9/0) — each held to its reach, and
    /// Backspace puts the whole tweak back to stock.
    #[test]
    fn design_keys_lean_and_rotate_a_dial_and_backspace_resets() {
        let mut game = Game::new();
        game.design = true;
        game.look.aim(0.0, 0.0);
        game.window_size = (1500.0, 1000.0);
        game.settings.readout_anchor = [-0.9, 0.9];
        let cam = game.camera(1.5);
        let a = game.settings.layout.anchor(Instrument::Speed).unwrap();
        let t = game.ref_tan();
        let k = t / (cam.fov_y * 0.5).tan();
        game.cursor = Some(((a[0] * k + 1.0) * 750.0, (1.0 - a[1] * k) * 500.0));
        assert_eq!(
            game.design_target(1.5),
            Some(DesignEl::Dial(Instrument::Speed))
        );
        game.design_key(KeyCode::Quote, 1.5);
        game.design_key(KeyCode::Quote, 1.5);
        game.design_key(KeyCode::Digit9, 1.5);
        game.design_key(KeyCode::Comma, 1.5);
        let d = game.settings.dials[Instrument::Speed as usize];
        assert_eq!(d.lean_deg, 10.0);
        assert_eq!(d.rotate_deg, -15.0);
        assert_eq!(d.tilt_deg, -5.0);
        for _ in 0..40 {
            game.design_key(KeyCode::Quote, 1.5);
            game.design_key(KeyCode::Digit0, 1.5);
        }
        let d = game.settings.dials[Instrument::Speed as usize];
        assert_eq!(d.lean_deg, settings::TILT_MAX);
        assert_eq!(d.rotate_deg, settings::ROTATE_MAX);
        game.design_key(KeyCode::Backspace, 1.5);
        assert_eq!(
            game.settings.dials[Instrument::Speed as usize],
            settings::DialTweak::DEFAULT
        );
    }

    /// The holo3PP, the mini map and the readout are design elements
    /// like the dials: DESIGN mode's pointer finds them, -/= sizes them,
    /// and the mini map can be dragged anywhere and keeps its place.
    #[test]
    fn design_mode_takes_the_holo_the_mini_map_and_the_readout() {
        let mut game = Game::new();
        game.design = true;
        game.look.aim(0.0, 0.0);
        game.window_size = (1500.0, 1000.0);
        let cam = game.camera(1.5);
        let t = game.ref_tan();
        let k = t / (cam.fov_y * 0.5).tan();
        let at = |a: [f32; 2]| ((a[0] * k + 1.0) * 750.0, (1.0 - a[1] * k) * 500.0);
        // The hologram: found by its anchor, sized with =.
        game.cursor = Some(at(game.settings.holo_anchor));
        assert_eq!(game.design_target(1.5), Some(DesignEl::Holo));
        let size = game.settings.holo_size;
        game.design_key(KeyCode::Equal, 1.5);
        assert!(game.settings.holo_size > size);
        // The mini map: found by its pane, shrunk with -, dragged and
        // its anchor kept on the layout.
        assert!(game.mini_map_shown(), "the mini map shows in DESIGN mode");
        let mini = game.settings.layout.inset(game.mini_map_anchor());
        game.cursor = Some(at(mini));
        assert_eq!(game.design_target(1.5), Some(DesignEl::MiniMap));
        game.design_key(KeyCode::Minus, 1.5);
        assert!(game.settings.dials[Instrument::Map as usize].size < 1.0);
        assert!(game.mini_map_half_h() < map::MINI_HALF_H);
        assert!(game.begin_drag(&cam, 0.4), "the mini map is picked up");
        assert!(matches!(game.drag, Some((Dragged::MiniMap, _))));
        game.cursor = Some(at([mini[0] - 0.3, mini[1] - 0.4]));
        game.update_drag(&cam);
        let moved = game.mini_map_anchor();
        assert!(
            (moved[0] - map::MINI_ANCHOR[0]).abs() > 0.1
                || (moved[1] - map::MINI_ANCHOR[1]).abs() > 0.1,
            "{moved:?}"
        );
        game.design_key(KeyCode::Backspace, 1.5);
        // The readout: found by its block; its SIZE scales the text.
        game.cursor = Some(at([
            game.settings.readout_anchor[0] + 0.06,
            game.settings.readout_anchor[1] - 0.14,
        ]));
        assert_eq!(game.design_target(1.5), Some(DesignEl::Readout));
        game.design_key(KeyCode::Equal, 1.5);
        game.design = false;
        assert!(game.text_fov_scale(&cam) > 1.0 * 0.99 * 1.1);
    }

    #[test]
    fn the_readout_block_is_picked_up_by_the_gaze_while_looking() {
        let mut game = Game::new();
        let cam = game.camera(1.5);
        let text_w = 0.4;
        // Not looking: no pointer, nothing to pick up.
        assert!(!game.begin_drag(&cam, text_w));
        // Put the readout somewhere and aim the head at its block.
        game.settings.readout_anchor = [0.3, 0.2];
        let t = game.ref_tan();
        let yaw = (0.4 * t * 1.5_f32).atan();
        let pitch = (0.1 * t).atan();
        game.look.aim(yaw, pitch);
        let gaze = game.look.gaze(t, 1.5);
        assert!(
            (gaze[0] - 0.4).abs() < 0.05 && (gaze[1] - 0.1).abs() < 0.05,
            "{gaze:?}"
        );
        assert!(game.begin_drag(&cam, text_w));
        assert!(matches!(game.drag, Some((Dragged::Readout, _))));
        // Turn the head: the block follows, keeping its offset.
        game.look.aim(yaw, pitch - 0.2);
        game.update_drag(&cam);
        assert!(game.settings.readout_anchor[1] < 0.2);
        assert!((game.settings.readout_anchor[0] - 0.3).abs() < 0.05);
    }

    #[test]
    fn the_hull_meets_more_rock_the_faster_it_goes_and_none_in_air() {
        assert_eq!(strike_rate_hz(0.0, 0.0), 0.0, "a still ship meets nothing");
        let cruise = strike_rate_hz(1_000.0, 0.0);
        assert!(
            (0.4..0.6).contains(&cruise),
            "one every couple of seconds: {cruise}"
        );
        assert!(strike_rate_hz(3_000.0, 0.0) > cruise * 4.0);
        assert_eq!(strike_rate_hz(3.0e7, 0.0), 6.0, "capped at a patter");
        assert_eq!(strike_rate_hz(3_000.0, 1.0), 0.0, "no dust in air");
        assert_eq!(hull_stress(0.0), 0.0);
        assert_eq!(hull_stress(sim::RELATIVITY_FROM_MPS * 3.0), 1.0);
        assert!(strike_size_from(0.5) < 0.3 && strike_size_from(0.999) > 0.9);
        // The dice are fair enough and the process runs: a fast ship in
        // space collects strikes over a minute, a slow one far fewer.
        let mut fast = Game::new();
        fast.state.ship.vel_mps = glam::DVec3::new(0.0, 0.0, -6_000.0);
        fast.state.ship.pos_m = glam::DVec3::new(0.0, 0.0, fast.params.planet.radius_m * 4.0);
        for _ in 0..(60 * 120) {
            fast.roll_for_strikes();
        }
        assert!(fast.strikes > 60 && fast.strikes < 400, "{}", fast.strikes);
        assert!(fast.strike_size > 0.0 && fast.strike_size <= 1.0);
        let mut slow = Game::new();
        slow.state.ship.vel_mps = glam::DVec3::ZERO;
        slow.state.ship.pos_m = fast.state.ship.pos_m;
        for _ in 0..(60 * 120) {
            slow.roll_for_strikes();
        }
        assert_eq!(slow.strikes, 0, "still: nothing, ever");
        // Every strike leaves an impact on the shell, ahead of the motion,
        // newest first, and the shell remembers only so many.
        assert!(!fast.impacts.is_empty());
        assert!(fast.impacts.len() <= farfall_render::shield::IMPACTS);
        for im in &fast.impacts {
            assert!((im.dir.length() - 1.0).abs() < 1e-4);
            assert!(im.dir.z < 0.0, "from ahead: {:?}", im.dir);
            assert!(im.size > 0.0 && im.size <= 1.0);
        }
        assert!(fast.impacts[0].at_s >= fast.impacts[fast.impacts.len() - 1].at_s);
        // Off: no rock at all.
        let mut mute = Game::new();
        mute.settings.hull_sound = false;
        mute.state.ship.vel_mps = glam::DVec3::new(0.0, 0.0, -6_000.0);
        for _ in 0..(60 * 120) {
            mute.roll_for_strikes();
        }
        assert_eq!(mute.strikes, 0);
    }

    #[test]
    fn the_hyper_drive_strains_slips_somewhere_else_and_collapses_to_a_crawl() {
        let mut game = Game::new();
        let home = game.state.ship.pos_m;
        // Hold the field at full run: strain climbs, and within a minute
        // the drive slips — into the flip, then a jump to some body.
        game.input
            .set(game.settings.bindings.named(Named::Hyper), true);
        game.hyper_was = true;
        game.state.ship.vel_mps = glam::DVec3::new(0.0, 0.0, -game.params.ship.hyper_max_mps);
        let mut slipped = false;
        for _ in 0..(60 * 60) {
            game.run_hyper_strain(1.0 / 60.0);
            if game.pending_slip {
                slipped = true;
                break;
            }
        }
        assert!(slipped, "strain {}", game.hyper_strain);
        assert!(game.warp.active());
        assert!(game.strain_text().is_none(), "the strain is spent");
        // The jump: near some body, at a few radii, at a speed about circular.
        game.slip_jump();
        let t = game.state.time_s;
        let bodies = game.params.bodies(t);
        let vels = game.params.body_velocities(t);
        let (i, b) = bodies
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, c)| {
                let da = (game.state.ship.pos_m - a.centre).length() / a.radius_m;
                let dc = (game.state.ship.pos_m - c.centre).length() / c.radius_m;
                da.partial_cmp(&dc).unwrap()
            })
            .unwrap();
        let r = (game.state.ship.pos_m - b.centre).length();
        assert!(
            r > b.radius_m * 1.7 && r < b.radius_m * 6.1,
            "{}",
            r / b.radius_m
        );
        let v = (game.state.ship.vel_mps - vels[i]).length();
        let vc = (b.mu / r).sqrt();
        assert!(v > vc * 0.45 && v < vc * 1.45, "{} of circular", v / vc);
        assert_ne!(game.state.ship.pos_m, home);
        // Let go of a running field: launched at that speed, nothing taken.
        let mut g2 = Game::new();
        g2.state.ship.vel_mps = glam::DVec3::new(0.0, 0.0, -2.0e7);
        g2.hyper_was = true;
        g2.run_hyper_strain(0.016);
        assert_eq!(g2.state.ship.vel_mps.length(), 2.0e7);
        // WARP STOP takes it all out and leaves the image behind.
        g2.state.ship.ang_vel_radps = glam::DVec3::new(0.5, 0.0, 0.0);
        g2.warp_stop();
        assert!(
            g2.state.ship.vel_mps.length() < 1.0,
            "{:?}",
            g2.state.ship.vel_mps
        );
        assert_eq!(g2.state.ship.ang_vel_radps, glam::DVec3::ZERO);
        let ghost = g2.ghost.expect("an after-image");
        assert!((ghost.dir_world - glam::DVec3::NEG_Z).length() < 1e-9);
        // Chaos: calm to halfway, shaking hard near the slip.
        assert_eq!(chaos_level(0.2, 1.0), 0.0);
        assert!(chaos_level(0.75, 1.0) > 0.1 && chaos_level(0.75, 1.0) < 0.5);
        assert_eq!(chaos_level(1.0, 1.0), 1.0);
        // Off, the strain eases.
        let mut g3 = Game::new();
        g3.hyper_strain = 0.5;
        for _ in 0..600 {
            g3.run_hyper_strain(0.1);
        }
        assert_eq!(g3.hyper_strain, 0.0);
    }

    #[test]
    fn sim_directions() {
        // Translation: compare against an unthrusted control run so gravity —
        // which pulls toward the planet regardless of input — cancels out and
        // only the thrust contribution remains.
        let coast = fly(&[], 1).vel_mps;
        let cases: [(KeyCode, DVec3, &str); 6] = [
            (KeyCode::KeyW, DVec3::NEG_Z, "W thrusts along the nose"),
            (KeyCode::KeyS, DVec3::Z, "S thrusts backward"),
            (KeyCode::KeyD, DVec3::X, "D strafes right"),
            (KeyCode::KeyA, DVec3::NEG_X, "A strafes left"),
            (KeyCode::KeyR, DVec3::Y, "R thrusts up"),
            (KeyCode::KeyF, DVec3::NEG_Y, "F thrusts down"),
        ];
        for (key, want, what) in cases {
            let v = (fly(&[key], 1).vel_mps - coast).normalize();
            assert!(
                (v - want).length() < 1e-4,
                "{what}: got {v:?}, want {want:?}"
            );
        }

        // Rotation: after a short burn the named axis must have moved the right way.
        let nose_up = fly(&[KeyCode::ArrowUp], 1).orient * DVec3::NEG_Z;
        assert!(
            nose_up.y > 0.05,
            "Up arrow must pitch the nose up: {nose_up:?}"
        );

        let nose_down = fly(&[KeyCode::ArrowDown], 1).orient * DVec3::NEG_Z;
        assert!(
            nose_down.y < -0.05,
            "Down arrow must pitch the nose down: {nose_down:?}"
        );

        let nose_right = fly(&[KeyCode::ArrowRight], 1).orient * DVec3::NEG_Z;
        assert!(
            nose_right.x > 0.05,
            "Right arrow must yaw the nose right: {nose_right:?}"
        );

        let nose_left = fly(&[KeyCode::ArrowLeft], 1).orient * DVec3::NEG_Z;
        assert!(
            nose_left.x < -0.05,
            "Left arrow must yaw the nose left: {nose_left:?}"
        );

        // Roll right = right wing down = the up vector tips toward +X.
        let up_right = fly(&[KeyCode::KeyE], 1).orient * DVec3::Y;
        assert!(
            up_right.x > 0.05,
            "E must roll right: up tipped to {up_right:?}"
        );

        let up_left = fly(&[KeyCode::KeyQ], 1).orient * DVec3::Y;
        assert!(
            up_left.x < -0.05,
            "Q must roll left: up tipped to {up_left:?}"
        );
    }

    /// A rotation must not bleed into the other two axes.
    #[test]
    fn rotation_axes_do_not_cross_couple() {
        for (key, axis) in [
            (KeyCode::ArrowUp, 0usize),
            (KeyCode::ArrowRight, 1),
            (KeyCode::KeyE, 2),
        ] {
            let w = fly(&[key], 1).ang_vel_radps;
            for other in 0..3 {
                if other != axis {
                    assert!(
                        w[other].abs() < 1e-12,
                        "{key:?} leaked {} rad/s into axis {other}",
                        w[other]
                    );
                }
            }
        }
    }

    /// The nose starts pointing along the velocity: the pilot begins looking
    /// where they are going, not backwards.
    #[test]
    fn orbit_preset_starts_nose_prograde() {
        let params = sim::presets::earth_compact();
        let s = sim::presets::circular_orbit(&params, 20_000.0).ship;
        let nose = s.orient * DVec3::NEG_Z;
        let prograde = s.vel_mps.normalize();
        assert!(
            (nose - prograde).length() < 1e-9,
            "nose {nose:?} is not prograde {prograde:?}"
        );
    }

    /// The planet must be in frame on the very first frame. Getting the spawn
    /// attitude right is pure geometry and I got it wrong twice by reasoning
    /// about it in prose, so it is asserted instead: the angle from the camera's
    /// forward axis to the planet's centre, minus the disc's angular radius,
    /// has to land inside the vertical half-FOV.
    #[test]
    fn planet_is_in_view_at_spawn() {
        let game = Game::new();
        let cam = game.camera(16.0 / 9.0);
        let forward = cam.basis().2.as_dvec3();

        let to_planet = (-game.state.ship.pos_m).normalize();
        let angle = forward.dot(to_planet).clamp(-1.0, 1.0).acos();

        let r = game.params.planet.radius_m;
        let d = game.state.ship.pos_m.length();
        let angular_radius = (r / d).asin();

        let half_fov = (cam.fov_y as f64) * 0.5;
        assert!(
            angle - angular_radius < half_fov,
            "planet off screen at spawn: centre {:.1} deg from forward, disc radius \
             {:.1} deg, half-FOV {:.1} deg",
            angle.to_degrees(),
            angular_radius.to_degrees(),
            half_fov.to_degrees(),
        );
        // ...and the *limb* must be on screen, not just some surface: a frame
        // filled edge to edge with ground reads as terrain, not as a world. The
        // horizon crossing the view is what sells the curve.
        let limb_offset = (angle - angular_radius).abs();
        assert!(
            limb_offset < half_fov,
            "horizon is off screen: limb sits {:.1} deg from forward, half-FOV {:.1}",
            (angle - angular_radius).to_degrees(),
            half_fov.to_degrees(),
        );
    }

    /// The ship starts flying where it is looking. With the flight computer
    /// steering velocity toward the nose, any large angle between the two is a
    /// trajectory change the pilot never asked for — so the spawn attitude must
    /// be close to prograde or the ship immediately departs its own orbit.
    #[test]
    fn spawn_flies_where_it_looks() {
        let game = Game::new();
        let nose = game.state.ship.orient * DVec3::NEG_Z;
        let prograde = game.state.ship.vel_mps.normalize();
        let off = nose.dot(prograde).clamp(-1.0, 1.0).acos().to_degrees();
        assert!(off < 20.0, "nose is {off:.1} deg off prograde at spawn");
    }

    /// The planet is underfoot, not overhead: the ship is the right way up.
    #[test]
    fn look_at_points_the_nose_where_asked() {
        for dir in [DVec3::X, DVec3::NEG_Y, DVec3::new(1.0, 2.0, -3.0)] {
            let q = look_at(dir, DVec3::Y);
            let nose = q * DVec3::NEG_Z;
            assert!((nose - dir.normalize()).length() < 1e-9, "{dir:?}");
            assert!((q.length() - 1.0).abs() < 1e-9);
        }
        assert_eq!(parse_vec3("1, -2,3.5"), Some(DVec3::new(1.0, -2.0, 3.5)));
        assert_eq!(parse_vec3("1,2"), None);
        assert_eq!(parse_vec3("1,2,3,4"), None);
    }

    #[test]
    fn spawn_attitude_puts_the_planet_below() {
        let game = Game::new();
        let down = (game.state.ship.orient * DVec3::NEG_Y).normalize();
        let to_planet = (-game.state.ship.pos_m).normalize();
        assert!(
            down.dot(to_planet) > 0.8,
            "planet is not below the ship: {:.2}",
            down.dot(to_planet)
        );
    }

    #[test]
    fn thrust_is_ship_relative_at_any_attitude() {
        let params = sim::presets::earth_compact();
        let attitudes = [
            DQuat::IDENTITY,
            DQuat::from_rotation_y(std::f64::consts::FRAC_PI_2),
            DQuat::from_rotation_x(-1.1),
            DQuat::from_euler(glam::EulerRot::YXZ, 2.3, -0.9, 1.7).normalize(),
        ];
        for (key, body_axis) in [
            (KeyCode::KeyW, DVec3::NEG_Z),
            (KeyCode::KeyD, DVec3::X),
            (KeyCode::KeyR, DVec3::Y),
        ] {
            for orient in attitudes {
                let mut state = sim::presets::circular_orbit(&params, 60_000.0);
                state.ship.orient = orient;
                state.ship.vel_mps = DVec3::ZERO;
                let mut input = InputState::default();
                input.set(key, true);

                let coast = {
                    let mut s = state;
                    for _ in 0..120 {
                        s = sim::step(&params, &s, InputState::default().controls(false));
                    }
                    s.ship.vel_mps
                };
                let mut s = state;
                for _ in 0..120 {
                    s = sim::step(&params, &s, input.controls_immediate(false));
                }

                // Gravity cancels; what remains must lie along the ship's axis.
                let delta = (s.ship.vel_mps - coast).normalize();
                let expected = (orient * body_axis).normalize();
                assert!(
                    (delta - expected).length() < 1e-4,
                    "{key:?} at attitude {orient:?}: thrust went {delta:?}, expected {expected:?}"
                );
            }
        }
    }

    /// Boost multiplies thrust without steering it somewhere new.
    #[test]
    fn boost_adds_thrust_along_the_same_axis() {
        let params = sim::presets::earth_compact();
        let mut base = sim::presets::circular_orbit(&params, 60_000.0);
        base.ship.orient = DQuat::IDENTITY;
        base.ship.vel_mps = DVec3::ZERO;

        let run = |boost: bool| {
            let mut input = InputState::default();
            input.set(KeyCode::KeyW, true);
            if boost {
                input.set(KeyCode::ShiftLeft, true);
            }
            let controls = input.controls_immediate(false);
            let mut s = base;
            for _ in 0..120 {
                s = sim::step(&params, &s, controls);
            }
            s.ship.vel_mps
        };
        let coast = {
            let mut s = base;
            for _ in 0..120 {
                s = sim::step(&params, &s, InputState::default().controls(false));
            }
            s.ship.vel_mps
        };

        let plain = run(false) - coast;
        let boosted = run(true) - coast;
        let ratio = boosted.length() / plain.length();
        assert!(
            (ratio - params.ship.boost_multiplier).abs() < 0.01,
            "boost gave {ratio:.2}x, expected {:.2}x",
            params.ship.boost_multiplier
        );
        // Same direction, only more of it.
        assert!((boosted.normalize() - plain.normalize()).length() < 1e-6);
    }

    #[test]
    fn action_count_matches_bindings() {
        assert_eq!(Action::COUNT, 12);
    }

    /// A little of everything a save is meant to carry, mutated away from
    /// `Game::new`'s defaults so a round trip that silently dropped a
    /// field would show up as a mismatch rather than an accidental match.
    fn mutate_for_save_tests(game: &mut Game) {
        game.state.ship.pos_m = DVec3::new(1.0e7, 2.0e6, -3.0e5);
        game.state.ship.vel_mps = DVec3::new(120.0, -5.0, 30.0);
        game.state.ship.orient = DQuat::from_euler(glam::EulerRot::YXZ, 0.4, -0.2, 0.1).normalize();
        game.state.ship.ang_vel_radps = DVec3::new(0.01, 0.0, -0.02);
        game.state.time_s = 4_321.0;
        game.assist = false;
        game.landing = true;
        game.appearance_index = 2;
        game.hyper_strain = 0.35;
        game.slip_at = 0.77;
        game.jumps = 5;
        game.arms.selected = arms::Weapon::Rail;
        game.arms.ammo = [400, 10];
        game.arms.jammed = [true, false];
        game.arms.heat = [0.6, 0.2];
        game.arms.charge = 0.9;
        game.haul.tonnes = [1.0, 2.0, 3.0, 4.0];
        game.mimics.hull = 0.6;
        game.strikes = 9;
        game.strike_rng = 0xDEAD_BEEF;
        game.odometer_m = 12_345.0;
        game.hoops_passed = 7;
        game.belt.dead.insert((1, 2, 3, 0));
        game.belt.wounds.insert((1, 2, 3, 0), 555.0);
        game.mimics.revealed.insert((4, 5, 6, 1));
        game.mimics.ships.push(mimic::Mimic::planted(
            DVec3::new(10.0, 0.0, 0.0),
            DVec3::new(1.0, 0.0, 0.0),
            DQuat::IDENTITY,
            game.state.time_s,
            mimic::Phase::Hailing,
            mimic::Mood::Hail,
            0.42,
        ));
    }

    #[test]
    fn a_saved_world_comes_back_bit_for_bit() {
        let mut game = Game::new();
        mutate_for_save_tests(&mut game);

        let saved = game.snapshot();
        let parsed = save::Save::parse(&saved.render()).expect("a fresh snapshot always parses");
        assert_eq!(parsed, saved, "render/parse must be lossless");

        let mut resumed = Game::new();
        resumed.restore(&parsed);

        assert_eq!(
            sim::state_hash(&resumed.state),
            sim::state_hash(&game.state)
        );
        assert_eq!(resumed.assist, game.assist);
        assert_eq!(resumed.landing, game.landing);
        assert_eq!(resumed.appearance_index, game.appearance_index);
        assert_eq!(resumed.hyper_strain, game.hyper_strain);
        assert_eq!(resumed.slip_at, game.slip_at);
        assert_eq!(resumed.jumps, game.jumps);
        assert_eq!(resumed.arms.selected, game.arms.selected);
        assert_eq!(resumed.arms.ammo, game.arms.ammo);
        assert_eq!(resumed.arms.jammed, game.arms.jammed);
        assert_eq!(resumed.arms.heat, game.arms.heat);
        assert_eq!(resumed.arms.charge, game.arms.charge);
        assert_eq!(resumed.haul.tonnes, game.haul.tonnes);
        assert_eq!(resumed.mimics.hull, game.mimics.hull);
        assert_eq!(resumed.strikes, game.strikes);
        assert_eq!(resumed.strike_rng, game.strike_rng);
        assert_eq!(resumed.odometer_m, game.odometer_m);
        assert_eq!(resumed.hoops_passed, game.hoops_passed);
        assert_eq!(resumed.belt.dead, game.belt.dead);
        assert_eq!(resumed.belt.wounds, game.belt.wounds);
        assert_eq!(resumed.mimics.revealed, game.mimics.revealed);
        assert_eq!(resumed.mimics.ships.len(), game.mimics.ships.len());
        assert_eq!(resumed.mimics.ships[0].pos, game.mimics.ships[0].pos);
        assert_eq!(resumed.mimics.ships[0].phase, game.mimics.ships[0].phase);
        assert_eq!(resumed.mimics.ships[0].mood, game.mimics.ships[0].mood);
    }

    #[test]
    fn a_resumed_world_runs_on_exactly_as_the_uninterrupted_one() {
        let mut original = Game::new();
        let controls = sim::Controls {
            thrust_body: DVec3::new(0.3, 0.0, -0.6),
            torque_body: DVec3::new(0.1, -0.05, 0.02),
            assist: true,
            ..Default::default()
        };
        for _ in 0..50 {
            original.state = sim::step(&original.params, &original.state, controls);
        }
        let saved = original.snapshot();

        let mut resumed = Game::new();
        resumed.restore(&saved);
        assert_eq!(
            sim::state_hash(&resumed.state),
            sim::state_hash(&original.state),
            "restore itself must be exact before either runs on"
        );

        for _ in 0..30 {
            original.state = sim::step(&original.params, &original.state, controls);
            resumed.state = sim::step(&resumed.params, &resumed.state, controls);
        }
        assert_eq!(
            sim::state_hash(&resumed.state),
            sim::state_hash(&original.state),
            "the same controls from the same state must land the same hash"
        );
    }

    #[test]
    fn the_haul_the_hull_the_ammo_and_the_dead_rocks_survive_a_quit() {
        let mut game = Game::new();
        mutate_for_save_tests(&mut game);
        let saved = game.snapshot();
        let mut resumed = Game::new();
        resumed.restore(&saved);

        assert_eq!(resumed.haul.tonnes, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(resumed.mimics.hull, 0.6);
        assert_eq!(resumed.arms.ammo, [400, 10]);
        assert_eq!(resumed.arms.jammed, [true, false]);
        assert!(resumed.belt.dead.contains(&(1, 2, 3, 0)));
        assert_eq!(resumed.belt.wounds.get(&(1, 2, 3, 0)), Some(&555.0));
    }

    #[test]
    fn a_warp_in_flight_and_a_hold_come_back_idle() {
        let mut game = Game::new();
        game.warp.engage();
        assert!(game.warp.active(), "test setup: the drive is mid-sequence");
        game.hold.target = Some((hold::Target::Rock((1, 2, 3, 0)), DVec3::new(10.0, 0.0, 0.0)));
        assert!(game.hold.engaged(), "test setup: the lock is on");

        let saved = game.snapshot();
        let mut resumed = Game::new();
        // Give it something to be idle FROM, so this is not just "a fresh
        // Game::new() happens to be idle".
        resumed.warp.engage();
        resumed.hold.target = Some((hold::Target::Rock((9, 9, 9, 0)), DVec3::ZERO));
        resumed.restore(&saved);

        assert!(
            !resumed.warp.active(),
            "a warp in flight is never resumed mid-jump"
        );
        assert!(!resumed.hold.engaged(), "a hold lock never survives a quit");
    }

    #[test]
    fn new_game_forgets_the_world_and_stands_at_the_stock_orbit() {
        let fresh_hash = sim::state_hash(&Game::new().state);

        let mut game = Game::new();
        mutate_for_save_tests(&mut game);
        assert_ne!(
            sim::state_hash(&game.state),
            fresh_hash,
            "test setup: the mutation actually moved the ship"
        );

        game.reset_world();
        assert_eq!(sim::state_hash(&game.state), fresh_hash);
        assert_eq!(game.arms.ammo, arms::Arms::default().ammo);
        assert!(game.belt.dead.is_empty());
        assert!(game.mimics.ships.is_empty());
    }

    #[test]
    fn new_game_keeps_the_pilots_settings_look_and_open_menu() {
        let mut game = Game::new();
        let mut settings = game.settings;
        settings.fov = 95.0;
        game.apply_settings(settings);
        game.look.sensitivity = 3.0;
        game.menu.toggle();
        assert!(game.menu.open, "test setup: the menu is open");

        game.reset_world();
        assert_eq!(game.settings.fov, 95.0, "settings survive a new game");
        assert_eq!(game.look.sensitivity, 3.0, "look survives a new game");
        assert!(game.menu.open, "the open menu itself survives a new game");
    }

    #[test]
    fn resume_off_or_a_bench_run_never_touches_the_world_file() {
        assert!(resume_allowed(true, false, None, false));
        assert!(!resume_allowed(false, false, None, false), "RESUME off");
        assert!(!resume_allowed(true, true, None, false), "frozen (a bench)");
        assert!(
            !resume_allowed(true, false, None, true),
            "a bench spawn override on its own, without FARFALL_BENCH itself"
        );
        for off in ["0", "off", "false"] {
            assert!(!resume_allowed(true, false, Some(off), false), "{off}");
        }
        assert!(
            resume_allowed(true, false, Some("1"), false),
            "a non-off value neither forces it on nor off"
        );
        assert!(
            !resume_allowed(false, false, Some("1"), false),
            "the environment can turn resume off but never force it on over the setting"
        );
    }

    #[test]
    fn bench_save_and_resume_knobs_only_ever_act_during_a_bench() {
        use std::ffi::OsStr;
        let path = Some(OsStr::new("C:\\tmp\\w.cfg"));
        assert!(bench_path_action_allowed(true, path), "bench + a path");
        assert!(
            !bench_path_action_allowed(false, path),
            "a path with no bench running does nothing"
        );
        assert!(
            !bench_path_action_allowed(true, None),
            "a bench with no path set does nothing"
        );
        assert!(!bench_path_action_allowed(false, None));
    }

    /// The mechanics `FARFALL_BENCH_SAVE`/`FARFALL_BENCH_RESUME` lean on:
    /// an explicit path, never `~/.farfall`, through the same seal as a
    /// real resume — including refusing a tampered file at that path.
    #[test]
    fn store_to_and_load_from_an_explicit_path_round_trip_and_refuse_tampering() {
        let mut game = Game::new();
        mutate_for_save_tests(&mut game);
        let path = std::env::temp_dir().join(format!(
            "farfall-bench-save-test-{}.cfg",
            std::process::id()
        ));
        let _cleanup = TempFileGuard(path.clone());

        game.snapshot().store_to(&path);
        let loaded = save::load_from(&path).expect("a freshly stored save loads back");
        assert_eq!(loaded, game.snapshot());

        let tampered = std::fs::read_to_string(&path).unwrap().replacen(
            "world.version = 1",
            "world.version = 99",
            1,
        );
        std::fs::write(&path, tampered).unwrap();
        assert_eq!(
            save::load_from(&path),
            None,
            "a tampered file at an explicit path is refused, same as ~/.farfall"
        );
    }

    /// Removes the file it names when dropped, so a test that writes to
    /// the real filesystem (an explicit-path save, never `~/.farfall`)
    /// never leaves anything behind, pass or fail.
    struct TempFileGuard(std::path::PathBuf);
    impl Drop for TempFileGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}
