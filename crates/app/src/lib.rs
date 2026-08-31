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
mod hold;
mod input;
mod landing;
mod look;
mod map;
mod menu;
mod mimic;
mod miner;
mod panel;
mod readout;
mod settings;
mod shake;
mod warp;

use cockpit::Instrument;
use look::Look;
use menu::{Change, Menu, MenuEvent};
use settings::Settings;
use warp::Warp;
mod telemetry;
#[cfg(target_arch = "wasm32")]
pub mod web;

use glam::{DQuat, DVec3, Quat, Vec3};
use std::sync::Arc;
use web_time::{Duration, Instant};

use capture::Capture;
use farfall_audio::Audio;
use farfall_render::debris::{debris_pass, DebrisPass, DebrisScene, DebrisUniforms, ShardView};
use farfall_render::dust::{DustPass, DustScene, DustUniforms};
use farfall_render::mimic::{mimic_pass, MimicPass, MimicUniforms, MimicView};
use farfall_render::scar::{scar_heat, scar_pass, ScarPass, ScarScene, ScarUniforms, ScarView};
use farfall_render::sight::{sight_pass, SightPass, SightScene, SightUniforms};
use farfall_render::tracer::{
    tracer_pass, BurstView, Occluder, SlugView, TracerPass, TracerScene, TracerUniforms,
};
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
        MountView,
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
        MenuEvent::Quit => {
            game.log_exit("menu quit");
            event_loop.exit();
        }
        MenuEvent::Engage => {
            game.settings.save();
            game.engage_warp();
        }
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
/// A glass anchor as the turned head sees it: the glass is a sphere about
/// the pilot, so every element is re-projected, not slid (look.rs). The
/// anchors are laid out in a REFERENCE projection — the pilot's base field
/// of view, head centred — and shown through the live one, so a throttle
/// flare or a warp does not slide them over the ship.
fn on_glass(look: &Look, cam: &CameraFrame, ref_tan: f32, a: [f32; 2]) -> [f32; 2] {
    look.reproject_from(a, ref_tan, (cam.fov_y * 0.5).tan(), cam.aspect)
}

/// Where a dial sits and whether it shows: a hidden instrument keeps any
/// anchor and a visibility of zero. The anchors themselves are the slots
/// in `cockpit.rs` (or wherever the pilot dragged it); the menu assigns
/// them.
fn slot_of(
    layout: &cockpit::Layout,
    look: &Look,
    cam: &CameraFrame,
    ref_tan: f32,
    i: Instrument,
) -> ([f32; 2], f32) {
    match layout.anchor(i) {
        Some(a) => (on_glass(look, cam, ref_tan, a), 1.0),
        None => ([0.0, 0.0], 0.0),
    }
}
/// The Chaos Drive's limits: seconds of full running before the drive
/// slips (the slip point itself is drawn between 70% and 100% of this —
/// the pilot never knows exactly), and how long the entropy takes to
/// ease off once the field is down.
const HYPER_STRAIN_S: f32 = 40.0;
const HYPER_EASE_S: f32 = 90.0;

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
            Dragged::HoloPanel => "HOLO3PP",
            Dragged::BayPanel => "SHIP BAY",
        }
    }
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
///                           being mistaken for a game with broken controls.)
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
///   FARFALL_BENCH_SHIP=1   (benchmark only: open the SHIP bay for a capture)
///   FARFALL_BENCH_STYLE=k  (benchmark only: the cockpit's gauge style by key — tron, jet, dial, warthog)
///   FARFALL_BENCH_MAP=1    (benchmark only: open the MAP page at once)
///   FARFALL_BENCH_HEAD=y,p (benchmark only: turn the head yaw,pitch degrees)
///   FARFALL_BENCH_LAND=1   (benchmark only: LANDING mode on)
///   FARFALL_BENCH_LANDED=1 (benchmark only: parked on the ground, LANDED,
///                           on its gear with the Sun up the sky)
///   FARFALL_BENCH_DISEMBARK=1 (benchmark only: DISEMBARK pressed at once,
///                           for its answer on the readout)
///   FARFALL_BENCH_DESIGN=1 (benchmark only: DESIGN mode on)
///   FARFALL_BENCH_MENU=n   (benchmark only: the settings menu open, paged n times)
///   FARFALL_BENCH_CARD=1   (benchmark only: the CONTROLS card up, as on the first run)
///   FARFALL_BENCH_HYPER=1  (benchmark only: the hyper drive's field fully up)
///   FARFALL_BENCH_NEBULA=1|off (benchmark only: a full sky of nebula at
///                           twice the stock glow, or off for a baseline)
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
///   FARFALL_MUTE=1         (no audio stream at all)
///   FARFALL_BENCH_WARP=s   (benchmark only: engage the wormhole drive s
///                           seconds in, so the sequence can be captured)
///   FARFALL_SKIP=a,b       (profiling only: leave out passes by name —
///                           starfield, bodies, planet, plasma, trajectory, cockpit, gauge,
///                           post (the picture: one plain fetch instead), bloom (the chain),
///                           hud, blit, dust —
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
        Self {
            msaa,
            vsync,
            gpu_sync,
            windowed,
            bench,
            bench_seconds,
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
    /// The arms' scars: craters glowing on the rocks.
    scar: ScarPass,
    /// The gun sight on the glass.
    sight: SightPass,
    /// The mimics: ships out of the rocks.
    mimic: MimicPass,
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
            scar: scar_pass(device, world, msaa),
            sight: sight_pass(device, format, msaa),
            mimic: mimic_pass(device, world, msaa),
        }
    }
}

