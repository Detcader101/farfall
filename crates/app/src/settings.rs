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
use crate::stick::StickMap;
use crate::warp::{Destination, Plan, LENGTH_MAX, LENGTH_MIN};
pub use farfall_render::post::Tonemap;
use std::path::PathBuf;

/// "x,y" → a pair of finite numbers.
fn parse_pair(v: &str) -> Option<[f32; 2]> {
    let (a, b) = v.split_once(',')?;
    let x = a.trim().parse::<f32>().ok()?;
    let y = b.trim().parse::<f32>().ok()?;
    (x.is_finite() && y.is_finite()).then_some([x, y])
}

/// Where the settings menu's card is centred and the map pane is centred,
/// until the pilot drags them. The card is a fixed size in canopy units
/// (its width over the aspect on the screen), so it is kept by its
/// centre and stays centred on any screen.
pub const MENU_ANCHOR_DEFAULT: [f32; 2] = [0.0, 0.04];
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
/// The readout: the top-left corner of the glass, clear of the arch.
pub const READOUT_ANCHOR_DEFAULT: [f32; 2] = [-0.96, 0.94];

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
    /// Leaned sideways about its own upright, degrees (−60..60): the
    /// face turned toward a seat off to one side.
    pub lean_deg: f32,
    /// The face turned in its own plane, degrees (−180..180). The plate
    /// and its markings turn together, so the needle still reads true.
    pub rotate_deg: f32,
}

pub const TILT_MIN: f32 = -60.0;
pub const TILT_MAX: f32 = 60.0;
pub const ROTATE_MIN: f32 = -180.0;
pub const ROTATE_MAX: f32 = 180.0;

impl DialTweak {
    pub const DEFAULT: DialTweak = DialTweak {
        size: 1.0,
        style: None,
        stay: None,
        tilt_deg: 0.0,
        lean_deg: 0.0,
        rotate_deg: 0.0,
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
        Some(GaugeStyle::Warthog),
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
    /// The A-10's steam gauges: black faces, white needles and numerals,
    /// a metal bezel, set into the dash.
    Warthog,
}

impl GaugeStyle {
    pub const ALL: [GaugeStyle; 4] = [
        GaugeStyle::Tron,
        GaugeStyle::Jet,
        GaugeStyle::Dial,
        GaugeStyle::Warthog,
    ];

    pub fn key(self) -> &'static str {
        match self {
            GaugeStyle::Tron => "tron",
            GaugeStyle::Jet => "jet",
            GaugeStyle::Dial => "dial",
            GaugeStyle::Warthog => "warthog",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            GaugeStyle::Tron => "TRON",
            GaugeStyle::Jet => "JET",
            GaugeStyle::Dial => "DIAL",
            GaugeStyle::Warthog => "WARTHOG",
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
            // The cabin: unlit metal bezels (the socket itself is a DIAL's).
            GaugeStyle::Warthog => 3,
        }
    }
}

/// Exposure, a multiplier on the scene's own: two stops either way.
pub const EXPOSURE_MIN: f32 = 0.25;
pub const EXPOSURE_MAX: f32 = 4.0;

/// Field of view limits, degrees.
pub const FOV_MIN: f32 = 50.0;
pub const FOV_MAX: f32 = 110.0;

/// The cabin's render sizes on offer, as fractions of the scene.
pub const COCKPIT_RES_CHOICES: [f32; 3] = [0.5, 0.75, 1.0];

