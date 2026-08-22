//! farfall-app — native shell (SPEC §5.1).
//!
//! Owns the window, the fixed-timestep accumulator (SPEC §7.2), and the
//! sim → render translation. The sim is authoritative (SPEC §5.2): this loop
//! feeds it inputs and *reads* state; nothing here mutates the world directly.
//!
//! M1 scope: the ship is hand-flown. Keys map to sim [`Controls`] (see
//! [`input`]), the camera rides the hull looking down the nose, and rotational
//! flight assist is toggleable. Planet, HUD, and sun arrive in M1 tasks 4-6.

mod capture;
mod cockpit;
mod input;
mod landing;
mod look;
mod map;
mod menu;
mod settings;
mod warp;

use cockpit::Instrument;
use look::Look;
use menu::{Change, Menu, MenuEvent};
use settings::Settings;
use warp::Warp;
mod telemetry;

use glam::{DQuat, DVec3};
use std::sync::Arc;
use std::time::{Duration, Instant};

use capture::Capture;
use farfall_audio::Audio;
use farfall_render::{
    attitude::{
        guide_pass, gyro_pass, horizon_pass, Attitude, GuideUniforms, GyroUniforms, HorizonFade,
        HorizonUniforms,
    },
    bake::BakedMaps,
    blit::{BlitPass, PostUniforms},
    bodies::{BodiesPass, BodiesUniforms},
    gauge::{
        gauge_pass, AltitudeFade, GForceFade, GaugeFade, GaugePass, GaugeUniforms, HoloSway,
        MachAlert,
    },
    hud::HudPass,
    instrument::InstrumentPass,
    planet::{PlanetAppearance, PlanetPass, PlanetUniforms},
    starfield::StarfieldPass,
    text::TextBitmap,
    thermal::{PlasmaPass, PlasmaUniforms, ThermalInputs, ThermalPass},
    trajectory::{TrajectoryPass, TrajectoryUniforms, TrajectoryWorld, MARK_SPACING_M},
    CameraFrame, FrameUniforms, SceneTarget,
};
use farfall_sim as sim;
use input::InputState;
use telemetry::FrameStats;
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::Window,
};

const STAR_DENSITY: f64 = 1.0;
/// World-space direction to the sun. Fixed for now: a moving sun is a sim
/// concern (planet rotation, orbit) and does not belong in the renderer.
/// Chosen so the terminator crosses the visible face at spawn.
/// How often the frame-time window is summarised to the log.
const PERF_LOG_EVERY: Duration = Duration::from_secs(5);
/// A glass anchor as the turned head sees it: the glass is a sphere about
/// the pilot, so every element is re-projected, not slid (look.rs).
fn on_glass(look: &Look, cam: &CameraFrame, a: [f32; 2]) -> [f32; 2] {
    look.reproject(a, (cam.fov_y * 0.5).tan(), cam.aspect)
}

/// Where a dial sits and whether it shows: a hidden instrument keeps any
/// anchor and a visibility of zero. The anchors themselves are the slots
/// in `cockpit.rs` (or wherever the pilot dragged it); the menu assigns
/// them.
fn slot_of(
    layout: &cockpit::Layout,
    look: &Look,
    cam: &CameraFrame,
    i: Instrument,
) -> ([f32; 2], f32) {
    match layout.anchor(i) {
        Some(a) => (on_glass(look, cam, a), 1.0),
        None => ([0.0, 0.0], 0.0),
    }
}
/// How far ahead the path predictor looks, seconds. A little over one
/// orbit at the spawn altitude (~8.5 min), so a closed orbit draws closed.
const TRAJECTORY_HORIZON_S: f32 = 560.0;
/// The text block's full width on the glass, NDC, for one font pixel of
/// `px_canopy`: the bitmap is 128 font pixels across.
fn text_width_ndc(px_canopy: f32) -> f32 {
    128.0 * px_canopy
}

/// Something on the glass the gaze can pick up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dragged {
    Dial(Instrument),
    /// The settings menu's text block.
    MenuPanel,
    /// The map pane with its DRIVE panel.
    MapPanel,
}

impl Dragged {
    fn name(self) -> &'static str {
        match self {
            Dragged::Dial(i) => i.name(),
            Dragged::MenuPanel => "SETTINGS PANEL",
            Dragged::MapPanel => "MAP",
        }
    }
}

/// What the text readout shows this frame.
struct Readout {
    altitude_m: f64,
    speed_mps: f64,
    assist: bool,
    show: bool,
    /// The LANDING line, in landing mode.
    landing: Option<String>,
}

