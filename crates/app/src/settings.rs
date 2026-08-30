//! The settings file: graphics, controls, cockpit layout.
//!
//! Plain `key = value` lines at `~/.farfall/settings.cfg`, written whole on
//! every change from the menu and read once at start. No format crate: a
//! file a pilot can fix with any editor, and nothing to get wrong in it
//! that the defaults can't cover — unknown keys are ignored, bad values
//! fall back, missing lines mean default.

use crate::bay::{Mount, STOCK};
use crate::cockpit::{Instrument, Layout, Slot};
use crate::input::{key_from_name, key_name, Action, Bindings, Named};
use crate::warp::{Destination, Plan};
use std::path::PathBuf;

/// "x,y" → a pair of finite numbers.
fn parse_pair(v: &str) -> Option<[f32; 2]> {
    let (a, b) = v.split_once(',')?;
    let x = a.trim().parse::<f32>().ok()?;
    let y = b.trim().parse::<f32>().ok()?;
    (x.is_finite() && y.is_finite()).then_some([x, y])
}

/// Where the settings menu's text starts and the map pane is centred,
/// until the pilot drags them.
pub const MENU_ANCHOR_DEFAULT: [f32; 2] = [-0.72, 0.62];
pub const MAP_ANCHOR_DEFAULT: [f32; 2] = [0.42, 0.12];
/// The SHIP bay's panel: right of the hologram, up.
pub const BAY_ANCHOR_DEFAULT: [f32; 2] = [0.95, 0.90];
pub const BAY_HUE_DEFAULT: f32 = 0.52;
pub const POINTER_SIZE_DEFAULT: f32 = 0.045;
pub const BAY_SCANLINES_DEFAULT: f32 = 120.0;
pub const BAY_SIZE_DEFAULT: f32 = 0.28;
pub const BAY_SIZE_MIN: f32 = 0.14;
pub const BAY_SIZE_MAX: f32 = 0.45;
pub const BAY_SCANLINES_MAX: f32 = 400.0;
pub const READOUT_ANCHOR_DEFAULT: [f32; 2] = [-0.72, 0.62];

fn clamp_anchor(a: [f32; 2]) -> [f32; 2] {
    [a[0].clamp(-0.95, 0.95), a[1].clamp(-0.95, 0.95)]
}

/// One dial's own settings, over the cockpit-wide ones.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DialTweak {
    /// Size, as a multiple of the stock dial.
    pub size: f32,
    /// Its own style, or the cockpit's.
    pub style: Option<GaugeStyle>,
    /// Stay lit / fade by relevance, or the cockpit's.
    pub stay: Option<bool>,
    /// Leaned toward the pilot about its own horizontal axis, degrees
    /// (−60..60): angles the face to read from where the pilot sits.
    pub tilt_deg: f32,
}

pub const TILT_MIN: f32 = -60.0;
pub const TILT_MAX: f32 = 60.0;

impl DialTweak {
    pub const DEFAULT: DialTweak = DialTweak {
        size: 1.0,
        style: None,
        stay: None,
        tilt_deg: 0.0,
    };
}

/// The next style a dial may take: the cockpit's (auto), a hologram, a
/// JET sphere or a flush DIAL. JET is offered to every instrument for
/// now; the gyro is the one that is truly a ball.
pub fn next_dial_style(
    cur: Option<GaugeStyle>,
    _dial: Instrument,
    forward: bool,
) -> Option<GaugeStyle> {
    let ring: &[Option<GaugeStyle>] = &[
        None,
        Some(GaugeStyle::Tron),
        Some(GaugeStyle::Jet),
        Some(GaugeStyle::Dial),
    ];
    let i = ring.iter().position(|&s| s == cur).unwrap_or(0);
    let n = ring.len();
    ring[if forward {
        (i + 1) % n
    } else {
        (i + n - 1) % n
    }]
}

/// The style a dial actually takes for a chosen one: every style is
/// open to every instrument for now.
pub fn style_for(style: GaugeStyle, _dial: Instrument) -> GaugeStyle {
    style
}

pub const DIAL_SIZE_MIN: f32 = 0.5;
pub const DIAL_SIZE_MAX: f32 = 2.5;

/// How the dials are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GaugeStyle {
    /// Holograms on the glass over lit sockets with beams.
    Tron,
    /// Spherical bowls hollowed into the dash, the hologram in each.
    Jet,
    /// Real instruments set flush into the dash, faces in its plane.
    Dial,
}

impl GaugeStyle {
    pub const ALL: [GaugeStyle; 3] = [GaugeStyle::Tron, GaugeStyle::Jet, GaugeStyle::Dial];