/// Frame-rate floors on offer (0: none). The cabin's moving detail gives
/// way while turning the head would cost more than the floor allows.
/// The holo3PP floats in the glass at the upper right, under the mini map
/// and outside the arch: clear of the five dials on the dash (it used to
/// stand between the speed dial and the G meter, over both) and of the
/// forward view. Pinned by `the_stock_hologram_sits_clear_of_the_dials_and_the_mini_map`.
pub const HOLO_ANCHOR_DEFAULT: [f32; 2] = [0.81, 0.30];
pub const HOLO_SIZE_MIN: f32 = 0.16;
pub const HOLO_SIZE_MAX: f32 = 0.50;
/// HOLO RANGE: how much space the hologram shows round the ship, as a
/// multiple of the stock (the ship shrinks by the same factor).
pub const HOLO_RANGE_MIN: f32 = 1.0;
pub const HOLO_RANGE_MAX: f32 = 4.0;
pub const HOLO_RANGE_STEP: f32 = 0.5;
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
    /// The file format's version (see [`SETTINGS_VERSION`]).
    pub version: u32,
    pub msaa: u32,
    pub scale: f32,
    /// The world's scale governs itself to hold the FPS floor: RENDER
    /// SCALE is then the ceiling, never the floor.
    pub auto_scale: bool,
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
    /// The control column on the console mirrors the live stick demand.
    pub cockpit_stick: bool,
    /// The least frame rate the pilot will have, or 0 for no floor.
    pub fps_floor: f32,
    /// The daytime sky's strength low down, 1 = stock.
    pub sky: f32,
    /// The ground's live relief: 0 the baked continents only, 1 stock,
    /// 2 an octave finer.
    pub terrain_detail: f32,
    /// The cloud deck: a multiplier on the preset's coverage, 0 clears it.
    pub clouds: f32,
    /// The night side's cities, 0 (dark) .. 2; 1 = stock.
    pub city_lights: f32,
    /// The lens flare's strength, 1 = stock, 0 none.
    pub flare: f32,
    /// The picture (the post pass): the bloom's strength, 0 (none) .. 2;
    /// 1 = stock.
    pub bloom: f32,
    /// Exposure, a multiplier on the scene's own: 0.25 .. 4 (±2 stops),
    /// 1 = stock. The eye's slow drift about it is built in.
    pub exposure: f32,
    /// The curve from radiance to the screen: OFF (a clip), SOFT, AGX.
    pub tonemap: Tonemap,
    /// The glass rim's chromatic fringing, 0 (none) .. 2; 1 = a hair.
    pub fringe: f32,
    /// Space dust and cabin motes: 0 none, 1 stock, up to 2.
    pub dust: f32,
    /// The nebula's glow, 0 (off) .. 3; 1 = stock.
    pub nebula: f32,
    /// Which nebula: the seed picks where the clouds sit and their shapes.
    pub nebula_seed: u32,
    /// How fine the gas is across the sky, 1 (broad veils) .. 8 (knots).
    pub nebula_scale: f32,
    /// How much of a cloud is gas, 0 (thin wisps) .. 1 (solid banks).
    pub nebula_density: f32,
    /// How many clouds there are, 1..8.
    pub nebula_clouds: u32,
    /// The two hues the gas drifts between, 0..1 around the wheel.
    pub nebula_hue: f32,
    pub nebula_hue2: f32,
    /// How far each cloud spreads, 0.25 (a knot) .. 3 (across the sky).
    pub nebula_spread: f32,
    /// Base field of view, degrees (vertical).
    pub fov: f32,
    /// Gauge style: WARTHOG (the default) steam gauges on the dash; TRON
    /// holograms on the glass; JET holograms over thin rings, the gyro a
    /// real ball; DIAL period instruments on the dash. Nothing is ever
    /// hollowed into the dash for any of them.
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
    /// LANDING ASSIST: in LANDING mode with a touchdown ahead, the flight
    /// computer holds the hull level over the ground on any axis the
    /// pilot is not using.
    pub landing_assist: bool,
    /// The landing pad: a ring drawn on the ground at the predicted
    /// touchdown, in LANDING mode.
    pub landing_pad: bool,
    /// Rings drawn around each body on the map, 0..=6.
    pub map_rings: u32,
    /// The map's reference grid.
    pub map_grid: bool,
    /// Helicopters parked on pads down on the planet.
    pub helis: bool,
    /// The chase camera: the whole view from outside the ship (the dev
    /// third person the holo3PP is measured against).
    pub camera_chase: bool,
    /// The holo3PP: a live volumetric hologram of the ship and its
    /// neighbourhood, over an emitter in the dash.
    pub holo_view: bool,
    /// The hologram's size (its radius is this times half a metre).
    pub holo_size: f32,
    /// How much space the hologram shows round the ship, 1 (the ship
    /// fills it) .. 4 (four times the room, the ship a quarter the size).
    pub holo_range: f32,
    /// The CONTROLS card at every start (it always shows on the first
    /// run, and F1 shows it any time).
    pub controls_card: bool,
    /// The hologram's emitter: a glass anchor (canopy NDC) whose direction
    /// meets the dash at the socket.
    pub holo_anchor: [f32; 2],
    /// The wormhole drive's destination and safe distance.
    pub plan: Plan,
    /// The jump sequence's length: a scale on the stock eight seconds,
    /// 0.5 (four seconds) .. 2 (sixteen); 1 = stock.
    pub warp_length: f32,
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
    /// MIMIC SIZE: a mimic hull over our own fighter, 0.5..3 (1 stock: the
    /// same ship).
    pub mimics_size: f32,
    /// MINERS: how many miner ships work the ring about us, 0..8, and
    /// MINER GROWTH: how fast they haul and so grow, 0.25..4 (1 stock).
    pub miners_count: u32,
    pub miners_growth: f32,
    /// HOLD GAIN: how hard the lock holds, 0.2..3; HOLD FACING: the nose
    /// kept on the target.
    pub hold_gain: f32,
    pub hold_face: bool,
    /// The gun sight on the glass: 0 off .. 2 bright.
    pub arms_sight: f32,
    /// The camera on the pilot's head: sway under load, tremor under
    /// thrust, jolts from the guns. 0 off .. 2 double.
    pub cam_shake: f32,
    /// How hard the chaos drive jostles the ship (and so the camera) on
    /// the way to the slip, 0..2 of the stock. The warning stays; the
    /// violence is the pilot's to choose.
    pub drive_shake: f32,
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
    /// The stick: which raw axis and button is which control (stick.*).
    pub stick: StickMap,
}