impl Gpu {
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
    window: Arc<Window>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    scene: SceneTarget,
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

impl Gpu {
    /// Every dial and overlay, from the cockpit layout: an instrument whose
    /// slot is Off gets visibility zero and draws nothing at all.
    fn update_instruments(&self, game: &Game, cam: &CameraFrame, aspect: f32, altitude_m: f32) {
        let layout = &game.settings.layout;
        let h = self.scene.size().1 as f32;
        let sway = game.holo_sway.sway();
        let look = &game.look;
        let ref_tan = game.ref_tan();
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
                if let Some(p) =
                    farfall_render::cabin::Placement::in_dash(head, t, dir, tw.size, tw.tilt)
                {
                    return Some(p);
                }
            }
            Some(farfall_render::cabin::Placement::glass_sized(tw.size * fov_scale).tilted(tw.tilt))
        };
        let (speed_anchor, speed_on) = slot_of(layout, look, cam, ref_tan, Instrument::Speed);
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
            .placed(placed(Instrument::Speed)),
        );
        let (alt_anchor, alt_on) = slot_of(layout, look, cam, ref_tan, Instrument::Altitude);
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
            .placed(placed(Instrument::Altitude)),
        );
        let (g_anchor, g_on) = slot_of(layout, look, cam, ref_tan, Instrument::GForce);
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
            .placed(placed(Instrument::GForce)),
        );
        let (gv_anchor, gv_on) = slot_of(layout, look, cam, ref_tan, Instrument::GVector);
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
            .placed(placed(Instrument::GVector)),
        );
        let (gyro_anchor, gyro_on) = slot_of(layout, look, cam, ref_tan, Instrument::Gyro);
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
            .ball_if(game.gyro_ball(cam, tweak(Instrument::Gyro))),
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
            let anchors: Vec<[f32; 2]> = Instrument::ALL
                .iter()
                .copied()
                .filter(|i| i.slotted())
                .filter_map(|i| layout.anchor(i))
                .map(|a| on_glass(look, cam, ref_tan, a))
                .take(6)
                .collect();
            self.passes.guide.update(
                &self.queue,
                &GuideUniforms::new(
                    aspect,
                    game.settings.guide || game.design,
                    layout.safe_edge,
                    on_glass(look, cam, ref_tan, gaze),
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
        if settings.auto_scale {
            (settings.scale * self.auto_scale).clamp(AUTO_SCALE_MIN, 1.0)
        } else {
            settings.scale
        }
    }

    /// AUTO SCALE: hold the FPS floor with the world's resolution — the
    /// one cost that scales with every pass at once — leaving the HUD,
    /// the dials and the text at native size. A miss drops the scale a
    /// step at a time; room above the floor, held for a while, brings
    /// it back. Vsync pins the rate at the floor, so "room" is the rate
    /// sitting on the floor with no slow frames under it.
    fn govern_scale(&mut self, settings: &Settings, fps_floor: f32) {
        if !settings.auto_scale || fps_floor <= 0.0 {
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
        fps_floor: f32,
        readout: &Readout,
    ) {
        let Readout {
            altitude_m,
            speed_mps,
            assist,
            show: show_readout,
            landing,
        } = readout;
        let (altitude_m, speed_mps, assist, show_readout) =
            (*altitude_m, *speed_mps, *assist, *show_readout);
        self.perf.cpu.record(cpu_seconds);
        self.perf.wait.record(wait_seconds);
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
                    status: landing.clone(),
                },
            );
        }

        if now.duration_since(self.perf.last_log) >= PERF_LOG_EVERY {
            self.perf.last_log = now;
            if let Some(s) = self.perf.stats.take_summary() {
                log::info!(
                    "perf {}x{} {}xMSAA vsync={} gpu_sync={}: {:.1} fps avg \
                     | 1% low {:.1} fps | frame avg {:.2}ms worst {:.2}ms \
                     best {:.2}ms | {} frames",
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
    /// The mimics — ships in the rocks — and what the guns bring in.
    mimics: mimic::Mimics,
    haul: mimic::Haul,
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
}

impl Game {
    fn new() -> Self {
        let params = sim::presets::earth_compact();
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
        let bench_landed = std::env::var("FARFALL_BENCH_LANDED").is_ok();
        if bench_landed {
            state.ship = landing::parked(&params, 0);
        }
        let now = Instant::now();
        Self {
            vr: None,
            vr_eye: 0,
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
            card_open: false,
            landing: false,
            touchdown: None,
            touchdown_record: bench_landed.then(landing::Record::sample),
            disembark_notice: None,
            cursor: None,
            window_size: (1.0, 1.0),
            left_down: false,
            warp: Warp::new(),
            hyper: 0.0,
            ghost: None,
            belt: belt::Belt::default(),
            arms: arms::Arms::default(),
            fire_held: false,
            mimics: mimic::Mimics::default(),
            haul: mimic::Haul::default(),
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
        let mini = !self.map_open() && self.mini_map_on();
        let look = map::MapLook {
            view: self.map_view,
            rings: self.settings.map_rings,
            grid: self.settings.map_grid,
            visibility: if self.map_open() || mini { 1.0 } else { 0.0 },
            aspect,
            time_s,
            centre: if mini {
                let cam = self.camera(aspect);
                let a = self.settings.layout.inset(map::MINI_ANCHOR);
                on_glass(&self.look, &cam, self.ref_tan(), a)
            } else {
                self.settings.map_anchor
            },
            half_h: if mini {
                map::MINI_HALF_H
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

    /// The mini map is a stock gauge: shown on the glass while no panel
    /// covers it and the view is the cockpit's.
    fn mini_map_on(&self) -> bool {
        self.settings.layout.shown(Instrument::Map) && !self.panel_open() && !self.chase_active()
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
        let mut rows = vec![BayRow::Header];
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
                Some(BayRow::Slot(i)) => {
                    self.bay_panel.set_cursor(i);
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
            self.bay_panel.set_cursor(i);
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
        let mut mounts = [MountView {
            at: Vec3::ZERO,
            kind: 0,
        }; farfall_render::hologram::HARDPOINTS];
        for ((m, h), fit) in mounts
            .iter_mut()
            .zip(bay::Hardpoint::ALL.iter())
            .zip(self.settings.mounts.iter())
        {
            *m = MountView {
                at: h.pos().as_vec3(),
                kind: fit.kind(),
            };
        }
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
            // Beside the selected dial, or where the readout lives.
            return match self
                .design_target(aspect)
                .and_then(|i| self.settings.layout.anchor(i))
            {
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
                farfall_render::cabin::Socket {
                    dir: anchor_direction(a, ref_tan, cam.aspect),
                    // The gyro's JET and WARTHOG are the ball itself.
                    style: if i == Instrument::Gyro
                        && matches!(
                            tw.style,
                            settings::GaugeStyle::Jet | settings::GaugeStyle::Warthog
                        ) {
                        3
                    } else if tw.style == settings::GaugeStyle::Warthog {
                        // The Warthog's face sits on the DIAL's plate.
                        2
                    } else {
                        tw.style.index()
                    },
                    size: tw.size,
                    tilt: tw.tilt,
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
        let cu = CabinUniforms::new(cam, self.head(), sun_ship, look, &sockets);
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
    /// panels sit on the screen and follow the head; the readout and the
    /// design card are on the glass, re-projected like a dial.
    fn text_screen_anchor(&self, cam: &CameraFrame, px: f32) -> [f32; 2] {
        let a = self.text_anchor(cam.aspect, px);
        if self.menu.open || self.pane_open() || self.card_open {
            a
        } else {
            on_glass(&self.look, cam, self.ref_tan(), a)
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
        if self.menu.open {
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
    fn gyro_ball(
        &self,
        cam: &CameraFrame,
        tw: DialEffective,
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
        let dir = anchor_direction(a, self.ref_tan(), cam.aspect);
        let t = (cam.fov_y * 0.5).tan();
        let place = farfall_render::cabin::Placement::ball(self.head(), t, dir, tw.size)?;
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
        (self.ref_tan() / t).clamp(0.4, 1.25)
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

    /// The dial under the pointer (within reach), in design mode.
    fn design_target(&self, aspect: f32) -> Option<Instrument> {
        let gaze = self.design_pointer(aspect)?;
        let mut best: Option<(Instrument, f32)> = None;
        for i in Instrument::ALL.iter().copied().filter(|i| i.slotted()) {
            if let Some(a) = self.settings.layout.anchor(i) {
                let dx = (a[0] - gaze[0]) * aspect;
                let dy = a[1] - gaze[1];
                let d = (dx * dx + dy * dy).sqrt();
                if d < DRAG_REACH && best.is_none_or(|b| d < b.1) {
                    best = Some((i, d));
                }
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

    /// A key in design mode: the selected dial's own settings.
    fn design_key(&mut self, code: KeyCode, aspect: f32) {
        let Some(i) = self.design_target(aspect) else {
            return;
        };
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

    /// The design card: the selected dial's settings, a few short lines,
    /// and the keys.
    fn design_text(&self, aspect: f32) -> Vec<String> {
        match self.design_target(aspect) {
            Some(i) => {
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
                    "- = SIZE  TAB STYLE  F FADE".to_string(),
                    ", . TILT  BKSP RESET  K DONE".to_string(),
                ]
            }
            None => vec![
                "[DESIGN]".to_string(),
                "LOOK AT A DIAL".to_string(),
                "CLICK DRAG TO MOVE".to_string(),
                "K DONE".to_string(),
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

    /// DISEMBARK (I): leave the ship. Today the answer is "not yet" — the
    /// bind, the state and the readout are the hook for the walk-out.
    fn disembark(&mut self) {
        let notice = landing::disembark_notice(self.state.ship.ground);
        self.disembark_notice = Some((notice, Instant::now()));
        log::info!("disembark: {notice}");
    }

    /// The landing readout lines, newline-joined, if there are any: the
    /// approach in LANDING mode; DOWN or LANDED whenever the ship is on
    /// the ground, mode or no mode.
    fn landing_text(&self) -> Option<String> {
        let (altitude_m, vspeed_mps) = self.altitude_vspeed();
        let notice = self
            .disembark_notice
            .filter(|(_, at)| at.elapsed() < Duration::from_secs(4))
            .map(|(n, _)| n);
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
        let held = self.input.controls(self.assist).hyper && !self.warp.active();
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
        if self.mimics.ships.is_empty() && self.miners.ships.is_empty() {
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
        .with_body_fill(body_dir, body_sin * body_sin);
        if pose.eye_ship == DVec3::ZERO {
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
                || (self.input.controls(self.assist).hyper && !self.warp.active())
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
        // Through a jump the stick is dead: the drive has the ship.
        let mut controls = if self.warp.active() {
            sim::Controls {
                assist: self.assist,
                ..Default::default()
            }
        } else {
            self.input.controls(self.assist)
        };
        controls.hyper_level = self.hyper_level();
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
            let trigger = self.fire_held && !self.menu.open && !self.map_open() && !self.design;
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

    /// This frame's eye on the world: the cockpit's, or the chase rig's
    /// when the CAMERA setting says so.
    fn pose(&self, aspect: f32) -> ViewPose {
        if self.chase_active() {
            self.chase_pose(aspect)
        } else {
            let mut pose = ViewPose {
                cam: self.cam_for(aspect, self.head()),
                head: self.head(),
                eye_ship: DVec3::ZERO,
            };
            if let Some(vr) = &self.vr {
                let eye = vr.eyes[self.vr_eye.min(1)];
                let (fov_y, aspect) = eye.symmetric();
                pose.cam.fov_y = fov_y;
                pose.cam.aspect = aspect;
                pose.eye_ship = eye.pos.as_dvec3();
            }
            pose
        }
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

    /// The key a named control answers to right now.
    fn bind(&self, n: Named) -> KeyCode {
        self.settings.bindings.named(n)
    }

    /// The holo3PP renders only when its panel is up and the main view is
    /// still the cockpit — in chase the whole screen already is the rig.
    fn holo_active(&self) -> bool {
        self.settings.holo_view && !self.chase_active()
    }

    /// Where a pose's eye sits in the world, metres.
    fn eye_m(&self, pose: &ViewPose) -> DVec3 {
        self.state.ship.pos_m + self.state.ship.orient * pose.eye_ship
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

/// Ask for the adapter and device, and configure the surface for it.
async fn request_gpu(
    instance: &wgpu::Instance,
    surface: &wgpu::Surface<'static>,
    window: &Window,
    cfg: &Config,
) -> GpuParts {
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
    let size = window.inner_size();
    let mut config = surface
        .get_default_config(&adapter, size.width.max(1), size.height.max(1))
        .expect("surface unsupported by adapter");
    config.present_mode = if cfg.vsync {
        wgpu::PresentMode::AutoVsync
    } else {
        wgpu::PresentMode::AutoNoVsync
    };
    if capture_final() {
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

        let display_handle = event_loop.owned_display_handle();
        let instance = wgpu::Instance::new(
            wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(display_handle)),
        );
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");

        #[cfg(not(target_arch = "wasm32"))]
        {
            let parts = pollster::block_on(request_gpu(&instance, &surface, &window, &cfg));
            self.finish_init(window, surface, settings, cfg, parts);
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
    ) {
        let GpuParts {
            device,
            queue,
            config,
            msaa_supported,
        } = parts;
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

        self.gpu = Some(Gpu {
            window,
            device,
            queue,
            surface,
            config,
            scene,
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
        game.apply_settings(settings);
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
        if game.frozen && std::env::var("FARFALL_BENCH_CHASE").is_ok() {
            game.settings.camera_chase = true;
        }
        if game.frozen && std::env::var("FARFALL_BENCH_HOLO").is_ok() {
            game.settings.holo_view = true;
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
        if game.frozen && std::env::var("FARFALL_BENCH_LAND").is_ok() {
            game.toggle_landing();
            game.touchdown = landing::predict(&game.params, &game.state.ship, game.state.time_s);
        }
        if game.frozen && std::env::var("FARFALL_BENCH_DISEMBARK").is_ok() {
            game.disembark();
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
                    gpu.update_instruments(game, &cam, aspect, altitude_m as f32);
                    // The capture should show what the pilot
                    // sees, text included. The HUD pipeline is
                    // single-sample (it draws in the present
                    // pass), so it can only join a 1x scene.
                    let capture_text = gpu.cfg.msaa == 1;
                    if capture_text && (game.map_open() || game.mini_map_on()) {
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
                        game.menu.render(&mut gpu.text, &game.settings);
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
                        gpu.passes.jet.draw(&mut pass);
                        if !game.chase_active() {
                            gpu.passes.plasma.draw(&mut pass, &gpu.passes.thermal);
                        }
                        gpu.passes.trajectory.draw(&mut pass);
                        gpu.passes.shield.draw(&mut pass);
                        gpu.passes.ghost.draw(&mut pass);
                    }
                    {
                        // The picture, then the ship over it.
                        let mut pass = gpu.post.begin_ship_pass(&mut encoder, &gpu.scene, true);
                        if !game.chase_active() {
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
                        if capture_text && (game.map_open() || game.mini_map_on()) {
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
                                log::info!("screenshot: {}", path.display())
                            }
                            Err(e) => log::warn!("headless capture failed: {e}"),
                        }
                    }
                }
                if t > gpu.cfg.bench_seconds {
                    if let Some(el) = event_loop {
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
    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());

    // In VR the surface is the stereo pair, side by side; each eye is a
    // full draw of its own, from its own seat, into its own half.
    let eyes: u32 = if game.vr.is_some() { 2 } else { 1 };
    let (ew, eh) = (gpu.config.width / eyes, gpu.config.height);
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
        let (altitude_m, _) = game.altitude_vspeed();
        gpu.update_instruments(game, &cam, aspect, altitude_m as f32);

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
            if gpu.cfg.draws("jet") {
                gpu.passes.jet.draw(&mut pass);
            }
            if gpu.cfg.draws("plasma") && !game.chase_active() {
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
            if gpu.cfg.draws("gauge") && !game.chase_active() {
                // At infinity, so under the dash: the cabin covers what
                // falls below its sill. On the ship side, so it never blooms.
                gpu.passes.horizon.draw(&mut pass);
            }
            if gpu.cfg.draws("cockpit") && !game.chase_active() {
                gpu.passes.cabin.draw(&mut pass);
                if gpu.cfg.draws("dust") {
                    gpu.passes.dust.draw_cabin(&mut pass, &du);
                }
            }
            if gpu.cfg.draws("gauge") && !game.chase_active() {
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
            if game.map_open() || game.mini_map_on() {
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
        // captures exactly the frame that was just drawn.
        let pending = if gpu.capture_requested {
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

    if let Some(capture) = captured {
        let bgra = matches!(
            gpu.scene.format(),
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        match capture.save(&gpu.device, bgra) {
            Ok(path) => log::info!("screenshot: {}", path.display()),
            Err(e) => log::warn!("screenshot failed: {e}"),
        }
        // The readback blocks on the GPU; that frame's timing says
        // nothing about the renderer.
        gpu.perf.stats.skip_next_frame();
    }

    gpu.queue.present(frame);
    if gpu.cfg.gpu_sync {
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    }
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
                el.exit();
            }
        }
    }
    gpu.govern_scale(&game.settings, game.settings.fps_floor);
    gpu.frame_timing(
        cpu_seconds,
        wait_seconds,
        game.settings.fps_floor,
        &Readout {
            altitude_m: game.altitude_vspeed().0,
            speed_mps: game.state.ship.vel_mps.length(),
            assist: game.assist,
            show: game.settings.layout.shown(Instrument::Readout)
                || game.landing
                || game.on_ground(),
            landing: game
                .landing_text()
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
        game.menu.render(&mut gpu.text, &game.settings);
    }
    gpu.window.request_redraw();
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
                game.look.motion(delta.0 as f32, delta.1 as f32);
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
                // The CONTROLS card: any key puts it away; F1 brings it back.
                if game.card_open {
                    if pressed && !event.repeat {
                        game.close_card();
                    }
                    return;
                }
                if pressed && !event.repeat && code == KeyCode::F1 && !game.design {
                    game.open_card();
                    return;
                }
                if game.design {
                    if pressed && !event.repeat {
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
                    if pressed && !event.repeat && code == game.bind(Named::Map) {
                        game.toggle_map();
                        return;
                    }
                    if pressed && !event.repeat && code == game.bind(Named::Bay) {
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
                    if pressed && !event.repeat {
                        let panel = if game.map_open() {
                            &mut game.map_panel
                        } else if game.bay_open() {
                            &mut game.bay_panel
                        } else {
                            &mut game.menu
                        };
                        let ev = panel.key(code, &mut game.settings);
                        apply_menu_event(game, gpu, event_loop, ev);
                    }
                    return;
                }
                match code {
                    // Whatever was held is released when a panel opens: the
                    // world pauses, the keys must not carry a thrust demand
                    // across.
                    KeyCode::Escape if pressed && !event.repeat => game.toggle_menu(),
                    // Every named control below reads its binding — the
                    // KEYS page lists all of them, so what the menu shows
                    // is what the keyboard does. Edge-triggered, and
                    // `repeat` is filtered: holding a key must not strobe
                    // a toggle.
                    c if pressed && !event.repeat && c == game.bind(Named::Bay) => {
                        game.toggle_bay()
                    }
                    c if pressed && !event.repeat && c == game.bind(Named::Hold) => {
                        game.toggle_hold()
                    }
                    c if pressed && !event.repeat && c == game.bind(Named::Map) => {
                        game.toggle_map()
                    }
                    c if pressed && !event.repeat && c == game.bind(Named::Appearance) => {
                        game.cycle_appearance()
                    }
                    c if pressed
                        && !event.repeat
                        && (c == game.bind(Named::Capture) || c == KeyCode::F12) =>
                    {
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            gpu.capture_requested = true;
                        }
                    }
                    c if pressed
                        && !event.repeat
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
                    c if pressed && !event.repeat && c == game.bind(Named::Engage) => {
                        game.engage_warp()
                    }
                    c if pressed && !event.repeat && c == game.bind(Named::WarpStop) => {
                        game.warp_stop()
                    }
                    c if pressed && !event.repeat && c == game.bind(Named::Landing) => {
                        game.toggle_landing()
                    }
                    c if pressed && !event.repeat && c == game.bind(Named::Disembark) => {
                        game.disembark()
                    }
                    c if pressed && !event.repeat && c == game.bind(Named::Design) => {
                        game.toggle_design();
                        gpu.set_look_cursor(game.look.engaged() && !game.design);
                    }
                    c if pressed && !event.repeat && c == game.bind(Named::LookLock) => {
                        game.look.toggle_lock();
                        gpu.set_look_cursor(game.look.engaged());
                    }
                    c if pressed && !event.repeat && c == game.bind(Named::Trajectory) => {
                        game.settings.layout.cycle(Instrument::Trajectory, true);
                        game.settings.save();
                        log::info!(
                            "trajectory {}",
                            game.settings.layout.get(Instrument::Trajectory).name()
                        );
                    }
                    c if pressed && !event.repeat && c == game.bind(Named::Chase) => {
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
                    c if pressed && !event.repeat && c == game.bind(Named::Holo) => {
                        game.settings.holo_view = !game.settings.holo_view;
                        game.settings.save();
                        log::info!(
                            "holo3PP {}",
                            if game.settings.holo_view { "ON" } else { "OFF" }
                        );
                    }
                    c if pressed && !event.repeat && c == game.bind(Named::Weapon1) => {
                        game.arms.select(arms::Weapon::Cannon);
                        log::info!("arms: {}", game.arms.selected.name());
                    }
                    c if pressed && !event.repeat && c == game.bind(Named::Weapon2) => {
                        game.arms.select(arms::Weapon::Rail);
                        log::info!("arms: {}", game.arms.selected.name());
                    }
                    c if pressed && !event.repeat && c == game.bind(Named::NextWeapon) => {
                        game.arms.next_weapon();
                        log::info!("arms: {}", game.arms.selected.name());
                    }
                    c if pressed && !event.repeat && c == game.bind(Named::Assist) => {
                        game.assist = !game.assist;
                        log::info!("flight assist {}", if game.assist { "ON" } else { "OFF" });
                    }
                    c if pressed
                        && (c == game.bind(Named::HoloOut) || c == game.bind(Named::HoloIn)) =>
                    {
                        game.zoom_holo(if c == game.bind(Named::HoloOut) {
                            1.0
                        } else {
                            -1.0
                        });
                    }
                    _ => game.input.set(code, pressed),
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Right,
                ..
            } => {
                game.look.set_held(state == ElementState::Pressed);
                gpu.set_look_cursor(game.look.engaged());
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
                game.fire_held = false;
                game.look.set_held(false);
                gpu.set_look_cursor(game.look.engaged());
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
        assert!(!game.chase_active());
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
}