    pub fn key(self) -> &'static str {
        match self {
            GaugeStyle::Tron => "tron",
            GaugeStyle::Jet => "jet",
            GaugeStyle::Dial => "dial",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            GaugeStyle::Tron => "TRON",
            GaugeStyle::Jet => "JET",
            GaugeStyle::Dial => "DIAL",
        }
    }

    pub fn from_key(k: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|s| s.key() == k)
    }

    pub fn next(self, forward: bool) -> Self {
        let i = Self::ALL.iter().position(|&s| s == self).unwrap_or(0);
        let n = Self::ALL.len();
        Self::ALL[if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        }]
    }

    /// The cabin's number for it.
    pub fn index(self) -> u32 {
        match self {
            GaugeStyle::Tron => 0,
            GaugeStyle::Jet => 1,
            GaugeStyle::Dial => 2,
        }
    }
}

/// Field of view limits, degrees.
pub const FOV_MIN: f32 = 50.0;
pub const FOV_MAX: f32 = 110.0;

/// The cabin's render sizes on offer, as fractions of the scene.
pub const COCKPIT_RES_CHOICES: [f32; 3] = [0.5, 0.75, 1.0];

/// Frame-rate floors on offer (0: none). The cabin's moving detail gives
/// way while turning the head would cost more than the floor allows.
pub const HOLO_ANCHOR_DEFAULT: [f32; 2] = [0.55, -0.55];
pub const HOLO_SIZE_MIN: f32 = 0.16;
pub const HOLO_SIZE_MAX: f32 = 0.50;
pub const FPS_FLOOR_CHOICES: [f32; 5] = [0.0, 30.0, 60.0, 90.0, 120.0];

/// The landing hoops' spacings on offer, metres.
pub const LANDING_SPACINGS: [f32; 4] = [100.0, 250.0, 500.0, 1000.0];