impl Settings {
    /// The browser's first run: the same picture as native, with the
    /// world's scale governing itself to the FPS floor on whatever
    /// machine opened the link. Saved settings override this.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn web_default() -> Self {
        Self {
            auto_scale: true,
            ..Self::default()
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            msaa: 4,
            scale: 1.0,
            auto_scale: false,
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
            cockpit_stick: true,
            fps_floor: 60.0,
            sky: 1.0,
            terrain_detail: 1.0,
            clouds: 1.0,
            city_lights: 1.0,
            nebula: 1.0,
            nebula_seed: 7,
            nebula_scale: 3.0,
            nebula_density: 0.55,
            nebula_clouds: 4,
            nebula_hue: 0.78,
            nebula_hue2: 0.55,
            nebula_spread: 1.5,
            flare: 1.0,
            bloom: 1.0,
            exposure: 1.0,
            tonemap: Tonemap::Agx,
            fringe: 1.0,
            dust: 1.0,
            fov: 70.0,
            gauge_style: GaugeStyle::Warthog,
            gauges_stay: true,
            guide: false,
            hull_sound: true,
            shield: 1.0,
            dials: [DialTweak::DEFAULT; Instrument::ALL.len()],
            landing_spacing_m: 250.0,
            landing_assist: true,
            landing_pad: true,
            map_rings: 4,
            map_grid: true,
            helis: true,
            camera_chase: false,
            holo_view: true,
            holo_size: 0.18,
            holo_range: 1.0,
            controls_card: false,
            holo_anchor: HOLO_ANCHOR_DEFAULT,
            plan: Plan::default(),
            warp_length: 1.0,
            arms_power: 0.5,
            arms_glow: 1.0,
            arms_shards: 24,
            arms_shard_life: 5.0,
            arms_scar_size: 1.0,
            arms_scar_cool: 12.0,
            arms_ore: 1.0,
            mimics_chance: 1.0,
            mimics_hostility: 0.5,
            mimics_size: 1.0,
            miners_count: 4,
            miners_growth: 1.0,
            hold_gain: 1.0,
            hold_face: true,
            arms_sight: 1.0,
            cam_shake: 1.0,
            // A warning, not a beating: the gauges must stay readable to
            // the slip. Turn it up on the CABIN page if you want the
            // violence.
            drive_shake: 0.0,
            mounts: STOCK,
            bay_hue: BAY_HUE_DEFAULT,
            bay_saturation: 1.0,
            bay_scanlines: BAY_SCANLINES_DEFAULT,
            bay_size: BAY_SIZE_DEFAULT,
            bay_spin: true,
            pointer_size: POINTER_SIZE_DEFAULT,
            bay_anchor: BAY_ANCHOR_DEFAULT,
            stick: StickMap::default(),
        }
    }
}

pub const MSAA_CHOICES: [u32; 4] = [1, 2, 4, 8];

/// Every key the settings file may hold, by name or by pattern (a `*`
/// stands for one segment: an action's key, a dial's key). The menu's
/// coverage test walks this list: a key here with no menu row is a
/// setting a pilot cannot find, save the panel anchors, which are set by
/// dragging the panel itself.
#[cfg(test)]
pub const KEYS: &[&str] = &[
    "graphics.msaa",
    "graphics.scale",
    "graphics.vsync",
    "graphics.auto-scale",
    "graphics.fov",
    "graphics.fps-floor",
    "graphics.sky",
    "graphics.flare",
    "graphics.bloom",
    "graphics.exposure",
    "graphics.tonemap",
    "graphics.fringe",
    "graphics.dust",
    "graphics.terrain-detail",
    "graphics.clouds",
    "graphics.city-lights",
    "graphics.nebula",
    "graphics.nebula-seed",
    "graphics.nebula-scale",
    "graphics.nebula-density",
    "graphics.nebula-clouds",
    "graphics.nebula-hue",
    "graphics.nebula-hue2",
    "graphics.nebula-spread",
    "camera.chase",
    "holo.view",
    "holo.size",
    "holo.range",
    "cam.shake",
    "cam.drive-shake",
    "cockpit.frame",
    "cockpit.glow",
    "cockpit.hull",
    "cockpit.res",
    "cockpit.stick",
    "ui.gauges",
    "ui.gauge-style",
    "ui.guide",
    "ui.shield",
    "ui.hoop-size",
    "ui.landing-hoops",
    "ui.safe-edge",
    "ui.controls-card",
    "ui.pointer-size",
    "ui.*",
    "ui.*.size",
    "ui.*.style",
    "ui.*.fade",
    "ui.*.tilt",
    "ui.*.lean",
    "ui.*.rotate",
    "map.rings",
    "map.grid",
    "world.helis",
    "warp.destination",
    "warp.safe-radii",
    "warp.length",
    "sound.hull",
    "control.*",
    "control.look-sens",
    "stick.enabled",
    "stick.pitch",
    "stick.yaw",
    "stick.roll",
    "stick.throttle",
    "stick.strafe",
    "stick.lift",
    "stick.deadzone",
    "stick.curve",
    "stick.throttle-zero",
    "stick.throttle-brake",
    "stick.throttle-jump",
    "stick.layout",
    "stick.fire",
    "stick.button.*",
    "arms.power",
    "arms.glow",
    "arms.sight",
    "arms.shards",
    "arms.shard-life",
    "arms.scar-size",
    "arms.scar-cool",
    "arms.ore",
    "mimics.chance",
    "mimics.hostility",
    "hold.gain",
    "hold.face",
    "ship.hardpoint.*",
    "ship.holo-hue",
    "ship.holo-saturation",
    "ship.holo-scanlines",
    "ship.holo-size",
    "ship.holo-spin",
    "mimics.size",
    "miners.count",
    "miners.growth",
    "landing.assist",
    "landing.pad",
];