/// How close (aspect-corrected NDC) the gaze must be to a dial's anchor to
/// pick it up with the left button.
const DRAG_REACH: f32 = 0.30;
/// The text readout's top-left corner on the canopy (the settings panel
/// and the DRIVE panel have anchors of their own in the settings).
const HUD_TEXT_ANCHOR: [f32; 2] = [-0.72, 0.62];
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
///                           ground, which is where this renderer hurts)
///   FARFALL_BENCH_POS=x,y,z (benchmark only: park the ship at this world
///                           position, nose on the planet — e.g. behind the
///                           Moon, to check what hides what); with
///                           FARFALL_BENCH_VEL=x,y,z for the velocity
///                           (else at rest) and FARFALL_BENCH_LOOK=x,y,z
///                           for where the nose points (else the planet)
///   FARFALL_BENCH_MAP=1    (benchmark only: open the MAP page at once)
///   FARFALL_BENCH_HEAD=y,p (benchmark only: turn the head yaw,pitch degrees)
///   FARFALL_BENCH_LAND=1   (benchmark only: LANDING mode on)
///   FARFALL_BENCH_THRUST=m,p,y,r (benchmark only: force main thrust 0..1 and
///                           pitch/yaw/roll demands -1..1, for the plumes)
///   FARFALL_BENCH_FULL=1   (benchmark only: borderless fullscreen, the real
///                           pixel count)
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
///                           hud, blit —
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
}

impl Passes {
    fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        msaa: u32,
        baked: &BakedMaps,
        cabin_res: f32,
    ) -> Self {
        let thermal = ThermalPass::new(device);
        let plasma = PlasmaPass::new(device, format, msaa, &thermal, baked);
        Self {
            starfield: StarfieldPass::new(device, format, msaa, STAR_DENSITY, baked),
            bodies: BodiesPass::new(device, format, msaa),
            planet: PlanetPass::new(device, format, msaa, baked),
            gauge: gauge_pass(device, format, msaa),
            alt_gauge: gauge_pass(device, format, msaa),
            g_gauge: gauge_pass(device, format, msaa),
            gyro: gyro_pass(device, format, msaa),
            horizon: horizon_pass(device, format, msaa),
            guide: guide_pass(device, format, msaa),
            cabin: farfall_render::cabin::CabinPass::new(device, format, msaa, cabin_res),
            thermal,
            plasma,
            trajectory: TrajectoryPass::new(device, format, msaa),
        }
    }
}