/// Hoop size range, as a multiple of the stock diameter.
pub const HOOP_SIZE_MIN: f32 = 0.25;
pub const HOOP_SIZE_MAX: f32 = 4.0;
/// The most shards a break may throw (the debris pass holds 64).
pub const ARMS_SHARDS_MAX: u32 = 48;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub msaa: u32,
    pub scale: f32,
    pub vsync: bool,
    pub bindings: Bindings,
    pub layout: Layout,
    /// Freelook: radians per mouse count, relative to the default.
    pub look_sensitivity: f32,
    /// Hoop diameter, as a multiple of the stock size.
    pub hoop_size: f32,
    /// Where the settings menu's text block sits (top-left, canopy NDC).
    pub menu_anchor: [f32; 2],
    /// Where the map pane's centre sits (canopy NDC); the DRIVE panel
    /// hangs off its left edge.
    pub map_anchor: [f32; 2],
    /// Where the text readout's block sits (top-left, canopy NDC).
    pub readout_anchor: [f32; 2],
    /// The wireframe cabin: drawn at all, how bright its lines, how opaque
    /// its hull.
    pub cockpit_frame: bool,
    pub cockpit_glow: f32,
    pub cockpit_hull: f32,
    /// The cabin is drawn at this fraction of the scene's size.
    pub cockpit_res: f32,
    /// The least frame rate the pilot will have, or 0 for no floor.
    pub fps_floor: f32,
    /// The daytime sky's strength low down, 1 = stock.
    pub sky: f32,
    /// The lens flare's strength, 1 = stock, 0 none.
    pub flare: f32,
    /// Base field of view, degrees (vertical).
    pub fov: f32,
    /// Gauge style: TRON holograms on the glass; JET bowls hollowed into
    /// the dash; DIAL real instruments set flush into the dash.
    pub gauge_style: GaugeStyle,
    /// Gauges stay lit (true) or fade by relevance (false).
    pub gauges_stay: bool,
    /// The design guide overlay.
    pub guide: bool,
    /// The hull's own voices: creak and crackle under speed, strikes.
    pub hull_sound: bool,
    /// The force field's glow on a strike, 0 (off) .. 2.
    pub shield: f32,
    /// Each dial's own settings, by [`Instrument`] index.
    pub dials: [DialTweak; Instrument::ALL.len()],
    /// Spacing of the landing hoops, metres.
    pub landing_spacing_m: f32,
    /// Rings drawn around each body on the map, 0..=6.
    pub map_rings: u32,
    /// The map's reference grid.
    pub map_grid: bool,
    /// The chase camera: the whole view from outside the ship (the dev
    /// third person the holo3PP is measured against).
    pub camera_chase: bool,
    /// The holo3PP: a live volumetric hologram of the ship and its
    /// neighbourhood, over an emitter in the dash.
    pub holo_view: bool,
    /// The hologram's size (its radius is this times half a metre).
    pub holo_size: f32,
    /// The hologram's emitter: a glass anchor (canopy NDC) whose direction
    /// meets the dash at the socket.
    pub holo_anchor: [f32; 2],
    /// The wormhole drive's destination and safe distance.
    pub plan: Plan,
    /// The reactor's share for the arms, 0..1, and their light's strength
    /// (tracers, flashes, bursts), 0 (off) .. 2.
    pub arms_power: f32,
    pub arms_glow: f32,
    /// Shards a broken rock throws (a hit chips a sixth), 0..48, and how
    /// long they last, seconds.
    pub arms_shards: u32,
    pub arms_shard_life: f32,
    /// The craters a hit leaves: their size (0 none .. 2), and how long
    /// one takes to cool, seconds.
    pub arms_scar_size: f32,
    pub arms_scar_cool: f32,
    /// ORE YIELD: what the guns bring in off the rocks, 0 (off) .. 2.
    pub arms_ore: f32,
    /// MIMICS: the share of rocks that are ships in a shroud, 0..0.5, and
    /// HOSTILITY: the share of those that shoot rather than hail, 0..1.
    pub mimics_chance: f32,
    pub mimics_hostility: f32,
    /// HOLD GAIN: how hard the lock holds, 0.2..3; HOLD FACING: the nose
    /// kept on the target.
    pub hold_gain: f32,
    pub hold_face: bool,
    /// The gun sight on the glass: 0 off .. 2 bright.
    pub arms_sight: f32,
    /// The camera on the pilot's head: sway under load, tremor under
    /// thrust, jolts from the guns. 0 off .. 2 double.
    pub cam_shake: f32,
    /// The ship's fit: what each hardpoint carries, by [`Hardpoint`] index.
    pub mounts: [Mount; 4],
    /// The bay's hologram: its hue 0..1 and saturation 0..1, scanlines
    /// per pane height, the pane's half width (NDC), and whether it yaws
    /// by itself.
    pub bay_hue: f32,
    pub bay_saturation: f32,
    pub bay_scanlines: f32,
    pub bay_size: f32,
    pub bay_spin: bool,
    /// The pointer's height, a fraction of the screen's.
    pub pointer_size: f32,
    /// Where the SHIP bay's panel sits (top-left, canopy NDC).
    pub bay_anchor: [f32; 2],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            msaa: 4,
            scale: 1.0,
            vsync: true,
            bindings: Bindings::default(),
            layout: Layout::default(),
            look_sensitivity: 1.0,
            hoop_size: 1.0,
            menu_anchor: MENU_ANCHOR_DEFAULT,
            map_anchor: MAP_ANCHOR_DEFAULT,
            readout_anchor: READOUT_ANCHOR_DEFAULT,
            cockpit_frame: true,
            cockpit_glow: 1.0,
            cockpit_hull: 0.92,
            cockpit_res: 0.5,
            fps_floor: 60.0,
            sky: 1.0,
            flare: 1.0,
            fov: 70.0,
            gauge_style: GaugeStyle::Tron,
            gauges_stay: true,
            guide: false,
            hull_sound: true,
            shield: 1.0,
            dials: [DialTweak::DEFAULT; Instrument::ALL.len()],
            landing_spacing_m: 250.0,
            map_rings: 4,
            map_grid: true,
            camera_chase: false,
            holo_view: false,
            holo_size: 0.30,
            holo_anchor: HOLO_ANCHOR_DEFAULT,
            plan: Plan::default(),
            arms_power: 0.5,
            arms_glow: 1.0,
            arms_shards: 24,
            arms_shard_life: 5.0,
            arms_scar_size: 1.0,
            arms_scar_cool: 12.0,
            arms_ore: 1.0,
            mimics_chance: 1.0,
            mimics_hostility: 0.5,
            hold_gain: 1.0,
            hold_face: true,
            arms_sight: 1.0,
            cam_shake: 1.0,
            mounts: STOCK,
            bay_hue: BAY_HUE_DEFAULT,
            bay_saturation: 1.0,
            bay_scanlines: BAY_SCANLINES_DEFAULT,
            bay_size: BAY_SIZE_DEFAULT,
            bay_spin: true,
            pointer_size: POINTER_SIZE_DEFAULT,
            bay_anchor: BAY_ANCHOR_DEFAULT,
        }
    }
}

pub const MSAA_CHOICES: [u32; 4] = [1, 2, 4, 8];