/// Keys the file carries for itself, with no menu row to reach them.
#[cfg(test)]
pub const FILE_ONLY_KEYS: &[&str] = &["settings.version"];

/// The anchors: set by dragging a panel, never by a row.
#[cfg(test)]
pub const DRAGGED_KEYS: &[&str] = &[
    "ui.panel-menu",
    "ui.panel-map",
    "ui.panel-readout",
    "ui.panel-holo",
    "ui.panel-bay-card",
];

/// Does a key match a listed name or pattern?
#[cfg(test)]
pub fn key_matches(pattern: &str, key: &str) -> bool {
    let (mut p, mut k) = (pattern.split('.'), key.split('.'));
    loop {
        match (p.next(), k.next()) {
            (None, None) => return true,
            (Some(ps), Some(ks)) if ps == "*" || ps == ks => {}
            _ => return false,
        }
    }
}

/// The settings file's format version, written as `settings.version`.
/// A file without the line predates the DRIVE SHAKE stock change (40% →
/// 12%): parse adopts the new stock for exactly the old stock value, and
/// keeps anything else — an explicit choice survives, an old default
/// does not.
pub const SETTINGS_VERSION: u32 = 2;
/// What `cam.drive-shake` used to default to, before the polish pass.
const OLD_DRIVE_SHAKE_STOCK: f32 = 0.40;

/// The pilot's home: `HOME` where a shell sets it, else Windows'
/// `USERPROFILE`. A plain Windows launch (Explorer, a shortcut) has no
/// HOME at all — without the fallback nothing was ever saved there.
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

impl Settings {
    pub fn path() -> Option<PathBuf> {
        home_dir().map(|h| h.join(".farfall").join("settings.cfg"))
    }