struct Gpu {
    window: Arc<Window>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    scene: SceneTarget,
    blit: BlitPass,
    passes: Passes,
    /// Owns the baked textures the passes sample.
    baked: BakedMaps,
    hud: HudPass,
    /// The system map pane, native resolution like the text.
    map: InstrumentPass,
    text: TextBitmap,
    cfg: Config,
    perf: Perf,
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
        let stay = game.settings.gauges_stay;
        let fade = |level: f32| if stay { 1.0 } else { level };
        let jet = game.settings.gauge_jet;
        let (speed_anchor, speed_on) = slot_of(layout, look, cam, Instrument::Speed);
        self.passes.gauge.update(
            &self.queue,
            &GaugeUniforms::speed(
                game.state.ship.vel_mps.length() as f32,
                fade(game.gauge_fade.level()) * speed_on,
                cam.time_s,
                aspect,
                h,
                speed_anchor,
                sway,
                game.mach(),
                game.mach_alert.level() * speed_on,
            )
            .jet(jet),
        );
        let (alt_anchor, alt_on) = slot_of(layout, look, cam, Instrument::Altitude);
        self.passes.alt_gauge.update(
            &self.queue,
            &GaugeUniforms::altitude(
                altitude_m,
                fade(game.alt_fade.level()) * alt_on,
                cam.time_s,
                aspect,
                h,
                alt_anchor,
                sway,
            )
            .jet(jet),
        );
        let (g_anchor, g_on) = slot_of(layout, look, cam, Instrument::GForce);
        self.passes.g_gauge.update(
            &self.queue,
            &GaugeUniforms::g_force(
                game.felt_g,
                fade(game.g_fade.level()) * g_on,
                cam.time_s,
                aspect,
                h,
                g_anchor,
                sway,
            )
            .jet(jet),
        );
        let (gyro_anchor, gyro_on) = slot_of(layout, look, cam, Instrument::Gyro);
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
            ),
        );
        // The design guide: the glass ruled, every shown dial's anchor and
        // reach, the gaze.
        {
            let t = (cam.fov_y * 0.5).tan();
            let gaze = look.gaze(t, cam.aspect);
            let anchors: Vec<[f32; 2]> = Instrument::ALL
                .iter()
                .copied()
                .filter(|i| i.slotted())
                .filter_map(|i| layout.anchor(i))
                .map(|a| on_glass(look, cam, a))
                .take(4)
                .collect();
            self.passes.guide.update(
                &self.queue,
                &GuideUniforms::new(
                    aspect,
                    game.settings.guide,
                    layout.safe_edge,
                    on_glass(look, cam, gaze),
                    DRAG_REACH,
                    look.engaged(),
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
        if (self.scene.scale() - settings.scale).abs() > 1e-4 {
            self.scene.set_scale(settings.scale);
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
            self.passes = Passes::new(
                &self.device,
                self.config.format,
                settings.msaa,
                &self.baked,
                settings.cockpit_res,
            );
            self.perf.stats.skip_next_frame();
            log::info!("MSAA {}x", settings.msaa);
        }
    }

    /// Close out the frame: record its duration, refresh the live readout in
    /// the title bar, and periodically summarise the window to the log.
    fn frame_timing(&mut self, cpu_seconds: f64, wait_seconds: f64, readout: &Readout) {
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

        // 4 Hz is fast enough to feel live and slow enough to stay readable.
        if now.duration_since(self.perf.last_title) >= Duration::from_millis(250) {
            self.perf.last_title = now;
            let fps = self.perf.stats.smoothed_fps();
            let low = self.perf.stats.recent_low_1pct_fps();
            self.text.clear();
            if !show_readout {
                return;
            }
            self.text.draw(0, 0, &format!("{fps:.0} FPS"));
            self.text.draw(0, 6, &format!("1% LOW {low:.0}"));

            // CPU against total, side by side, because "the CPU feels busy" is
            // a hypothesis and this is the measurement that settles it.
            let frame_ms = if fps > 0.0 { 1000.0 / fps } else { 0.0 };
            let cpu_fps = self.perf.cpu.smoothed_fps();
            let cpu_ms = if cpu_fps > 0.0 { 1000.0 / cpu_fps } else { 0.0 };
            self.text.draw(0, 12, &format!("CPU {cpu_ms:.1}MS"));
            self.text.draw(
                0,
                18,
                &format!("REST {:.1}MS", (frame_ms - cpu_ms).max(0.0)),
            );
            let (sw, sh) = self.scene.size();
            self.text.draw(
                0,
                24,
                &format!(
                    "{}X MSAA  {:.0}%",
                    self.cfg.msaa,
                    self.scene.scale() * 100.0
                ),
            );
            self.text.draw(0, 30, &format!("{sw}X{sh}"));
            self.text.draw(
                0,
                36,
                &format!(
                    "ALT {}",
                    farfall_render::gauge::length_text(altitude_m as f32)
                ),
            );
            self.text.draw(
                0,
                42,
                &format!(
                    "VEL {}",
                    farfall_render::gauge::speed_text(speed_mps as f32)
                ),
            );
            // The flight computer's state lives on the HUD because the log is
            // invisible in fullscreen — X seemed broken when it was merely
            // silent.
            self.text
                .draw(0, 48, if assist { "FC ON" } else { "FC OFF" });
            if self.cfg.bench {
                self.text.draw(0, 54, "BENCH SIM FROZEN");
            }
            if let Some(line) = landing {
                self.text.draw(0, 60, line);
            }
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
    /// Jumps made, for the drive's crack.
    jumps: u32,
    /// Bench: thrust and RCS demands forced for a capture.
    bench_thrust: Option<[f32; 4]>,
    /// LANDING mode (G): the hoops close up and judge the touchdown.
    landing: bool,
    /// The predicted touchdown, refreshed each frame in landing mode.
    touchdown: Option<landing::Touchdown>,
    /// Mouse: last cursor position and whether the left button is down,
    /// for dragging the map round.
    cursor: Option<(f32, f32)>,
    left_down: bool,
    /// The wormhole drive's sequence.
    warp: Warp,
    /// Felt acceleration over the last sim step, g, and the meter's fade.
    felt_g: f32,
    g_fade: GForceFade,
    /// Metres of path flown, so the path's marks can stay fixed to the
    /// world. Presentation only: a wrapped f32 is fine for a phase.
    odometer_m: f64,
    /// Hoops that have passed the ship while the path was showing: the
    /// audio womps on every increment.
    hoops_passed: u32,
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
        }
        let now = Instant::now();
        Self {
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
            jumps: 0,
            bench_thrust: std::env::var("FARFALL_BENCH_THRUST").ok().and_then(|v| {
                let xs: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                (xs.len() == 4).then(|| [xs[0], xs[1], xs[2], xs[3]])
            }),
            landing: false,
            touchdown: None,
            cursor: None,
            left_down: false,
            warp: Warp::new(),
            felt_g: 0.0,
            g_fade: GForceFade::new(),
            odometer_m: 0.0,
            hoops_passed: 0,
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
    }

    /// The Sun and the Moon as the camera sees them: where the sim has them
    /// at sim time, subtracted in f64 (P3).
    fn bodies_uniforms(&self, cam: &CameraFrame, height_px: f32) -> BodiesUniforms {
        let [_, moon, sun, uranus] = self.params.bodies(self.state.time_s);
        let tags = if self.settings.layout.shown(Instrument::BodyTags) {
            1.0
        } else {
            0.0
        };
        BodiesUniforms::new(
            cam,
            (
                (moon.centre - self.state.ship.pos_m).as_vec3(),
                moon.radius_m as f32,
            ),
            (
                (sun.centre - self.state.ship.pos_m).as_vec3(),
                sun.radius_m as f32,
            ),
            (
                (uranus.centre - self.state.ship.pos_m).as_vec3(),
                uranus.radius_m as f32,
            ),
            tags,
            height_px,
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
        let look = map::MapLook {
            view: self.map_view,
            rings: self.settings.map_rings,
            grid: self.settings.map_grid,
            visibility: if self.map_open() { 1.0 } else { 0.0 },
            aspect,
            time_s,
            centre: self.settings.map_anchor,
        };
        map::MapUniforms::new(&world, &look)
    }

    /// The path's world-fixed marks, from the odometer and the settings.
    fn marks(&self) -> farfall_render::trajectory::Marks {
        let spacing = self.mark_spacing_m();
        farfall_render::trajectory::Marks {
            odometer_m: (self.odometer_m % 1.0e6) as f32,
            hoops: self.settings.layout.shown(Instrument::Hoops),
            // Landing hoops are big: a gate to fly through, not a bead.
            hoop_scale: self.settings.hoop_size * if self.landing { 2.5 } else { 1.0 },
            spacing_m: spacing,
            landing: if self.landing {
                Some(self.touchdown.map_or(0.0, |t| t.danger()))
            } else {
                None
            },
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

    /// Any panel is up: the world waits and the keys go to it.
    fn panel_open(&self) -> bool {
        self.menu.open || self.map_panel.open
    }

    /// M: the map up or down. One panel at a time.
    fn toggle_map(&mut self) {
        self.map_panel.toggle();
        if self.map_panel.open {
            self.menu.open = false;
        }
        self.input.release_all();
    }

    /// Esc: the settings menu up or down. One panel at a time.
    fn toggle_menu(&mut self) {
        self.menu.toggle();
        if self.menu.open {
            self.map_panel.open = false;
        }
        self.input.release_all();
    }

    /// The map pane's geometry this frame.
    fn map_pane(&self, aspect: f32) -> [f32; 3] {
        map::pane_rect(aspect, self.settings.map_anchor)
    }

    /// Where the DRIVE panel's text block starts: hung off the map pane's
    /// top-left, `text_w` (NDC) to its left.
    fn drive_text_anchor(&self, aspect: f32, text_w: f32) -> [f32; 2] {
        let [cx, cy, hw] = self.map_pane(aspect);
        [cx - hw - text_w - 0.03, cy + hw * aspect]
    }

    /// Where the text block's top-left sits this frame: the DRIVE panel
    /// beside the map, the settings panel at its anchor, else the readout.
    fn text_anchor(&self, aspect: f32, text_w: f32) -> [f32; 2] {
        if self.map_open() {
            self.drive_text_anchor(aspect, text_w)
        } else if self.menu.open {
            self.settings.menu_anchor
        } else {
            self.settings.layout.inset(HUD_TEXT_ANCHOR)
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
        let t = (cam.fov_y * 0.5).tan();
        let sockets: Vec<glam::Vec3> = Instrument::ALL
            .iter()
            .copied()
            .filter(|i| i.slotted())
            .filter_map(|i| self.settings.layout.anchor(i))
            .map(|a| anchor_direction(a, t, cam.aspect))
            .take(if std::env::var("FARFALL_NO_SOCKETS").is_ok() {
                0
            } else {
                4
            })
            .collect();
        let look = CabinLook {
            glow: self.settings.cockpit_glow,
            metal: self.settings.cockpit_hull,
            on: self.settings.cockpit_frame,
            jet: self.settings.gauge_jet,
            thrust: self.bench_thrust.unwrap_or_else(|| {
                let c = self.input.controls(self.assist);
                [
                    self.effort,
                    c.torque_body.x as f32,
                    c.torque_body.y as f32,
                    c.torque_body.z as f32,
                ]
            }),
        };
        let cu = CabinUniforms::new(cam, self.look.rotation(), sun_ship, look, &sockets);
        let bu = cu.blit(look);
        (cu, bu)
    }

    /// G: landing mode on or off.
    fn toggle_landing(&mut self) {
        self.landing = !self.landing;
        if !self.landing {
            self.touchdown = None;
        }
        log::info!("landing mode {}", if self.landing { "on" } else { "off" });
    }

    /// The landing readout line, if there is one.
    fn landing_text(&self) -> Option<String> {
        if !self.landing {
            return None;
        }
        let (_, vspeed) = self.altitude_vspeed();
        Some(match self.touchdown {
            Some(t) => format!(
                "LAND {} IN {:.0}S  DOWN {:.0}  ALONG {:.0} M/S  VS {:+.0}",
                t.verdict(),
                t.in_s,
                t.into_mps,
                t.along_mps,
                vspeed
            ),
            None => format!(
                "LAND  NO TOUCHDOWN IN {:.0}S  VS {:+.0}",
                landing::HORIZON_S,
                vspeed
            ),
        })
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
        self.look.update(frame_dt.min(0.25) as f32);
        if self.warp.update(frame_dt.min(0.25) as f32) {
            self.jump();
        }

        // A pilot reading a panel is not flying: the world waits.
        if self.panel_open() {
            self.accumulator = 0.0;
            return;
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
        let controls = if self.warp.active() {
            sim::Controls {
                assist: self.assist,
                ..Default::default()
            }
        } else {
            self.input.controls(self.assist)
        };
        while self.accumulator >= sim::DT {
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
            let before = self.state.ship;
            let before_t = self.state.time_s;
            self.state = sim::step(&self.params, &self.state, controls);
            self.felt_g =
                (sim::felt_acceleration(&self.params, before_t, &before, &self.state.ship).length()
                    / 9.81) as f32;
            self.accumulator -= sim::DT;
        }

        if self.landing {
            self.touchdown = landing::predict(&self.params, &self.state.ship, self.state.time_s);
        }

        // Camera response to the ship's own physics. Exponential smoothing,
        // framerate-independent: at 30 fps and at 240 fps the view opens up over
        // the same wall-clock time, so the ship's weight is a property of the
        // ship and not of the machine.
        let target = self.input.thrust_effort(self.params.ship.boost_multiplier) as f32;
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
    fn planet_uniforms(&self, cam: &CameraFrame) -> PlanetUniforms {
        let centre_rel = (DVec3::ZERO - self.state.ship.pos_m).as_vec3();
        let [_, moon, sun, _] = self.params.bodies(self.state.time_s);
        let rel = |b: &sim::Body| {
            (
                (b.centre - self.state.ship.pos_m).as_vec3(),
                b.radius_m as f32,
            )
        };
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
    fn trajectory_world(&self) -> TrajectoryWorld {
        let planet = &self.params.planet;
        let ship = &self.state.ship;
        TrajectoryWorld {
            centre_rel: (DVec3::ZERO - ship.pos_m).as_vec3(),
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
            effort: self.input.thrust_effort(self.params.ship.boost_multiplier) as f32,
            wind_q: ((q / q_ref) as f32).clamp(0.0, 1.0),
            vacuum: 1.0 - ((rho_ratio * 12.0) as f32).clamp(0.0, 1.0),
            brake: if controls.brake { 1.0 } else { 0.0 },
            // Attitude thrusters: the largest torque demand. Rolling is
            // flying, and a silent manoeuvre reads as a broken game.
            rcs: controls.torque_body.abs().max_element() as f32,
            entry: entry as f32,
            supersonic: if self.is_supersonic() { 1.0 } else { 0.0 },
            hoops: self.hoops_passed as f32,
            warp: self.warp.look().charge,
            jumps: self.jumps as f32,
            master: 0.8,
        }
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
    fn begin_drag(&mut self, cam: &CameraFrame, text_w: f32) -> bool {
        if !self.look.engaged() {
            return false;
        }
        let t = (cam.fov_y * 0.5).tan();
        let gaze = self.look.gaze(t, cam.aspect);
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
        if self.menu.open {
            let a = self.settings.menu_anchor;
            let on_text = gaze[0] >= a[0] - 0.02
                && gaze[0] <= a[0] + text_w + 0.02
                && gaze[1] <= a[1] + 0.02
                && gaze[1] >= a[1] - 0.5;
            if on_text {
                self.drag = Some((Dragged::MenuPanel, [a[0] - gaze[0], a[1] - gaze[1]]));
                log::info!("drag: picked up the settings panel");
                return true;
            }
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
        if !self.look.engaged() {
            self.end_drag();
            return;
        }
        let t = (cam.fov_y * 0.5).tan();
        let gaze = self.look.gaze(t, cam.aspect);
        let at = [gaze[0] + off[0], gaze[1] + off[1]];
        let clamp = |a: [f32; 2]| [a[0].clamp(-0.95, 0.95), a[1].clamp(-0.95, 0.95)];
        match i {
            Dragged::Dial(i) => {
                let at = self.settings.layout.uninset(at);
                self.settings.layout.set_free(i, at);
            }
            Dragged::MenuPanel => self.settings.menu_anchor = clamp(at),
            Dragged::MapPanel => self.settings.map_anchor = clamp(at),
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
        // The camera is the ship's orientation — both use the same
        // right-handed frame with the nose at -Z, so no fix-up rotation is
        // needed — times the pilot's head. The head is a view, not a
        // control: nothing downstream of the sim sees it.
        let orient = self.state.ship.orient.as_quat() * self.look.rotation();
        CameraFrame {
            orient,
            fov_y: ((self.settings.fov + FOV_THRUST_GAIN * self.effort)
                * self.warp.look().fov_scale)
                .to_radians()
                .min(2.9),
            aspect,
            time_s: self.started.elapsed().as_secs_f32(),
            exposure: 1.6,
        }
    }
}

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
        if !cfg.windowed {
            // Borderless fullscreen on the current monitor: no mode switch, so
            // alt-tab stays instant and the resolution is the desktop's.
            attrs = attrs.with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
        }
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));

        let display_handle = event_loop.owned_display_handle();
        let instance = wgpu::Instance::new(
            wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(display_handle)),
        );
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");

        let (device, queue, config, msaa_supported) = pollster::block_on(async {
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: Some(&surface),
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
            (device, queue, config, msaa_supported)
        });
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
        // Bake the static world fields before the first frame. Everything the
        // planet pass reads per pixel is generated here, by shader, once.
        let baked = BakedMaps::bake(&device, &queue);
        let passes = Passes::new(
            &device,
            config.format,
            cfg.msaa,
            &baked,
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
            blit,
            passes,
            baked,
            hud,
            map,
            text: TextBitmap::new(),
            cfg,
            perf: Perf::new(),
            capture_requested: false,
            bench_captured: false,
            bench_spin_taken: 0,
        });
        let mut game = Game::new();
        let mut settings = settings;
        settings.msaa = msaa_in_use;
        game.apply_settings(settings);
        game.menu.set_msaa_supported(&msaa_supported);
        if game.frozen && std::env::var("FARFALL_BENCH_MAP").is_ok() {
            game.toggle_map();
        }
        if game.frozen && std::env::var("FARFALL_BENCH_LAND").is_ok() {
            game.toggle_landing();
            game.touchdown = landing::predict(&game.params, &game.state.ship, game.state.time_s);
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
        self.game = Some(game);
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_none() {
            self.init_gpu(event_loop);
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
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
        let (Some(gpu), Some(game)) = (self.gpu.as_mut(), self.game.as_mut()) else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => {
                game.log_exit("window closed");
                event_loop.exit();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                let pressed = event.state == ElementState::Pressed;
                if game.panel_open() {
                    // M closes the map from anywhere in it.
                    if pressed && !event.repeat && code == KeyCode::KeyM {
                        game.toggle_map();
                        return;
                    }
                    // Map zoom from the keyboard, for a mouse with no wheel.
                    if pressed && game.map_open() {
                        match code {
                            KeyCode::Equal | KeyCode::NumpadAdd => game.map_view.zoom_by(1.0),
                            KeyCode::Minus | KeyCode::NumpadSubtract => game.map_view.zoom_by(-1.0),
                            _ => {}
                        }
                    }
                    if pressed && !event.repeat {
                        let panel = if game.map_open() {
                            &mut game.map_panel
                        } else {
                            &mut game.menu
                        };
                        match panel.key(code, &mut game.settings) {
                            MenuEvent::Changed(change) => {
                                game.settings.save();
                                match change {
                                    Change::Bindings => {
                                        game.input.set_bindings(game.settings.bindings);
                                        game.look.sensitivity = game.settings.look_sensitivity;
                                    }
                                    Change::Graphics => gpu.apply_graphics(&game.settings),
                                    Change::Layout => {}
                                }
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
                    return;
                }
                match code {
                    // Whatever was held is released when a panel opens: the
                    // world pauses, the keys must not carry a thrust demand
                    // across.
                    KeyCode::Escape if pressed && !event.repeat => game.toggle_menu(),
                    KeyCode::KeyM if pressed && !event.repeat => game.toggle_map(),
                    // Edge-triggered, and `repeat` is filtered: holding the key
                    // must not strobe the toggle.
                    KeyCode::KeyC if pressed && !event.repeat => game.cycle_appearance(),
                    KeyCode::F12 | KeyCode::KeyP if pressed && !event.repeat => {
                        gpu.capture_requested = true;
                    }
                    KeyCode::BracketLeft | KeyCode::BracketRight if pressed && !event.repeat => {
                        let step = if code == KeyCode::BracketRight {
                            0.1
                        } else {
                            -0.1
                        };
                        let next = gpu.scene.scale() + step;
                        gpu.scene.set_scale(next);
                        log::info!("render scale {:.0}%", gpu.scene.scale() * 100.0);
                    }
                    KeyCode::KeyJ if pressed && !event.repeat => game.engage_warp(),
                    KeyCode::KeyG if pressed && !event.repeat => game.toggle_landing(),
                    KeyCode::KeyL if pressed && !event.repeat => {
                        game.look.toggle_lock();
                        gpu.set_look_cursor(game.look.engaged());
                    }
                    KeyCode::KeyT if pressed && !event.repeat => {
                        game.settings.layout.cycle(Instrument::Trajectory, true);
                        game.settings.save();
                        log::info!(
                            "trajectory {}",
                            game.settings.layout.get(Instrument::Trajectory).name()
                        );
                    }
                    KeyCode::KeyX if pressed && !event.repeat => {
                        game.assist = !game.assist;
                        log::info!("flight assist {}", if game.assist { "ON" } else { "OFF" });
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
                if game.left_down {
                    let aspect = gpu.config.width as f32 / gpu.config.height as f32;
                    let cam = game.camera(aspect);
                    let hud_scale = (gpu.config.height as f32 / 260.0).clamp(2.0, 8.0).floor();
                    game.begin_drag(
                        &cam,
                        text_width_ndc(hud_scale * 2.0 / gpu.config.height as f32),
                    );
                } else {
                    game.end_drag();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let now = (position.x as f32, position.y as f32);
                if let Some(last) = game.cursor {
                    // Turning the map with the mouse, unless the gaze has
                    // hold of the pane itself.
                    if game.left_down && game.map_open() && game.drag.is_none() {
                        game.map_view.drag(now.0 - last.0, now.1 - last.1);
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
            }
            // A key held while the window loses focus never sees its release
            // event; without this the ship keeps thrusting unattended.
            WindowEvent::Focused(false) => {
                game.input.release_all();
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
                if let Some(audio) = &self.audio {
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
                    wgpu::CurrentSurfaceTexture::Timeout
                    | wgpu::CurrentSurfaceTexture::Occluded => {
                        // Benchmarks must neither hang nor lie when occluded:
                        // the window being buried is no reason to skip the
                        // capture, because the scene target is offscreen and
                        // needs no swapchain. Render it headless and save.
                        // (This is the seed of the golden-image harness: a
                        // frame produced with no visible window at all.)
                        if gpu.cfg.bench {
                            let t = game.started.elapsed().as_secs_f64();
                            if gpu.bench_capture_due(t) {
                                if gpu.scene.ensure(
                                    &gpu.device,
                                    gpu.config.width,
                                    gpu.config.height,
                                ) {
                                    if let Some(view) = gpu.scene.colour_view() {
                                        gpu.blit.rebind(&gpu.device, view);
                                    }
                                }
                                let aspect = gpu.config.width as f32 / gpu.config.height as f32;
                                let cam = game.camera(aspect);
                                gpu.passes.starfield.update(
                                    &gpu.queue,
                                    &FrameUniforms::from_camera(&cam).with_occluder(
                                        (DVec3::ZERO - game.state.ship.pos_m).as_vec3(),
                                        game.params.planet.radius_m as f32,
                                    ),
                                );
                                gpu.passes
                                    .planet
                                    .update(&gpu.queue, &game.planet_uniforms(&cam));
                                gpu.passes.bodies.update(
                                    &gpu.queue,
                                    &game.bodies_uniforms(&cam, gpu.scene.size().1 as f32),
                                );
                                let (altitude_m, _) = game.altitude_vspeed();
                                gpu.update_instruments(game, &cam, aspect, altitude_m as f32);
                                // The capture should show what the pilot
                                // sees, text included. The HUD pipeline is
                                // single-sample (it draws in the present
                                // pass), so it can only join a 1x scene.
                                let capture_text = gpu.cfg.msaa == 1;
                                if capture_text && game.menu.map_open() {
                                    gpu.map
                                        .update(&gpu.queue, &game.map_uniforms(aspect, cam.time_s));
                                }
                                if capture_text && game.map_open() {
                                    game.map_panel.render(&mut gpu.text, &game.settings);
                                } else if capture_text && game.menu.open {
                                    game.menu.render(&mut gpu.text, &game.settings);
                                } else if capture_text {
                                    gpu.text.clear();
                                    gpu.text.draw(0, 0, "HEADLESS CAPTURE");
                                    gpu.text.draw(
                                        0,
                                        6,
                                        &format!(
                                            "ALT {}",
                                            farfall_render::gauge::length_text(altitude_m as f32)
                                        ),
                                    );
                                    gpu.text.draw(
                                        0,
                                        12,
                                        &format!("VEL {:.0}M/S", game.state.ship.vel_mps.length()),
                                    );
                                    let (_, sh) = gpu.scene.size();
                                    let hud_scale = (sh as f32 / 260.0).clamp(2.0, 8.0).floor();
                                    let px_canopy = hud_scale * 2.0 / sh as f32;
                                    gpu.hud.update(
                                        &gpu.queue,
                                        &gpu.text,
                                        on_glass(
                                            &game.look,
                                            &cam,
                                            game.text_anchor(aspect, text_width_ndc(px_canopy)),
                                        ),
                                        px_canopy,
                                        aspect,
                                        sh as f32,
                                        game.holo_sway.sway(),
                                    );
                                }
                                let mut encoder = gpu.device.create_command_encoder(
                                    &wgpu::CommandEncoderDescriptor {
                                        label: Some("headless"),
                                    },
                                );
                                let thermal_in = game.thermal_inputs(game.frame_dt);
                                gpu.passes.plasma.update(
                                    &gpu.queue,
                                    &PlasmaUniforms::new(
                                        &cam,
                                        thermal_in.vel_ship_mps,
                                        game.look.rotation(),
                                    ),
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
                                        &game.trajectory_world(),
                                        TRAJECTORY_HORIZON_S,
                                        game.trajectory_vis,
                                        gpu.scene.size().1 as f32,
                                        game.marks(),
                                    ),
                                );
                                {
                                    let mut pass =
                                        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                            label: Some("scene headless"),
                                            color_attachments: &[Some(
                                                gpu.scene.colour_attachment(),
                                            )],
                                            depth_stencil_attachment: None,
                                            timestamp_writes: None,
                                            occlusion_query_set: None,
                                            multiview_mask: None,
                                        });
                                    gpu.passes.starfield.draw(&mut pass);
                                    gpu.passes.bodies.draw(&mut pass);
                                    gpu.passes.planet.draw(&mut pass);
                                    gpu.passes.plasma.draw(&mut pass, &gpu.passes.thermal);
                                    gpu.passes.trajectory.draw(&mut pass);
                                    gpu.passes.cabin.draw(&mut pass);
                                    gpu.passes.horizon.draw(&mut pass);
                                    gpu.passes.gauge.draw(&mut pass);
                                    gpu.passes.alt_gauge.draw(&mut pass);
                                    gpu.passes.g_gauge.draw(&mut pass);
                                    gpu.passes.gyro.draw(&mut pass);
                                    gpu.passes.guide.draw(&mut pass);
                                    gpu.passes.guide.draw(&mut pass);
                                    if capture_text && game.map_open() {
                                        gpu.map.draw(&mut pass);
                                    }
                                    if capture_text {
                                        gpu.hud.draw(&mut pass);
                                    }
                                }
                                let capture = gpu.scene.colour_texture().map(|tex| {
                                    let path = std::env::temp_dir()
                                        .join(format!("farfall-{:.0}.png", t * 1000.0));
                                    Capture::record(&gpu.device, &mut encoder, tex, path)
                                });
                                gpu.queue.submit([encoder.finish()]);
                                if let Some(capture) = capture {
                                    let bgra = matches!(
                                        gpu.scene.format(),
                                        wgpu::TextureFormat::Bgra8Unorm
                                            | wgpu::TextureFormat::Bgra8UnormSrgb
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
                                event_loop.exit();
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
                let encode_start = Instant::now();
                let view = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());

                if gpu
                    .scene
                    .ensure(&gpu.device, gpu.config.width, gpu.config.height)
                {
                    // The scene textures were recreated; a bind group still
                    // pointing at the old view would sample a destroyed
                    // resource.
                    let view = gpu.scene.colour_view().expect("scene target");
                    gpu.blit.rebind(&gpu.device, view);
                    gpu.perf.stats.skip_next_frame();
                }

                let aspect = gpu.config.width as f32 / gpu.config.height as f32;
                let cam = game.camera(aspect);
                game.update_drag(&cam);
                gpu.passes.starfield.update(
                    &gpu.queue,
                    &FrameUniforms::from_camera(&cam).with_occluder(
                        (DVec3::ZERO - game.state.ship.pos_m).as_vec3(),
                        game.params.planet.radius_m as f32,
                    ),
                );
                gpu.passes
                    .planet
                    .update(&gpu.queue, &game.planet_uniforms(&cam));
                gpu.passes.bodies.update(
                    &gpu.queue,
                    &game.bodies_uniforms(&cam, gpu.scene.size().1 as f32),
                );
                let thermal_in = game.thermal_inputs(game.frame_dt);
                gpu.passes.plasma.update(
                    &gpu.queue,
                    &PlasmaUniforms::new(&cam, thermal_in.vel_ship_mps, game.look.rotation()),
                );
                gpu.passes.trajectory.update(
                    &gpu.queue,
                    &TrajectoryUniforms::new(
                        &cam,
                        &game.trajectory_world(),
                        TRAJECTORY_HORIZON_S,
                        game.trajectory_vis,
                        gpu.scene.size().1 as f32,
                        game.marks(),
                    ),
                );
                let (altitude_m, _) = game.altitude_vspeed();
                gpu.update_instruments(game, &cam, aspect, altitude_m as f32);

                // Scale the readout with the surface so it keeps the same
                // apparent size on a retina fullscreen and a small window;
                // the size is chosen in pixels and expressed in canopy units.
                let hud_scale = (gpu.config.height as f32 / 260.0).clamp(2.0, 8.0).floor();
                let px_canopy = hud_scale * 2.0 / gpu.config.height as f32;
                {
                    let l = game.warp.look();
                    gpu.blit.update(
                        &gpu.queue,
                        &PostUniforms::new(
                            l.fisheye,
                            l.invert,
                            l.particles,
                            l.charge,
                            aspect,
                            cam.time_s,
                        ),
                    );
                    gpu.map
                        .update(&gpu.queue, &game.map_uniforms(aspect, cam.time_s));
                }
                gpu.hud.update(
                    &gpu.queue,
                    &gpu.text,
                    on_glass(
                        &game.look,
                        &cam,
                        game.text_anchor(aspect, text_width_ndc(px_canopy)),
                    ),
                    px_canopy,
                    aspect,
                    gpu.config.height as f32,
                    game.holo_sway.sway(),
                );

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
                    gpu.passes.cabin.update(&gpu.queue, &mut encoder, &cu, &bu);
                }
                {
                    // Pass 1: the expensive world, at whatever scale is set.
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("scene"),
                        color_attachments: &[Some(gpu.scene.colour_attachment())],
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
                    if gpu.cfg.draws("plasma") {
                        gpu.passes.plasma.draw(&mut pass, &gpu.passes.thermal);
                    }
                    if gpu.cfg.draws("trajectory") {
                        gpu.passes.trajectory.draw(&mut pass);
                    }
                    if gpu.cfg.draws("cockpit") {
                        gpu.passes.cabin.draw(&mut pass);
                    }
                    if gpu.cfg.draws("gauge") {
                        gpu.passes.horizon.draw(&mut pass);
                        gpu.passes.gauge.draw(&mut pass);
                        gpu.passes.alt_gauge.draw(&mut pass);
                        gpu.passes.g_gauge.draw(&mut pass);
                        gpu.passes.gyro.draw(&mut pass);
                        gpu.passes.guide.draw(&mut pass);
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
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    if gpu.cfg.draws("blit") {
                        gpu.blit.draw(&mut pass);
                    }
                    if game.map_open() {
                        gpu.map.draw(&mut pass);
                    }
                    if gpu.cfg.draws("hud") {
                        gpu.hud.draw(&mut pass);
                    }
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
                let cpu_seconds = sim_seconds + encode_start.elapsed().as_secs_f64();

                if let Some(capture) = pending {
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
                        event_loop.exit();
                    }
                }
                gpu.frame_timing(
                    cpu_seconds,
                    wait_seconds,
                    &Readout {
                        altitude_m: game.altitude_vspeed().0,
                        speed_mps: game.state.ship.vel_mps.length(),
                        assist: game.assist,
                        show: game.settings.layout.shown(Instrument::Readout) || game.landing,
                        landing: game.landing_text(),
                    },
                );
                if game.map_open() {
                    gpu.text.clear();
                    game.map_panel.render(&mut gpu.text, &game.settings);
                } else if game.menu.open {
                    gpu.text.clear();
                    game.menu.render(&mut gpu.text, &game.settings);
                }
                gpu.window.request_redraw();
            }
            _ => {}
        }
    }
}

fn main() {
    env_logger::init();
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.run_app(&mut App::default()).expect("run");
}

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