impl Settings {
    pub fn path() -> Option<PathBuf> {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".farfall").join("settings.cfg"))
    }

    /// Read the file, or defaults if there is none.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let s = Self::parse(&text);
                log::info!("settings: loaded {}", path.display());
                s
            }
            Err(_) => Self::default(),
        }
    }

    /// Write the file. Failure is logged, never fatal: a read-only home
    /// must not stop the game.
    pub fn save(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, self.render()) {
            log::warn!("settings: could not write {}: {e}", path.display());
        }
    }

    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "graphics.msaa" => {
                    if let Ok(n) = v.parse::<u32>() {
                        if MSAA_CHOICES.contains(&n) {
                            s.msaa = n;
                        }
                    }
                }
                "graphics.scale" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.scale = f.clamp(0.25, 1.0);
                        }
                    }
                }
                "graphics.vsync" => s.vsync = matches!(v, "on" | "true" | "1"),
                "ui.safe-edge" => {
                    if let Ok(f) = v.trim_end_matches('%').parse::<f32>() {
                        s.layout.set_safe_edge(f / 100.0);
                    }
                }
                "warp.destination" => {
                    if let Some(d) = Destination::from_key(v) {
                        s.plan.dest = d;
                    }
                }
                "warp.safe-radii" => {
                    if let Ok(f) = v.parse::<f64>() {
                        s.plan.set_safe(f);
                    }
                }
                "control.look-sens" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.look_sensitivity = f.clamp(0.1, 5.0);
                        }
                    }
                }
                "ui.panel-menu" => {
                    if let Some(a) = parse_pair(v) {
                        s.menu_anchor = clamp_anchor(a);
                    }
                }
                "ui.panel-readout" => {
                    if let Some(a) = parse_pair(v) {
                        // Glass, not screen: anywhere a dial may go.
                        s.readout_anchor = [a[0].clamp(-1.6, 1.6), a[1].clamp(-1.6, 1.6)];
                    }
                }
                "ui.panel-holo" => {
                    if let Some(a) = parse_pair(v) {
                        s.holo_anchor = clamp_anchor(a);
                    }
                }
                "ui.panel-map" => {
                    if let Some(a) = parse_pair(v) {
                        s.map_anchor = clamp_anchor(a);
                    }
                }
                "ui.panel-bay-card" => {
                    if let Some(a) = parse_pair(v) {
                        s.bay_anchor = clamp_anchor(a);
                    }
                }
                "ship.holo-hue" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.bay_hue = f.rem_euclid(1.0);
                        }
                    }
                }
                "ship.holo-saturation" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.bay_saturation = f.clamp(0.0, 1.0);
                        }
                    }
                }
                "ship.holo-scanlines" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.bay_scanlines = f.clamp(0.0, BAY_SCANLINES_MAX);
                        }
                    }
                }
                "ship.holo-size" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.bay_size = f.clamp(BAY_SIZE_MIN, BAY_SIZE_MAX);
                        }
                    }
                }
                "ui.pointer-size" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.pointer_size = f.clamp(0.02, 0.1);
                        }
                    }
                }
                "ship.holo-spin" => match v {
                    "on" => s.bay_spin = true,
                    "off" => s.bay_spin = false,
                    _ => {}
                },
                "cockpit.frame" => match v {
                    "on" => s.cockpit_frame = true,
                    "off" => s.cockpit_frame = false,
                    _ => {}
                },
                "cockpit.glow" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.cockpit_glow = f.clamp(0.25, 2.0);
                        }
                    }
                }
                "graphics.fov" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.fov = f.clamp(FOV_MIN, FOV_MAX);
                        }
                    }
                }
                "ui.gauge-style" => {
                    if let Some(style) = GaugeStyle::from_key(v) {
                        s.gauge_style = style;
                    }
                }
                "ui.gauges" => match v {
                    "stay" => s.gauges_stay = true,
                    "fade" => s.gauges_stay = false,
                    _ => {}
                },
                "arms.power" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.arms_power = f.clamp(0.0, 1.0);
                        }
                    }
                }
                "arms.shards" => {
                    if let Ok(n) = v.parse::<u32>() {
                        s.arms_shards = n.min(ARMS_SHARDS_MAX);
                    }
                }
                "cam.shake" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.cam_shake = f.clamp(0.0, 2.0);
                        }
                    }
                }
                "arms.sight" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.arms_sight = f.clamp(0.0, 2.0);
                        }
                    }
                }
                "arms.scar-size" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.arms_scar_size = f.clamp(0.0, 2.0);
                        }
                    }
                }
                "arms.scar-cool" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.arms_scar_cool = f.clamp(2.0, 60.0);
                        }
                    }
                }
                "arms.ore" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.arms_ore = f.clamp(0.0, 2.0);
                        }
                    }
                }
                "mimics.chance" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.mimics_chance = f.clamp(0.0, 1.0);
                        }
                    }
                }
                "mimics.hostility" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.mimics_hostility = f.clamp(0.0, 1.0);
                        }
                    }
                }
                "hold.gain" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.hold_gain = f.clamp(0.2, 3.0);
                        }
                    }
                }
                "hold.face" => s.hold_face = v == "on",
                "arms.shard-life" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.arms_shard_life = f.clamp(1.0, 12.0);
                        }
                    }
                }
                "arms.glow" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.arms_glow = f.clamp(0.0, 2.0);
                        }
                    }
                }
                "ui.shield" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.shield = f.clamp(0.0, 2.0);
                        }
                    }
                }
                "camera.chase" => match v {
                    "on" => s.camera_chase = true,
                    "off" => s.camera_chase = false,
                    _ => {}
                },
                "holo.view" => match v {
                    "on" => s.holo_view = true,
                    "off" => s.holo_view = false,
                    _ => {}
                },
                "holo.size" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.holo_size = f.clamp(HOLO_SIZE_MIN, HOLO_SIZE_MAX);
                        }
                    }
                }
                "sound.hull" => match v {
                    "on" => s.hull_sound = true,
                    "off" => s.hull_sound = false,
                    _ => {}
                },
                "ui.guide" => match v {
                    "on" => s.guide = true,
                    "off" => s.guide = false,
                    _ => {}
                },
                "graphics.flare" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.flare = f.clamp(0.0, 2.0);
                        }
                    }
                }
                "graphics.sky" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.sky = f.clamp(0.0, 2.0);
                        }
                    }
                }
                "graphics.fps-floor" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if FPS_FLOOR_CHOICES.contains(&f) {
                            s.fps_floor = f;
                        }
                    }
                }
                "cockpit.res" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if COCKPIT_RES_CHOICES.contains(&f) {
                            s.cockpit_res = f;
                        }
                    }
                }
                "cockpit.hull" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.cockpit_hull = f.clamp(0.0, 1.0);
                        }
                    }
                }
                "ui.landing-hoops" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if LANDING_SPACINGS.contains(&f) {
                            s.landing_spacing_m = f;
                        }
                    }
                }
                "map.rings" => {
                    if let Ok(n) = v.parse::<u32>() {
                        s.map_rings = n.min(crate::map::RINGS_MAX);
                    }
                }
                "map.grid" => match v {
                    "on" => s.map_grid = true,
                    "off" => s.map_grid = false,
                    _ => {}
                },
                "ui.hoop-size" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.hoop_size = f.clamp(HOOP_SIZE_MIN, HOOP_SIZE_MAX);
                        }
                    }
                }
                k2 if k2
                    .strip_prefix("control.")
                    .is_some_and(|name| Named::ALL.iter().any(|n| n.key() == name)) =>
                {
                    let name = k2.strip_prefix("control.").unwrap();
                    let n = Named::ALL
                        .iter()
                        .copied()
                        .find(|n| n.key() == name)
                        .unwrap();
                    if let Some(key) = key_from_name(v) {
                        s.bindings.bind_named(n, key);
                    }
                }
                k if k.starts_with("ship.hardpoint.") => {
                    let n = k["ship.hardpoint.".len()..].parse::<usize>().ok();
                    if let (Some(n), Some(m)) =
                        (n.filter(|&n| n < s.mounts.len()), Mount::from_key(v))
                    {
                        s.mounts[n] = m;
                    }
                }
                _ => {
                    if let Some(name) = k.strip_prefix("control.") {
                        if let (Some(action), Some(key)) = (
                            Action::ALL.iter().copied().find(|a| a.key() == name),
                            key_from_name(v),
                        ) {
                            s.bindings.bind(action, key);
                        }
                    } else if let Some((name, field)) =
                        k.strip_prefix("ui.").and_then(|rest| rest.split_once('.'))
                    {
                        // A dial's own setting: ui.<dial>.<field>.
                        if let Some(inst) =
                            Instrument::ALL.iter().copied().find(|i| i.key() == name)
                        {
                            let d = &mut s.dials[inst as usize];
                            match field {
                                "size" => {
                                    if let Ok(f) = v.parse::<f32>() {
                                        if f.is_finite() {
                                            d.size = f.clamp(DIAL_SIZE_MIN, DIAL_SIZE_MAX);
                                        }
                                    }
                                }
                                "style" => {
                                    d.style = if v == "auto" {
                                        None
                                    } else {
                                        GaugeStyle::from_key(v).or(d.style)
                                    }
                                }
                                "tilt" => {
                                    if let Ok(f) = v.parse::<f32>() {
                                        if f.is_finite() {
                                            d.tilt_deg = f.clamp(TILT_MIN, TILT_MAX);
                                        }
                                    }
                                }
                                "fade" => {
                                    d.stay = match v {
                                        "auto" => None,
                                        "stay" => Some(true),
                                        "fade" => Some(false),
                                        _ => d.stay,
                                    }
                                }
                                _ => {}
                            }
                        }
                    } else if let Some(name) = k.strip_prefix("ui.") {
                        let inst = Instrument::ALL.iter().copied().find(|i| i.key() == name);
                        // "slot at x,y": the slot, then the dragged anchor.
                        let (slot_key, free) = match v.split_once(" at ") {
                            Some((sk, at)) => (sk.trim(), parse_pair(at)),
                            None => (v, None),
                        };
                        if let (Some(inst), Some(slot)) = (inst, Slot::from_key(slot_key)) {
                            // A dial cannot be "on" and an overlay has no
                            // slot: keep each to its own choices.
                            let valid = if inst.slotted() {
                                Slot::DIALS.contains(&slot)
                            } else {
                                Slot::OVERLAYS.contains(&slot)
                            };
                            if valid {
                                s.layout.set(inst, slot);
                                if let Some(at) = free {
                                    s.layout.set_free(inst, at);
                                }
                            }
                        }
                    }
                }
            }
        }
        s
    }

    pub fn render(&self) -> String {
        let mut out = String::from("# FARFALL settings — edited by the in-game menu (Esc)\n");
        out.push_str(&format!("graphics.msaa = {}\n", self.msaa));
        out.push_str(&format!("graphics.scale = {:.2}\n", self.scale));
        out.push_str(&format!(
            "graphics.vsync = {}\n",
            if self.vsync { "on" } else { "off" }
        ));
        for a in Action::ALL {
            out.push_str(&format!(
                "control.{} = {}\n",
                a.key(),
                key_name(self.bindings.key_for(a))
            ));
        }
        for n in Named::ALL {
            out.push_str(&format!(
                "control.{} = {}\n",
                n.key(),
                key_name(self.bindings.named(n))
            ));
        }
        out.push_str(&format!(
            "control.look-sens = {:.2}\n",
            self.look_sensitivity
        ));
        out.push_str(&format!("ui.hoop-size = {:.2}\n", self.hoop_size));
        out.push_str(&format!(
            "ui.panel-menu = {:.3},{:.3}\n",
            self.menu_anchor[0], self.menu_anchor[1]
        ));
        out.push_str(&format!(
            "ui.panel-map = {:.3},{:.3}\n",
            self.map_anchor[0], self.map_anchor[1]
        ));
        out.push_str(&format!(
            "ui.panel-readout = {:.3},{:.3}\n",
            self.readout_anchor[0], self.readout_anchor[1]
        ));
        out.push_str(&format!(
            "cockpit.frame = {}\n",
            if self.cockpit_frame { "on" } else { "off" }
        ));
        out.push_str(&format!("cockpit.glow = {:.2}\n", self.cockpit_glow));
        out.push_str(&format!("cockpit.hull = {:.2}\n", self.cockpit_hull));
        out.push_str(&format!("cockpit.res = {:.2}\n", self.cockpit_res));
        out.push_str(&format!("graphics.fps-floor = {:.0}\n", self.fps_floor));
        out.push_str(&format!("graphics.sky = {:.2}\n", self.sky));
        out.push_str(&format!("graphics.flare = {:.2}\n", self.flare));
        out.push_str(&format!("graphics.fov = {:.0}\n", self.fov));
        out.push_str(&format!("ui.gauge-style = {}\n", self.gauge_style.key()));
        out.push_str(&format!(
            "ui.gauges = {}\n",
            if self.gauges_stay { "stay" } else { "fade" }
        ));
        out.push_str(&format!(
            "ui.guide = {}\n",
            if self.guide { "on" } else { "off" }
        ));
        out.push_str(&format!("ui.shield = {:.2}\n", self.shield));
        out.push_str(&format!("arms.power = {:.2}\n", self.arms_power));
        out.push_str(&format!("arms.glow = {:.2}\n", self.arms_glow));
        out.push_str(&format!("arms.shards = {}\n", self.arms_shards));
        out.push_str(&format!("arms.shard-life = {:.1}\n", self.arms_shard_life));
        out.push_str(&format!("arms.scar-size = {:.2}\n", self.arms_scar_size));
        out.push_str(&format!("arms.scar-cool = {:.0}\n", self.arms_scar_cool));
        out.push_str(&format!("arms.ore = {:.2}\n", self.arms_ore));
        out.push_str(&format!("mimics.chance = {:.3}\n", self.mimics_chance));
        out.push_str(&format!(
            "mimics.hostility = {:.2}\n",
            self.mimics_hostility
        ));
        out.push_str(&format!("hold.gain = {:.2}\n", self.hold_gain));
        out.push_str(&format!(
            "hold.face = {}\n",
            if self.hold_face { "on" } else { "off" }
        ));
        out.push_str(&format!("arms.sight = {:.2}\n", self.arms_sight));
        out.push_str(&format!("cam.shake = {:.2}\n", self.cam_shake));
        out.push_str(&format!(
            "camera.chase = {}\n",
            if self.camera_chase { "on" } else { "off" }
        ));
        out.push_str(&format!(
            "holo.view = {}\n",
            if self.holo_view { "on" } else { "off" }
        ));
        out.push_str(&format!("holo.size = {:.2}\n", self.holo_size));
        out.push_str(&format!(
            "ui.panel-holo = {:.3},{:.3}\n",
            self.holo_anchor[0], self.holo_anchor[1]
        ));
        for (n, m) in self.mounts.iter().enumerate() {
            out.push_str(&format!("ship.hardpoint.{n} = {}\n", m.key()));
        }
        out.push_str(&format!("ship.holo-hue = {:.3}\n", self.bay_hue));
        out.push_str(&format!(
            "ship.holo-saturation = {:.2}\n",
            self.bay_saturation
        ));
        out.push_str(&format!(
            "ship.holo-scanlines = {:.0}\n",
            self.bay_scanlines
        ));
        out.push_str(&format!("ship.holo-size = {:.3}\n", self.bay_size));
        out.push_str(&format!(
            "ship.holo-spin = {}\n",
            if self.bay_spin { "on" } else { "off" }
        ));
        out.push_str(&format!("ui.pointer-size = {:.3}\n", self.pointer_size));
        out.push_str(&format!(
            "ui.panel-bay-card = {:.3},{:.3}\n",
            self.bay_anchor[0], self.bay_anchor[1]
        ));
        out.push_str(&format!(
            "sound.hull = {}\n",
            if self.hull_sound { "on" } else { "off" }
        ));
        out.push_str(&format!(
            "ui.landing-hoops = {:.0}\n",
            self.landing_spacing_m
        ));
        out.push_str(&format!("map.rings = {}\n", self.map_rings));
        out.push_str(&format!(
            "map.grid = {}\n",
            if self.map_grid { "on" } else { "off" }
        ));
        out.push_str(&format!("warp.destination = {}\n", self.plan.dest.key()));
        out.push_str(&format!("warp.safe-radii = {:.3}\n", self.plan.safe_radii));
        for i in Instrument::ALL {
            match self.layout.free(i) {
                Some([x, y]) => out.push_str(&format!(
                    "ui.{} = {} at {:.3},{:.3}\n",
                    i.key(),
                    self.layout.get(i).key(),
                    x,
                    y
                )),
                None => out.push_str(&format!("ui.{} = {}\n", i.key(), self.layout.get(i).key())),
            }
        }
        for i in Instrument::ALL {
            let d = self.dials[i as usize];
            if d != DialTweak::DEFAULT {
                out.push_str(&format!("ui.{}.size = {:.2}\n", i.key(), d.size));
                out.push_str(&format!(
                    "ui.{}.style = {}\n",
                    i.key(),
                    d.style.map_or("auto", |st| st.key())
                ));
                out.push_str(&format!(
                    "ui.{}.fade = {}\n",
                    i.key(),
                    match d.stay {
                        None => "auto",
                        Some(true) => "stay",
                        Some(false) => "fade",
                    }
                ));
                out.push_str(&format!("ui.{}.tilt = {:.0}\n", i.key(), d.tilt_deg));
            }
        }
        out.push_str(&format!(
            "ui.safe-edge = {:.0}%\n",
            self.layout.safe_edge * 100.0
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode;

    #[test]
    fn defaults_round_trip() {
        let s = Settings::default();
        assert_eq!(Settings::parse(&s.render()), s);
    }

    #[test]
    fn edits_round_trip() {
        let mut s = Settings {
            msaa: 2,
            scale: 0.75,
            vsync: false,
            ..Default::default()
        };
        s.bindings.bind(Action::PitchUp, KeyCode::KeyI);
        s.bindings.bind_named(Named::Boost, KeyCode::ControlLeft);
        s.layout.set(Instrument::Gyro, Slot::TopCentre);
        s.layout.set(Instrument::Horizon, Slot::Off);
        s.layout.set_safe_edge(0.07);
        s.layout.set_free(Instrument::Speed, [0.125, -0.5]);
        s.look_sensitivity = 1.75;
        s.hull_sound = false;
        s.shield = 1.5;
        s.arms_power = 0.75;
        s.arms_glow = 1.5;
        s.arms_shards = 40;
        s.arms_shard_life = 8.0;
        s.arms_scar_size = 1.5;
        s.arms_scar_cool = 30.0;
        s.arms_ore = 1.5;
        s.mimics_chance = 0.25;
        s.mimics_hostility = 0.75;
        s.hold_gain = 1.75;
        s.hold_face = false;
        s.arms_sight = 0.5;
        s.cam_shake = 1.75;
        s.hoop_size = 2.5;
        s.map_rings = 2;
        s.landing_spacing_m = 500.0;
        s.cockpit_frame = false;
        s.cockpit_glow = 1.5;
        s.cockpit_hull = 0.25;
        s.cockpit_res = 1.0;
        s.fps_floor = 90.0;
        s.sky = 1.5;
        s.flare = 0.5;
        s.fov = 85.0;
        s.gauge_style = GaugeStyle::Dial;
        s.gauges_stay = false;
        s.guide = true;
        s.dials[Instrument::Speed as usize] = DialTweak {
            size: 1.5,
            style: Some(GaugeStyle::Dial),
            stay: Some(true),
            tilt_deg: 45.0,
        };
        s.dials[Instrument::Gyro as usize].stay = Some(false);
        s.menu_anchor = [-0.25, 0.5];
        s.map_anchor = [0.125, -0.125];
        s.bay_anchor = [0.5, -0.25];
        s.mounts = [Mount::Cannon, Mount::Empty, Mount::Rail, Mount::Cannon];
        s.bay_hue = 0.11;
        s.bay_saturation = 0.5;
        s.bay_scanlines = 60.0;
        s.bay_size = 0.2;
        s.bay_spin = false;
        s.pointer_size = 0.06;
        s.readout_anchor = [-0.5, 0.25];
        s.map_grid = false;
        s.camera_chase = true;
        s.holo_view = true;
        s.holo_size = 0.40;
        s.holo_anchor = [-0.375, 0.25];
        s.plan.dest = Destination::Moon;
        s.plan.set_safe(3.5);
        assert_eq!(Settings::parse(&s.render()), s);
    }

    #[test]
    fn dials_stay_lit_by_default_and_every_style_is_open_to_every_dial() {
        assert!(
            Settings::default().gauges_stay,
            "gauges do not fade unless asked"
        );
        for i in Instrument::ALL {
            let mut cur = None;
            let mut seen = vec![cur];
            for _ in 0..3 {
                cur = next_dial_style(cur, i, true);
                seen.push(cur);
            }
            assert_eq!(
                seen,
                vec![
                    None,
                    Some(GaugeStyle::Tron),
                    Some(GaugeStyle::Jet),
                    Some(GaugeStyle::Dial)
                ],
                "{i:?}"
            );
            assert_eq!(next_dial_style(cur, i, true), None, "and round");
            assert_eq!(next_dial_style(None, i, false), Some(GaugeStyle::Dial));
            assert_eq!(style_for(GaugeStyle::Jet, i), GaugeStyle::Jet);
        }
    }

    #[test]
    fn a_tilt_is_held_within_reach() {
        let s = Settings::parse("ui.speed.tilt = 95\nui.gyro.tilt = -12\nui.g-meter.tilt = nan\n");
        assert_eq!(s.dials[Instrument::Speed as usize].tilt_deg, TILT_MAX);
        assert_eq!(s.dials[Instrument::Gyro as usize].tilt_deg, -12.0);
        assert_eq!(s.dials[Instrument::GForce as usize].tilt_deg, 0.0);
        assert!(s.render().contains("ui.gyro.tilt = -12\n"));
    }

    #[test]
    fn garbage_falls_back_to_defaults() {
        let s = Settings::parse(
            "graphics.msaa = 3\ngraphics.scale = nope\nui.gyro = on\nnonsense\ncontrol.pitch-up = ESCAPE\n",
        );
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn a_key_means_one_thing() {
        let mut b = Bindings::default();
        // I takes pitch-up; pitch-up's old key (Up) goes to whatever I had (nothing).
        assert!(b.bind(Action::PitchUp, KeyCode::KeyI));
        assert_eq!(b.key_for(Action::PitchUp), KeyCode::KeyI);
        // W (thrust forward) rebound to pitch-up: thrust-forward takes I.
        assert!(b.bind(Action::PitchUp, KeyCode::KeyW));
        assert_eq!(b.key_for(Action::ThrustForward), KeyCode::KeyI);
        assert_eq!(b.action_for(KeyCode::KeyW), Some(Action::PitchUp));
        // Reserved keys are refused.
        assert!(!b.bind(Action::YawLeft, KeyCode::Escape));
    }
}