    /// Is there a settings file (or saved web settings) at all? Its
    /// absence is the first run: the CONTROLS card shows itself.
    pub fn file_exists() -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            return crate::web::storage_get("farfall.settings").is_some();
        }
        #[allow(unreachable_code)]
        Self::path().is_some_and(|p| p.is_file())
    }

    /// Read the file, or defaults if there is none.
    pub fn load() -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            return match crate::web::storage_get("farfall.settings") {
                Some(text) => Self::parse(&text),
                None => Self::web_default(),
            };
        }
        #[allow(unreachable_code)]
        let Some(path) = Self::path() else {
            return Self::with_env_hud(Self::default());
        };
        let s = match std::fs::read_to_string(&path) {
            Ok(text) => {
                let s = Self::parse(&text);
                log::info!("settings: loaded {}", path.display());
                s
            }
            Err(_) => Self::default(),
        };
        Self::with_env_hud(s)
    }

    /// FARFALL_HUD=path: wear a HUD layout file (.fhud) for this run —
    /// the bench's way to stage a cockpit, and a way to try a shared one.
    /// (On the web there is no environment: var() errs, this is a no-op.)
    fn with_env_hud(s: Self) -> Self {
        let Ok(p) = std::env::var("FARFALL_HUD") else {
            return s;
        };
        match std::fs::read_to_string(&p) {
            Ok(text) => {
                log::info!("hud: wearing {p}");
                crate::hud_file::apply(&s, &text)
            }
            Err(e) => {
                log::warn!("hud: could not read {p}: {e}");
                s
            }
        }
    }

    /// Write the file. Failure is logged, never fatal: a read-only home
    /// must not stop the game.
    pub fn save(&self) {
        #[cfg(target_arch = "wasm32")]
        {
            crate::web::storage_set("farfall.settings", &self.render());
            return;
        }
        #[allow(unreachable_code)]
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
        let mut saw_version = false;
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
                "settings.version" => {
                    if let Ok(n) = v.parse::<u32>() {
                        s.version = n.max(1);
                        saw_version = true;
                    }
                }
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
                "graphics.auto-scale" => s.auto_scale = matches!(v, "on" | "true" | "1"),
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
                "warp.length" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.warp_length = f.clamp(LENGTH_MIN, LENGTH_MAX);
                        }
                    }
                }
                "control.look-sens" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.look_sensitivity = f.clamp(0.1, 5.0);
                        }
                    }
                }
                k if k.starts_with("stick.") => {
                    s.stick.parse_key(k, v);
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
                "cam.drive-shake" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.drive_shake = f.clamp(0.0, 2.0);
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
                "mimics.size" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.mimics_size = f.clamp(0.5, 3.0);
                        }
                    }
                }
                "miners.count" => {
                    if let Ok(n) = v.parse::<u32>() {
                        s.miners_count = n.min(crate::miner::MAX_MINERS as u32);
                    }
                }
                "miners.growth" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.miners_growth = f.clamp(0.25, 4.0);
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
                "holo.range" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.holo_range = f.clamp(HOLO_RANGE_MIN, HOLO_RANGE_MAX);
                        }
                    }
                }
                "ui.controls-card" => match v {
                    "on" => s.controls_card = true,
                    "off" => s.controls_card = false,
                    _ => {}
                },
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
                "graphics.bloom" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.bloom = f.clamp(0.0, 2.0);
                        }
                    }
                }
                "graphics.exposure" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.exposure = f.clamp(EXPOSURE_MIN, EXPOSURE_MAX);
                        }
                    }
                }
                "graphics.tonemap" => {
                    if let Some(t) = Tonemap::from_key(v) {
                        s.tonemap = t;
                    }
                }
                "graphics.fringe" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.fringe = f.clamp(0.0, 2.0);
                        }
                    }
                }
                "graphics.dust" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.dust = f.clamp(0.0, 2.0);
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
                "graphics.terrain-detail" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.terrain_detail = f.clamp(0.0, 2.0);
                        }
                    }
                }
                "graphics.clouds" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.clouds = f.clamp(0.0, 2.0);
                        }
                    }
                }
                "graphics.city-lights" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.city_lights = f.clamp(0.0, 2.0);
                        }
                    }
                }
                "graphics.nebula" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.nebula = f.clamp(0.0, 3.0);
                        }
                    }
                }
                "graphics.nebula-seed" => {
                    if let Ok(n) = v.parse::<u32>() {
                        s.nebula_seed = n % 100_000;
                    }
                }
                "graphics.nebula-scale" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.nebula_scale = f.clamp(1.0, 8.0);
                        }
                    }
                }
                "graphics.nebula-density" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.nebula_density = f.clamp(0.0, 1.0);
                        }
                    }
                }
                "graphics.nebula-clouds" => {
                    if let Ok(n) = v.parse::<u32>() {
                        s.nebula_clouds = n.clamp(1, 8);
                    }
                }
                "graphics.nebula-hue" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.nebula_hue = f.rem_euclid(1.0);
                        }
                    }
                }
                "graphics.nebula-hue2" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.nebula_hue2 = f.rem_euclid(1.0);
                        }
                    }
                }
                "graphics.nebula-spread" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.nebula_spread = f.clamp(0.25, 3.0);
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
                "cockpit.stick" => s.cockpit_stick = matches!(v, "on" | "true" | "1"),
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
                "landing.assist" => s.landing_assist = matches!(v, "on" | "true" | "1"),
                "landing.pad" => s.landing_pad = matches!(v, "on" | "true" | "1"),
                "map.rings" => {
                    if let Ok(n) = v.parse::<u32>() {
                        s.map_rings = n.min(crate::map::RINGS_MAX);
                    }
                }
                "world.helis" => s.helis = matches!(v, "on" | "true" | "1"),
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
                                "lean" => {
                                    if let Ok(f) = v.parse::<f32>() {
                                        if f.is_finite() {
                                            d.lean_deg = f.clamp(TILT_MIN, TILT_MAX);
                                        }
                                    }
                                }
                                "rotate" => {
                                    if let Ok(f) = v.parse::<f32>() {
                                        if f.is_finite() {
                                            d.rotate_deg = f.clamp(ROTATE_MIN, ROTATE_MAX);
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
        // A file from before `settings.version` carrying exactly the old
        // DRIVE SHAKE stock is an old default, not a choice: it adopts
        // the new stock. Any other value in an old file is kept.
        if !saw_version {
            if (s.drive_shake - OLD_DRIVE_SHAKE_STOCK).abs() < 1e-3 {
                s.drive_shake = Self::default().drive_shake;
            }
            s.version = SETTINGS_VERSION;
        }
        s
    }

    pub fn render(&self) -> String {
        let mut out = String::from("# FARFALL settings — edited by the in-game menu (Esc)\n");
        out.push_str(&format!("settings.version = {}\n", self.version));
        out.push_str(&format!("graphics.msaa = {}\n", self.msaa));
        out.push_str(&format!("graphics.scale = {:.2}\n", self.scale));
        out.push_str(&format!(
            "graphics.vsync = {}\n",
            if self.vsync { "on" } else { "off" }
        ));
        out.push_str(&format!(
            "graphics.auto-scale = {}\n",
            if self.auto_scale { "on" } else { "off" }
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
        self.stick.render(&mut out);
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
        out.push_str(&format!(
            "cockpit.stick = {}\n",
            if self.cockpit_stick { "on" } else { "off" }
        ));
        out.push_str(&format!("graphics.fps-floor = {:.0}\n", self.fps_floor));
        out.push_str(&format!("graphics.sky = {:.2}\n", self.sky));
        out.push_str(&format!(
            "graphics.terrain-detail = {:.2}\n",
            self.terrain_detail
        ));
        out.push_str(&format!("graphics.clouds = {:.2}\n", self.clouds));
        out.push_str(&format!("graphics.city-lights = {:.2}\n", self.city_lights));
        out.push_str(&format!("graphics.flare = {:.2}\n", self.flare));
        out.push_str(&format!("graphics.bloom = {:.2}\n", self.bloom));
        out.push_str(&format!("graphics.exposure = {:.3}\n", self.exposure));
        out.push_str(&format!("graphics.tonemap = {}\n", self.tonemap.key()));
        out.push_str(&format!("graphics.fringe = {:.2}\n", self.fringe));
        out.push_str(&format!("graphics.dust = {:.2}\n", self.dust));
        out.push_str(&format!("graphics.nebula = {:.2}\n", self.nebula));
        out.push_str(&format!("graphics.nebula-seed = {}\n", self.nebula_seed));
        out.push_str(&format!(
            "graphics.nebula-scale = {:.2}\n",
            self.nebula_scale
        ));
        out.push_str(&format!(
            "graphics.nebula-density = {:.2}\n",
            self.nebula_density
        ));
        out.push_str(&format!(
            "graphics.nebula-clouds = {}\n",
            self.nebula_clouds
        ));
        out.push_str(&format!("graphics.nebula-hue = {:.3}\n", self.nebula_hue));
        out.push_str(&format!("graphics.nebula-hue2 = {:.3}\n", self.nebula_hue2));
        out.push_str(&format!(
            "graphics.nebula-spread = {:.2}\n",
            self.nebula_spread
        ));
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
        out.push_str(&format!("mimics.size = {:.2}\n", self.mimics_size));
        out.push_str(&format!("miners.count = {}\n", self.miners_count));
        out.push_str(&format!("miners.growth = {:.2}\n", self.miners_growth));
        out.push_str(&format!("hold.gain = {:.2}\n", self.hold_gain));
        out.push_str(&format!(
            "hold.face = {}\n",
            if self.hold_face { "on" } else { "off" }
        ));
        out.push_str(&format!("arms.sight = {:.2}\n", self.arms_sight));
        out.push_str(&format!("cam.shake = {:.2}\n", self.cam_shake));
        out.push_str(&format!("cam.drive-shake = {:.2}\n", self.drive_shake));
        out.push_str(&format!(
            "camera.chase = {}\n",
            if self.camera_chase { "on" } else { "off" }
        ));
        out.push_str(&format!(
            "holo.view = {}\n",
            if self.holo_view { "on" } else { "off" }
        ));
        out.push_str(&format!("holo.size = {:.2}\n", self.holo_size));
        out.push_str(&format!("holo.range = {:.1}\n", self.holo_range));
        out.push_str(&format!(
            "ui.controls-card = {}\n",
            if self.controls_card { "on" } else { "off" }
        ));
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
        out.push_str(&format!(
            "landing.assist = {}\n",
            if self.landing_assist { "on" } else { "off" }
        ));
        out.push_str(&format!(
            "landing.pad = {}\n",
            if self.landing_pad { "on" } else { "off" }
        ));
        out.push_str(&format!("map.rings = {}\n", self.map_rings));
        out.push_str(&format!(
            "map.grid = {}\n",
            if self.map_grid { "on" } else { "off" }
        ));
        out.push_str(&format!(
            "world.helis = {}\n",
            if self.helis { "on" } else { "off" }
        ));

        out.push_str(&format!("warp.destination = {}\n", self.plan.dest.key()));
        out.push_str(&format!("warp.safe-radii = {:.3}\n", self.plan.safe_radii));
        out.push_str(&format!("warp.length = {:.2}\n", self.warp_length));
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
                out.push_str(&format!("ui.{}.lean = {:.0}\n", i.key(), d.lean_deg));
                out.push_str(&format!("ui.{}.rotate = {:.0}\n", i.key(), d.rotate_deg));
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
    fn nebula_keys_clamp_and_wrap() {
        let s = Settings::parse(
            "graphics.nebula = 9\ngraphics.nebula-seed = 123456\ngraphics.nebula-scale = 0\n\
             graphics.nebula-density = 2\ngraphics.nebula-clouds = 0\ngraphics.nebula-hue = 1.25\n\
             graphics.nebula-hue2 = -0.25\ngraphics.nebula-spread = 10\n",
        );
        assert_eq!(s.nebula, 3.0);
        assert_eq!(s.nebula_seed, 23456);
        assert_eq!(s.nebula_scale, 1.0);
        assert_eq!(s.nebula_density, 1.0);
        assert_eq!(s.nebula_clouds, 1);
        assert!((s.nebula_hue - 0.25).abs() < 1e-6);
        assert!((s.nebula_hue2 - 0.75).abs() < 1e-6);
        assert_eq!(s.nebula_spread, 3.0);
        assert_eq!(Settings::parse("").nebula, 1.0, "on by default");
    }

    #[test]
    fn the_ground_keys_clamp_and_default_to_stock() {
        let s = Settings::parse(
            "graphics.terrain-detail = 7\ngraphics.clouds = -2\ngraphics.city-lights = 1.5\n",
        );
        assert_eq!(s.terrain_detail, 2.0);
        assert_eq!(s.clouds, 0.0);
        assert_eq!(s.city_lights, 1.5);
        let d = Settings::parse("");
        assert_eq!((d.terrain_detail, d.clouds, d.city_lights), (1.0, 1.0, 1.0));
        assert_eq!(Settings::parse("graphics.clouds = nan").clouds, 1.0);
    }

    #[test]
    fn edits_round_trip() {
        let mut s = Settings {
            msaa: 2,
            scale: 0.75,
            auto_scale: true,
            helis: false,
            vsync: false,
            terrain_detail: 1.5,
            clouds: 0.5,
            city_lights: 2.0,
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
        s.mimics_size = 1.75;
        s.miners_count = 7;
        s.miners_growth = 2.5;
        s.hold_gain = 1.75;
        s.hold_face = false;
        s.stick.enabled = false;
        s.stick.deadzone = 0.14;
        s.stick.curve = 2.25;
        s.stick.throttle_zero = crate::stick::ThrottleZero::Bottom;
        s.stick.bind_axis(
            crate::stick::Flight::Lift,
            crate::stick::AxisMap::parse("3-").unwrap(),
        );
        s.stick.bind_fire(Some(2));
        s.arms_sight = 0.5;
        s.cam_shake = 1.75;
        s.drive_shake = 0.8;
        s.hoop_size = 2.5;
        s.map_rings = 2;
        s.landing_spacing_m = 500.0;
        s.landing_assist = false;
        s.landing_pad = false;
        s.cockpit_frame = false;
        s.cockpit_glow = 1.5;
        s.cockpit_hull = 0.25;
        s.cockpit_res = 1.0;
        s.cockpit_stick = false;
        s.fps_floor = 90.0;
        s.sky = 1.5;
        s.flare = 0.5;
        s.bloom = 1.5;
        s.exposure = 0.5;
        s.tonemap = Tonemap::Soft;
        s.fringe = 0.0;
        s.dust = 1.75;
        s.nebula = 2.0;
        s.nebula_seed = 42;
        s.nebula_scale = 5.0;
        s.nebula_density = 0.25;
        s.nebula_clouds = 6;
        s.nebula_hue = 0.125;
        s.nebula_hue2 = 0.375;
        s.nebula_spread = 2.0;
        s.fov = 85.0;
        s.gauge_style = GaugeStyle::Dial;
        s.gauges_stay = false;
        s.guide = true;
        s.dials[Instrument::Speed as usize] = DialTweak {
            size: 1.5,
            style: Some(GaugeStyle::Dial),
            stay: Some(true),
            tilt_deg: 45.0,
            lean_deg: -20.0,
            rotate_deg: 90.0,
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
        s.holo_range = 2.5;
        s.controls_card = true;
        s.holo_anchor = [-0.375, 0.25];
        s.plan.dest = Destination::Moon;
        s.plan.set_safe(3.5);
        s.warp_length = 1.5;
        assert_eq!(Settings::parse(&s.render()), s);
    }

    /// A new player's dash is the Warthog's: steam gauges on the metal.
    /// The GAUGES page still cycles the other three, and a file that
    /// names another style keeps it.
    #[test]
    fn warthog_is_the_default_and_the_menu_still_cycles_every_style() {
        let s = Settings::default();
        assert_eq!(s.gauge_style, GaugeStyle::Warthog);
        assert!(s.render().contains("ui.gauge-style = warthog\n"));
        let mut seen = vec![s.gauge_style];
        let mut cur = s.gauge_style;
        for _ in 0..GaugeStyle::ALL.len() - 1 {
            cur = cur.next(true);
            seen.push(cur);
        }
        seen.sort_by_key(|g| g.index());
        assert_eq!(seen, GaugeStyle::ALL.to_vec());
        assert_eq!(cur.next(true), GaugeStyle::Warthog, "and round");
        assert_eq!(
            Settings::parse("ui.gauge-style = tron\n").gauge_style,
            GaugeStyle::Tron
        );
        assert_eq!(Settings::parse("").gauge_style, GaugeStyle::Warthog);
    }

    /// The KEYS list is the ledger of the file: every line the file writes
    /// is on it (or is a dragged anchor), and every fixed name on it is a
    /// line the file writes. A key written but unlisted would slip past
    /// the menu's coverage test.
    #[test]
    fn the_key_list_is_the_file() {
        let mut s = Settings::default();
        s.dials[Instrument::Speed as usize].size = 1.5;
        let written: Vec<String> = s
            .render()
            .lines()
            .filter_map(|l| l.split_once('=').map(|(k, _)| k.trim().to_string()))
            .collect();
        for k in &written {
            assert!(
                KEYS.iter()
                    .chain(DRAGGED_KEYS)
                    .chain(FILE_ONLY_KEYS)
                    .any(|p| key_matches(p, k)),
                "{k} is written but not listed"
            );
        }
        for p in KEYS {
            if !p.contains('*') {
                assert!(
                    written.iter().any(|k| k == p),
                    "{p} is listed but never written"
                );
            }
        }
        assert!(key_matches("ui.*.size", "ui.speed.size"));
        assert!(!key_matches("ui.*", "ui.speed.size"));
        assert!(!key_matches("control.*", "ui.speed"));
    }

    #[test]
    fn the_hologram_range_and_the_card_are_kept() {
        let s = Settings::parse("holo.range = 9\nui.controls-card = on\n");
        assert_eq!(s.holo_range, HOLO_RANGE_MAX);
        assert!(s.controls_card);
        let d = Settings::default();
        assert!(d.holo_view, "the holo3PP is a stock gauge");
        assert!(
            !d.controls_card,
            "the card shows itself on the first run only"
        );
        assert_eq!(d.holo_range, 1.0);
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
            for _ in 0..GaugeStyle::ALL.len() {
                cur = next_dial_style(cur, i, true);
                seen.push(cur);
            }
            assert_eq!(
                seen,
                vec![
                    None,
                    Some(GaugeStyle::Tron),
                    Some(GaugeStyle::Jet),
                    Some(GaugeStyle::Dial),
                    Some(GaugeStyle::Warthog)
                ],
                "{i:?}"
            );
            assert_eq!(next_dial_style(cur, i, true), None, "and round");
            assert_eq!(next_dial_style(None, i, false), Some(GaugeStyle::Warthog));
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

    /// The other two orientation axes: a sideways lean held to the tilt's
    /// reach, a rotation to the half turn either way, garbage refused.
    #[test]
    fn a_lean_and_a_rotation_are_held_within_reach() {
        let s = Settings::parse(
            "ui.speed.lean = 95\nui.gyro.lean = -25\nui.speed.rotate = 260\n\
             ui.gyro.rotate = -45\nui.g-meter.rotate = nan\n",
        );
        assert_eq!(s.dials[Instrument::Speed as usize].lean_deg, TILT_MAX);
        assert_eq!(s.dials[Instrument::Gyro as usize].lean_deg, -25.0);
        assert_eq!(s.dials[Instrument::Speed as usize].rotate_deg, ROTATE_MAX);
        assert_eq!(s.dials[Instrument::Gyro as usize].rotate_deg, -45.0);
        assert_eq!(s.dials[Instrument::GForce as usize].rotate_deg, 0.0);
        assert!(s.render().contains("ui.gyro.lean = -25\n"));
        assert!(s.render().contains("ui.gyro.rotate = -45\n"));
    }

    /// A settings file from before `settings.version` that still carries
    /// the old DRIVE SHAKE stock (40%) adopts the new stock; an explicit
    /// choice — any other value — is kept, and a versioned file is
    /// believed as written. Saving stamps the current version.
    #[test]
    fn an_old_files_stock_drive_shake_adopts_the_new_stock() {
        let old_stock = Settings::parse("cam.drive-shake = 0.40\n");
        assert_eq!(old_stock.drive_shake, Settings::default().drive_shake);
        assert_eq!(old_stock.version, SETTINGS_VERSION);
        let old_choice = Settings::parse("cam.drive-shake = 0.80\n");
        assert_eq!(old_choice.drive_shake, 0.80);
        let versioned = Settings::parse("settings.version = 2\ncam.drive-shake = 0.40\n");
        assert_eq!(versioned.drive_shake, 0.40, "a versioned file is a choice");
        assert!(Settings::default()
            .render()
            .contains(&format!("settings.version = {SETTINGS_VERSION}\n")));
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

    /// The stock hologram floats where nothing else is drawn: its disc on
    /// the glass is outside every dial on the dash, under the mini map with
    /// room to spare, out of the forward view, and inside the glass — at
    /// the narrow screen (4:3) and the wide (16:9).
    #[test]
    fn the_stock_hologram_sits_clear_of_the_dials_and_the_mini_map() {
        use crate::map::{MINI_ANCHOR, MINI_HALF_H};
        use farfall_render::holo::{holo_centre, HOLO_RADIUS_M};
        let s = Settings::default();
        assert!(s.holo_view, "the hologram is a stock gauge");
        let tan_half = (s.fov.to_radians() * 0.5).tan();
        for aspect in [4.0 / 3.0, 16.0 / 9.0] {
            let radius_m = s.holo_size * HOLO_RADIUS_M;
            let c = holo_centre(s.holo_anchor, tan_half, aspect, radius_m);
            assert!(c.z < 0.0, "in front of the pilot: {c}");
            // Its disc on the glass, NDC.
            let cx = c.x / -c.z / (tan_half * aspect);
            let cy = c.y / -c.z / tan_half;
            let ry = radius_m / c.length() / tan_half;
            let rx = ry / aspect;
            assert!(
                cx + rx < 0.98 && cy + ry < 0.98,
                "inside the glass: {cx} {cy}"
            );
            // Under the mini map, with a gap.
            let map_bottom = s.layout.inset(MINI_ANCHOR)[1] - MINI_HALF_H;
            assert!(
                cy + ry < map_bottom - 0.05,
                "under the mini map: {} vs {map_bottom}",
                cy + ry
            );
            // Outside every dial on the dash: a stock dial is about 0.28 of
            // the half height tall on the glass.
            let dial_ry = 0.28;
            for i in Instrument::ALL {
                if !i.slotted() {
                    continue;
                }
                let Some(a) = s.layout.anchor(i) else {
                    continue;
                };
                let dx = (a[0] - cx).abs() - (dial_ry / aspect + rx);
                let dy = (a[1] - cy).abs() - (dial_ry + ry);
                assert!(
                    dx > 0.0 || dy > 0.0,
                    "{i:?} at {a:?} is under the hologram at {cx},{cy}"
                );
            }
            // Out of the forward view: the middle of the glass is for the world.
            assert!(cx - rx > 0.6, "right of the forward view: {}", cx - rx);
        }
    }
}
