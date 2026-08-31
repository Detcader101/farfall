//! The in-game menu — GFX / KEYS / CABIN / GAUGES — a flat panel on the
//! screen while the sim is paused.
//!
//! Rendered through the HUD text pass (a bit mask the GPU draws on the
//! canopy), driven by the keyboard, editing [`Settings`] in place. Esc
//! opens it and closes it; while it is open the sim is paused and the
//! flight keys are released, because a pilot reading a menu is not flying.
//!
//! Four pages, Tab between them. Up/Down moves, Left/Right changes a
//! value, Enter starts a key rebind (the next key pressed takes it; Esc
//! cancels). Every change is applied at once and written to the settings
//! file; there is no "save" — the file is the state.

use crate::bay::Hardpoint;
use crate::cockpit::Instrument;
use crate::input::{is_reserved, key_name, Action, Named};
use crate::settings::{
    Settings, COCKPIT_RES_CHOICES, FOV_MAX, FOV_MIN, FPS_FLOOR_CHOICES, HOOP_SIZE_MAX,
    HOOP_SIZE_MIN, LANDING_SPACINGS, MSAA_CHOICES,
};
use crate::settings::{BAY_SCANLINES_MAX, BAY_SIZE_MAX, BAY_SIZE_MIN};
use farfall_render::text::TextBitmap;
use winit::keyboard::KeyCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Graphics,
    Controls,
    Cockpit,
    Gauges,
    Arms,
    Map,
    /// The SHIP bay: the hologram's own panel (B), not a page.
    Ship,
}

impl Page {
    /// The settings menu's pages. The MAP is its own panel (M), not a page.
    const ALL: [Page; 5] = [
        Page::Graphics,
        Page::Controls,
        Page::Cockpit,
        Page::Gauges,
        Page::Arms,
    ];

    fn short(self) -> &'static str {
        match self {
            Page::Graphics => "GFX",
            Page::Controls => "KEYS",
            Page::Cockpit => "CABIN",
            Page::Gauges => "GAUGES",
            Page::Arms => "ARMS",
            Page::Map => "MAP",
            Page::Ship => "SHIP",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Page::Graphics => "GRAPHICS",
            Page::Controls => "CONTROLS",
            Page::Cockpit => "COCKPIT",
            Page::Gauges => "GAUGES",
            Page::Arms => "ARMS",
            Page::Map => "MAP",
            Page::Ship => "SHIP",
        }
    }
}

/// What a key press in the menu asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuEvent {
    Nothing,
    /// Settings changed: apply and save.
    Changed(Change),
    Closed,
    Quit,
    /// Close the menu and fire the wormhole drive at the plan.
    Engage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Graphics,
    Bindings,
    Layout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Msaa,
    Scale,
    AutoScale,
    Vsync,
    Quit,
    Bind(Action),
    BindNamed(Named),
    Slot(Instrument),
    HoopSize,
    LandingHoops,
    CockpitFrame,
    CockpitGlow,
    CockpitHull,
    CockpitRes,
    FpsFloor,
    Sky,
    Flare,
    Dust,
    /// The nebula block: glow, then which one and its shape and colours.
    Nebula,
    NebulaSeed,
    NebulaScale,
    NebulaDensity,
    NebulaClouds,
    NebulaHue,
    NebulaHue2,
    NebulaSpread,
    Fov,
    GaugeStyle,
    GaugesStay,
    Guide,
    HullSound,
    Shield,
    /// The GAUGES page's per-dial block: which dial, and its own numbers.
    DialSelect,
    DialSize,
    DialStyle,
    DialFade,
    DialTilt,
    Camera,
    HoloView,
    HoloSize,
    MapRings,
    MapGrid,
    LookSens,
    Destination,
    SafeDist,
    Engage,
    /// The ARMS page: the reactor's share, and the light.
    ArmsPower,
    ArmsGlow,
    ArmsShards,
    ArmsShardLife,
    ArmsScarSize,
    ArmsScarCool,
    ArmsOre,
    MimicsChance,
    MimicsHostility,
    HoldGain,
    HoldFace,
    ArmsSight,
    /// The camera on the head: sway, tremor, jolts.
    CamShake,
    DriveShake,
    /// The SHIP bay: what each hardpoint carries, and the hologram's look.
    Mount(Hardpoint),
    BayHue,
    BaySaturation,
    BayScanlines,
    BaySize,
    BaySpin,
    PointerSize,
}

impl Item {
    fn label(self) -> &'static str {
        match self {
            Item::Msaa => "MSAA",
            Item::Scale => "RENDER SCALE",
            Item::AutoScale => "AUTO SCALE",
            Item::Vsync => "VSYNC",
            Item::Quit => "QUIT GAME",
            Item::Bind(a) => a.name(),
            Item::BindNamed(n) => n.name(),
            Item::Slot(i) => i.name(),
            Item::HoopSize => "HOOP SIZE",
            Item::LandingHoops => "LANDING HOOPS",
            Item::CockpitFrame => "CABIN FRAME",
            Item::CockpitGlow => "CABIN GLOW",
            Item::CockpitHull => "CABIN METAL",
            Item::CockpitRes => "CABIN DETAIL",
            Item::FpsFloor => "FPS FLOOR",
            Item::Sky => "SKY",
            Item::Flare => "LENS FLARE",
            Item::Dust => "DUST",
            Item::Nebula => "NEBULA",
            Item::NebulaSeed => "NEBULA SEED",
            Item::NebulaScale => "NEBULA SCALE",
            Item::NebulaDensity => "NEBULA DENSITY",
            Item::NebulaClouds => "NEBULA CLOUDS",
            Item::NebulaHue => "NEBULA HUE",
            Item::NebulaHue2 => "NEBULA HUE 2",
            Item::NebulaSpread => "NEBULA SPREAD",
            Item::Camera => "CAMERA",
            Item::HoloView => "HOLO VIEW",
            Item::HoloSize => "HOLO SIZE",
            Item::Fov => "FOV",
            Item::GaugeStyle => "GAUGE STYLE",
            Item::GaugesStay => "GAUGES",
            Item::Guide => "GUIDE",
            Item::HullSound => "HULL SOUNDS",
            Item::Shield => "SHIELD",
            Item::ArmsPower => "REACTOR TO ARMS",
            Item::ArmsGlow => "MUZZLE LIGHT",
            Item::ArmsSight => "GUN SIGHT",
            Item::CamShake => "CAMERA SHAKE",
            Item::DriveShake => "DRIVE SHAKE",
            Item::ArmsShards => "DEBRIS",
            Item::ArmsShardLife => "DEBRIS LIFE",
            Item::ArmsScarSize => "SCARS",
            Item::ArmsScarCool => "SCAR COOLING",
            Item::ArmsOre => "ORE YIELD",
            Item::MimicsChance => "MIMICS",
            Item::MimicsHostility => "HOSTILITY",
            Item::HoldGain => "HOLD GAIN",
            Item::HoldFace => "HOLD FACING",
            Item::Mount(h) => h.name(),
            Item::BayHue => "HOLO HUE",
            Item::BaySaturation => "HOLO COLOUR",
            Item::BayScanlines => "SCANLINES",
            Item::BaySize => "HOLO SIZE",
            Item::BaySpin => "HOLO SPIN",
            Item::PointerSize => "POINTER SIZE",
            Item::DialSelect => "DIAL",
            Item::DialSize => "  SIZE",
            Item::DialStyle => "  STYLE",
            Item::DialFade => "  FADE",
            Item::DialTilt => "  TILT",
            Item::MapRings => "BODY RINGS",
            Item::MapGrid => "GRID",
            Item::LookSens => "LOOK SENS",
            Item::Destination => "DESTINATION",
            Item::SafeDist => "SAFE DISTANCE",
            Item::Engage => "ENGAGE DRIVE",
        }
    }

    fn value(self, s: &Settings) -> String {
        match self {
            Item::Msaa => format!("{}X", s.msaa),
            Item::Scale => format!("{:.0}%", s.scale * 100.0),
            Item::AutoScale => (if s.auto_scale { "ON" } else { "OFF" }).to_string(),
            Item::Vsync => (if s.vsync { "ON" } else { "OFF" }).to_string(),
            Item::Quit => String::new(),
            Item::Bind(a) => key_name(s.bindings.key_for(a)).to_string(),
            Item::BindNamed(n) => key_name(s.bindings.named(n)).to_string(),
            Item::Slot(i) => match s.layout.free(i) {
                Some(_) => "DRAGGED".to_string(),
                None => s.layout.get(i).name().to_string(),
            },
            Item::HoopSize => format!("{:.2}x", s.hoop_size),
            Item::LandingHoops => format!("{:.0}M", s.landing_spacing_m),
            Item::CockpitFrame => if s.cockpit_frame { "ON" } else { "OFF" }.to_string(),
            Item::CockpitGlow => format!("{:.2}x", s.cockpit_glow),
            Item::CockpitHull => format!("{:.0}%", s.cockpit_hull * 100.0),
            Item::CockpitRes => format!("{:.0}%", s.cockpit_res * 100.0),
            Item::Sky => format!("{:.0}%", s.sky * 100.0),
            Item::Nebula => {
                if s.nebula > 0.0 {
                    format!("{:.0}%", s.nebula * 100.0)
                } else {
                    "OFF".to_string()
                }
            }
            Item::NebulaSeed => format!("{}", s.nebula_seed),
            Item::NebulaScale => format!("{:.1}", s.nebula_scale),
            Item::NebulaDensity => format!("{:.0}%", s.nebula_density * 100.0),
            Item::NebulaClouds => format!("{}", s.nebula_clouds),
            Item::NebulaHue => format!("{:.0}", s.nebula_hue * 360.0),
            Item::NebulaHue2 => format!("{:.0}", s.nebula_hue2 * 360.0),
            Item::NebulaSpread => format!("{:.2}x", s.nebula_spread),
            Item::Flare => {
                if s.flare > 0.0 {
                    format!("{:.0}%", s.flare * 100.0)
                } else {
                    "OFF".to_string()
                }
            }
            Item::Dust => {
                if s.dust > 0.0 {
                    format!("{:.0}%", s.dust * 100.0)
                } else {
                    "OFF".to_string()
                }
            }
            Item::FpsFloor => {
                if s.fps_floor > 0.0 {
                    format!("{:.0}", s.fps_floor)
                } else {
                    "OFF".to_string()
                }
            }
            Item::Fov => format!("{:.0} DEG", s.fov),
            Item::Camera => if s.camera_chase { "CHASE" } else { "FIRST" }.to_string(),
            Item::HoloView => if s.holo_view { "ON" } else { "OFF" }.to_string(),
            Item::HoloSize => format!("{:.0}%", s.holo_size * 100.0),
            Item::GaugeStyle => s.gauge_style.name().to_string(),
            Item::GaugesStay => if s.gauges_stay { "STAY" } else { "FADE" }.to_string(),
            Item::Guide => if s.guide { "ON" } else { "OFF" }.to_string(),
            Item::HullSound => if s.hull_sound { "ON" } else { "OFF" }.to_string(),
            Item::Shield => {
                if s.shield > 0.0 {
                    format!("{:.0}%", s.shield * 100.0)
                } else {
                    "OFF".to_string()
                }
            }
            Item::DialSelect => String::new(),
            Item::DialSize => String::new(),
            Item::DialStyle => String::new(),
            Item::DialFade => String::new(),
            Item::DialTilt => String::new(),
            Item::MapRings => s.map_rings.to_string(),
            Item::MapGrid => if s.map_grid { "ON" } else { "OFF" }.to_string(),
            Item::LookSens => format!("{:.2}", s.look_sensitivity),
            Item::Destination => s.plan.dest.name().to_string(),
            // Uranus is arrived at in its belt whatever the distance says.
            Item::SafeDist => {
                if s.plan.dest == crate::warp::Destination::Uranus {
                    "IN THE BELT".to_string()
                } else {
                    format!("{:.2} R", s.plan.safe_radii)
                }
            }
            Item::Engage => String::new(),
            Item::ArmsPower => format!("{:.0}%", s.arms_power * 100.0),
            Item::ArmsGlow => {
                if s.arms_glow > 0.0 {
                    format!("{:.0}%", s.arms_glow * 100.0)
                } else {
                    "OFF".to_string()
                }
            }
            Item::ArmsShards => {
                if s.arms_shards > 0 {
                    s.arms_shards.to_string()
                } else {
                    "OFF".to_string()
                }
            }
            Item::ArmsShardLife => format!("{:.0} S", s.arms_shard_life),
            Item::ArmsScarSize => {
                if s.arms_scar_size > 0.0 {
                    format!("{:.0}%", s.arms_scar_size * 100.0)
                } else {
                    "OFF".to_string()
                }
            }
            Item::ArmsScarCool => format!("{:.0} S", s.arms_scar_cool),
            Item::ArmsOre => {
                if s.arms_ore > 0.0 {
                    format!("{:.0}%", s.arms_ore * 100.0)
                } else {
                    "OFF".to_string()
                }
            }
            Item::MimicsChance => {
                if s.mimics_chance > 0.0 {
                    format!("{:.0}%", s.mimics_chance * 100.0)
                } else {
                    "NONE".to_string()
                }
            }
            Item::MimicsHostility => format!("{:.0}%", s.mimics_hostility * 100.0),
            Item::HoldGain => format!("{:.0}%", s.hold_gain * 100.0),
            Item::HoldFace => if s.hold_face { "ON" } else { "OFF" }.to_string(),
            Item::ArmsSight => {
                if s.arms_sight > 0.0 {
                    format!("{:.0}%", s.arms_sight * 100.0)
                } else {
                    "OFF".to_string()
                }
            }
            Item::CamShake => {
                if s.cam_shake > 0.0 {
                    format!("{:.0}%", s.cam_shake * 100.0)
                } else {
                    "OFF".to_string()
                }
            }
            Item::DriveShake => {
                if s.drive_shake > 0.0 {
                    format!("{:.0}%", s.drive_shake * 100.0)
                } else {
                    "OFF".to_string()
                }
            }
            Item::Mount(h) => s.mounts[h as usize].name().to_string(),
            Item::BayHue => format!("{:.0}", s.bay_hue * 360.0),
            Item::BaySaturation => format!("{:.0}%", s.bay_saturation * 100.0),
            Item::BayScanlines => format!("{:.0}", s.bay_scanlines),
            Item::BaySize => format!("{:.0}%", s.bay_size * 100.0),
            Item::BaySpin => if s.bay_spin { "ON" } else { "OFF" }.to_string(),
            Item::PointerSize => format!("{:.0}%", s.pointer_size / 0.045 * 100.0),
        }
    }

    fn rebindable(self) -> bool {
        matches!(self, Item::Bind(_) | Item::BindNamed(_))
    }
}

/// The path and its hoops belong with the cabin, not the dials.
fn path_item(i: Instrument) -> bool {
    matches!(
        i,
        Instrument::Trajectory | Instrument::Hoops | Instrument::HoopSound
    )
}

/// Rows of text the bitmap can hold (64 px / 6 px pitch), minus the
/// header and the footer.
const VISIBLE_ITEMS: usize = 8;
const ROW_PX: usize = 6;
/// Characters per row (128 px / 4 px advance).
const COLS: usize = 32;

#[derive(Debug, Clone, Copy)]
pub struct Menu {
    pub open: bool,
    /// A one-page panel (the DRIVE panel beside the map): no paging, its
    /// own header.
    standalone: bool,
    page: Page,
    cursor: usize,
    scroll: usize,
    /// Waiting for a key to bind to the item under the cursor.
    rebinding: bool,
    /// Which of MSAA_CHOICES this GPU can render at (set at start).
    msaa_ok: [bool; 4],
    /// The dial the GAUGES page's per-dial block edits.
    dial: Instrument,
}

impl Default for Menu {
    fn default() -> Self {
        Self {
            open: false,
            standalone: false,
            page: Page::Graphics,
            cursor: 0,
            scroll: 0,
            rebinding: false,
            msaa_ok: [true; 4],
            dial: Instrument::Speed,
        }
    }
}

/// Step a clamped float knob; `Nothing` at the end of its range.
fn step_f32(v: &mut f32, forward: bool, step: f32, lo: f32, hi: f32) -> MenuEvent {
    let next = (*v + if forward { step } else { -step }).clamp(lo, hi);
    if (next - *v).abs() < 1e-6 {
        return MenuEvent::Nothing;
    }
    *v = next;
    MenuEvent::Changed(Change::Layout)
}

impl Menu {
    pub fn new() -> Self {
        Self::default()
    }

    fn items(&self) -> Vec<Item> {
        match self.page {
            Page::Graphics => vec![
                Item::Msaa,
                Item::Scale,
                Item::AutoScale,
                Item::Vsync,
                Item::Fov,
                Item::Camera,
                Item::HoloView,
                Item::HoloSize,
                Item::BayHue,
                Item::BaySaturation,
                Item::BayScanlines,
                Item::BaySize,
                Item::BaySpin,
                Item::PointerSize,
                Item::CockpitRes,
                Item::FpsFloor,
                Item::Sky,
                Item::Flare,
                Item::Dust,
                Item::Nebula,
                Item::NebulaSeed,
                Item::NebulaScale,
                Item::NebulaDensity,
                Item::NebulaClouds,
                Item::NebulaHue,
                Item::NebulaHue2,
                Item::NebulaSpread,
                Item::Quit,
            ],
            Page::Controls => {
                let mut v: Vec<Item> = Action::ALL.iter().map(|&a| Item::Bind(a)).collect();
                for n in Named::ALL {
                    v.push(Item::BindNamed(n));
                }
                v.push(Item::LookSens);
                v
            }
            // The cabin, in groups: the ship itself (frame, glow, metal,
            // shield, its sounds), then everything about the path and its
            // hoops together. No safe edge: the glass has no margin.
            Page::Cockpit => vec![
                Item::CockpitFrame,
                Item::CockpitGlow,
                Item::CockpitHull,
                Item::Shield,
                Item::HullSound,
                Item::Slot(Instrument::Trajectory),
                Item::Slot(Instrument::Hoops),
                Item::Slot(Instrument::HoopSound),
                Item::HoopSize,
                Item::LandingHoops,
                Item::CamShake,
                Item::DriveShake,
            ],
            // The gauges: the cockpit-wide look, then one dial's own
            // numbers, then where each instrument sits (or OFF) — the
            // dials and the glass's own elements; the path lives with the
            // cabin.
            Page::Gauges => {
                let mut v: Vec<Item> = vec![
                    Item::GaugeStyle,
                    Item::GaugesStay,
                    Item::Guide,
                    Item::DialSelect,
                    Item::DialSize,
                    Item::DialStyle,
                    Item::DialFade,
                    Item::DialTilt,
                ];
                v.extend(
                    Instrument::ALL
                        .iter()
                        .filter(|i| !path_item(**i))
                        .map(|&i| Item::Slot(i)),
                );
                v
            }
            Page::Arms => vec![
                Item::ArmsPower,
                Item::ArmsGlow,
                Item::ArmsSight,
                Item::ArmsShards,
                Item::ArmsShardLife,
                Item::ArmsScarSize,
                Item::ArmsScarCool,
                Item::ArmsOre,
                Item::MimicsChance,
                Item::MimicsHostility,
                Item::HoldGain,
                Item::HoldFace,
            ],
            Page::Map => vec![
                Item::Destination,
                Item::SafeDist,
                Item::Engage,
                Item::MapRings,
                Item::MapGrid,
            ],
            Page::Ship => Hardpoint::ALL.iter().map(|&h| Item::Mount(h)).collect(),
        }
    }

    /// Restrict the MSAA choices to what the GPU supports.
    pub fn set_msaa_supported(&mut self, supported: &[u32]) {
        for (i, n) in MSAA_CHOICES.iter().enumerate() {
            self.msaa_ok[i] = supported.contains(n);
        }
    }

    /// The MAP page is showing: draw the system map under the text.
    pub fn map_open(&self) -> bool {
        self.open && self.page == Page::Map
    }

    /// The DRIVE panel: the map's own controls, a one-page menu of its own
    /// that opens with the map (M).
    pub fn map_panel() -> Self {
        Self {
            standalone: true,
            page: Page::Map,
            ..Self::default()
        }
    }

    /// The SHIP bay's panel: the fit and the trim, beside the hologram.
    pub fn ship_panel() -> Self {
        Self {
            standalone: true,
            page: Page::Ship,
            ..Self::default()
        }
    }

    /// The hardpoint the cursor is on, for the hologram to light.
    pub fn bay_selected(&self) -> Option<usize> {
        match self.items().get(self.cursor) {
            Some(Item::Mount(h)) => Some(*h as usize),
            _ => None,
        }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.rebinding = false;
    }

    fn set_page(&mut self, page: Page) {
        self.page = page;
        self.cursor = 0;
        self.scroll = 0;
        self.rebinding = false;
    }

    /// A key press while the menu is open.
    /// A click on the panel's row `row` (0 is the header): the cursor
    /// goes there and the item is adjusted forward — the pointer's way
    /// through a menu.
    pub fn click(&mut self, row: usize, settings: &mut Settings) -> MenuEvent {
        let items = self.items();
        if row == 0 || row > VISIBLE_ITEMS {
            return MenuEvent::Nothing;
        }
        let idx = self.scroll + row - 1;
        if idx >= items.len() {
            return MenuEvent::Nothing;
        }
        self.cursor = idx;
        self.key(KeyCode::Enter, settings)
    }

    /// Put the cursor on this item, if the page has it.
    pub fn set_cursor(&mut self, idx: usize) {
        if idx < self.items().len() {
            self.cursor = idx;
            self.keep_cursor_visible();
        }
    }

    pub fn key(&mut self, key: KeyCode, settings: &mut Settings) -> MenuEvent {
        let items = self.items();
        let item = items[self.cursor.min(items.len() - 1)];

        if self.rebinding {
            if key == KeyCode::Escape {
                self.rebinding = false;
                return MenuEvent::Nothing;
            }
            if is_reserved(key) {
                return MenuEvent::Nothing;
            }
            let bound = match item {
                Item::Bind(a) => settings.bindings.bind(a, key),
                Item::BindNamed(n) => settings.bindings.bind_named(n, key),
                _ => false,
            };
            self.rebinding = false;
            return if bound {
                MenuEvent::Changed(Change::Bindings)
            } else {
                MenuEvent::Nothing
            };
        }

        match key {
            KeyCode::Escape => {
                self.open = false;
                MenuEvent::Closed
            }
            KeyCode::Tab if !self.standalone => {
                let i = Page::ALL.iter().position(|&p| p == self.page).unwrap_or(0);
                self.set_page(Page::ALL[(i + 1) % Page::ALL.len()]);
                MenuEvent::Nothing
            }
            KeyCode::ArrowUp => {
                self.cursor = (self.cursor + items.len() - 1) % items.len();
                self.keep_cursor_visible();
                MenuEvent::Nothing
            }
            KeyCode::ArrowDown => {
                self.cursor = (self.cursor + 1) % items.len();
                self.keep_cursor_visible();
                MenuEvent::Nothing
            }
            KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                let forward = key == KeyCode::ArrowRight;
                self.adjust(item, forward, settings)
            }
            KeyCode::Enter | KeyCode::Space => match item {
                Item::Quit => MenuEvent::Quit,
                Item::Engage => {
                    self.open = false;
                    MenuEvent::Engage
                }
                i if i.rebindable() => {
                    self.rebinding = true;
                    MenuEvent::Nothing
                }
                i => self.adjust(i, true, settings),
            },
            _ => MenuEvent::Nothing,
        }
    }

    fn adjust(&mut self, item: Item, forward: bool, s: &mut Settings) -> MenuEvent {
        match item {
            Item::Msaa => {
                let n = MSAA_CHOICES.len();
                let mut i = MSAA_CHOICES.iter().position(|&m| m == s.msaa).unwrap_or(2);
                // Step to the next count this GPU can do; none other: stay.
                for _ in 0..n {
                    i = if forward {
                        (i + 1) % n
                    } else {
                        (i + n - 1) % n
                    };
                    if self.msaa_ok[i] {
                        break;
                    }
                }
                if !self.msaa_ok[i] || MSAA_CHOICES[i] == s.msaa {
                    return MenuEvent::Nothing;
                }
                s.msaa = MSAA_CHOICES[i];
                MenuEvent::Changed(Change::Graphics)
            }
            Item::Scale => {
                let step = if forward { 0.05 } else { -0.05 };
                s.scale = ((s.scale + step) * 100.0).round() / 100.0;
                s.scale = s.scale.clamp(0.25, 1.0);
                MenuEvent::Changed(Change::Graphics)
            }
            Item::AutoScale => {
                s.auto_scale = !s.auto_scale;
                MenuEvent::Changed(Change::Graphics)
            }
            Item::Vsync => {
                s.vsync = !s.vsync;
                MenuEvent::Changed(Change::Graphics)
            }
            Item::Slot(i) => {
                s.layout.cycle(i, forward);
                MenuEvent::Changed(Change::Layout)
            }
            Item::LookSens => {
                let step = if forward { 0.25 } else { -0.25 };
                let next = (s.look_sensitivity + step).clamp(0.25, 5.0);
                if (next - s.look_sensitivity).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.look_sensitivity = next;
                MenuEvent::Changed(Change::Bindings)
            }
            Item::Destination => {
                s.plan.cycle_destination(forward);
                MenuEvent::Changed(Change::Layout)
            }
            Item::SafeDist => {
                let before = s.plan.safe_radii;
                s.plan.adjust_safe(forward);
                if (s.plan.safe_radii - before).abs() < 1e-9 {
                    return MenuEvent::Nothing;
                }
                MenuEvent::Changed(Change::Layout)
            }
            Item::Engage => MenuEvent::Nothing,
            Item::LandingHoops => {
                let n = LANDING_SPACINGS.len();
                let i = LANDING_SPACINGS
                    .iter()
                    .position(|&x| x == s.landing_spacing_m)
                    .unwrap_or(1);
                let j = if forward {
                    (i + 1) % n
                } else {
                    (i + n - 1) % n
                };
                s.landing_spacing_m = LANDING_SPACINGS[j];
                MenuEvent::Changed(Change::Layout)
            }
            Item::CockpitFrame => {
                s.cockpit_frame = !s.cockpit_frame;
                MenuEvent::Changed(Change::Layout)
            }
            Item::CockpitGlow => {
                let step = if forward { 0.25 } else { -0.25 };
                let next = (s.cockpit_glow + step).clamp(0.25, 2.0);
                if (next - s.cockpit_glow).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.cockpit_glow = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::CockpitHull => {
                let step = if forward { 0.1 } else { -0.1 };
                let next = (s.cockpit_hull + step).clamp(0.0, 1.0);
                if (next - s.cockpit_hull).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.cockpit_hull = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::Fov => {
                let step = if forward { 5.0 } else { -5.0 };
                let next = (s.fov + step).clamp(FOV_MIN, FOV_MAX);
                if (next - s.fov).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.fov = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::GaugeStyle => {
                s.gauge_style = s.gauge_style.next(forward);
                MenuEvent::Changed(Change::Layout)
            }
            Item::GaugesStay => {
                s.gauges_stay = !s.gauges_stay;
                MenuEvent::Changed(Change::Layout)
            }
            Item::Guide => {
                s.guide = !s.guide;
                MenuEvent::Changed(Change::Layout)
            }
            Item::HullSound => {
                s.hull_sound = !s.hull_sound;
                MenuEvent::Changed(Change::Layout)
            }
            Item::ArmsPower => {
                let step = if forward { 0.1 } else { -0.1 };
                let next = (s.arms_power + step).clamp(0.0, 1.0);
                if (next - s.arms_power).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.arms_power = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::ArmsGlow => {
                let step = if forward { 0.25 } else { -0.25 };
                let next = (s.arms_glow + step).clamp(0.0, 2.0);
                if (next - s.arms_glow).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.arms_glow = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::ArmsShards => {
                let next = if forward {
                    (s.arms_shards + 8).min(crate::settings::ARMS_SHARDS_MAX)
                } else {
                    s.arms_shards.saturating_sub(8)
                };
                if next == s.arms_shards {
                    return MenuEvent::Nothing;
                }
                s.arms_shards = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::ArmsShardLife => {
                let step = if forward { 1.0 } else { -1.0 };
                let next = (s.arms_shard_life + step).clamp(1.0, 12.0);
                if (next - s.arms_shard_life).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.arms_shard_life = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::CamShake => {
                let step = if forward { 0.25 } else { -0.25 };
                let next = (s.cam_shake + step).clamp(0.0, 2.0);
                if (next - s.cam_shake).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.cam_shake = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::DriveShake => {
                let step = if forward { 0.25 } else { -0.25 };
                let next = (s.drive_shake + step).clamp(0.0, 2.0);
                if (next - s.drive_shake).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.drive_shake = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::ArmsSight => {
                let step = if forward { 0.25 } else { -0.25 };
                let next = (s.arms_sight + step).clamp(0.0, 2.0);
                if (next - s.arms_sight).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.arms_sight = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::ArmsScarSize => {
                let step = if forward { 0.25 } else { -0.25 };
                let next = (s.arms_scar_size + step).clamp(0.0, 2.0);
                if (next - s.arms_scar_size).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.arms_scar_size = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::ArmsScarCool => {
                let step = if forward { 4.0 } else { -4.0 };
                let next = (s.arms_scar_cool + step).clamp(2.0, 60.0);
                if (next - s.arms_scar_cool).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.arms_scar_cool = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::ArmsOre => {
                let step = if forward { 0.25 } else { -0.25 };
                let next = (s.arms_ore + step).clamp(0.0, 2.0);
                if (next - s.arms_ore).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.arms_ore = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::MimicsChance => {
                let step = if forward { 0.05 } else { -0.05 };
                let next = (s.mimics_chance + step).clamp(0.0, 1.0);
                if (next - s.mimics_chance).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.mimics_chance = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::MimicsHostility => {
                let step = if forward { 0.1 } else { -0.1 };
                let next = (s.mimics_hostility + step).clamp(0.0, 1.0);
                if (next - s.mimics_hostility).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.mimics_hostility = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::HoldGain => {
                let step = if forward { 0.25 } else { -0.25 };
                let next = (s.hold_gain + step).clamp(0.2, 3.0);
                if (next - s.hold_gain).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.hold_gain = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::HoldFace => {
                s.hold_face = !s.hold_face;
                MenuEvent::Changed(Change::Layout)
            }
            Item::Mount(h) => {
                let i = h as usize;
                s.mounts[i] = s.mounts[i].next(forward);
                MenuEvent::Changed(Change::Layout)
            }
            Item::BayHue => {
                s.bay_hue =
                    (s.bay_hue + if forward { 1.0 / 24.0 } else { -1.0 / 24.0 }).rem_euclid(1.0);
                MenuEvent::Changed(Change::Layout)
            }
            Item::BaySaturation => {
                let before = s.bay_saturation;
                s.bay_saturation = (before + if forward { 0.1 } else { -0.1 }).clamp(0.0, 1.0);
                if (s.bay_saturation - before).abs() < 1e-6 {
                    MenuEvent::Nothing
                } else {
                    MenuEvent::Changed(Change::Layout)
                }
            }
            Item::BayScanlines => {
                let before = s.bay_scanlines;
                s.bay_scanlines =
                    (before + if forward { 20.0 } else { -20.0 }).clamp(0.0, BAY_SCANLINES_MAX);
                if (s.bay_scanlines - before).abs() < 1e-6 {
                    MenuEvent::Nothing
                } else {
                    MenuEvent::Changed(Change::Layout)
                }
            }
            Item::BaySize => {
                let before = s.bay_size;
                s.bay_size =
                    (before + if forward { 0.02 } else { -0.02 }).clamp(BAY_SIZE_MIN, BAY_SIZE_MAX);
                if (s.bay_size - before).abs() < 1e-6 {
                    MenuEvent::Nothing
                } else {
                    MenuEvent::Changed(Change::Layout)
                }
            }
            Item::BaySpin => {
                s.bay_spin = !s.bay_spin;
                MenuEvent::Changed(Change::Layout)
            }
            Item::PointerSize => {
                let before = s.pointer_size;
                s.pointer_size = (before + if forward { 0.005 } else { -0.005 }).clamp(0.02, 0.1);
                if (s.pointer_size - before).abs() < 1e-6 {
                    MenuEvent::Nothing
                } else {
                    MenuEvent::Changed(Change::Layout)
                }
            }
            Item::Shield => {
                let step = if forward { 0.25 } else { -0.25 };
                let next = (s.shield + step).clamp(0.0, 2.0);
                if (next - s.shield).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.shield = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::DialSelect => {
                let dials: Vec<Instrument> = Instrument::ALL
                    .iter()
                    .copied()
                    .filter(|i| i.slotted())
                    .collect();
                let i = dials.iter().position(|&d| d == self.dial).unwrap_or(0);
                let n = dials.len();
                self.dial = dials[if forward {
                    (i + 1) % n
                } else {
                    (i + n - 1) % n
                }];
                MenuEvent::Nothing
            }
            Item::DialSize => {
                let d = &mut s.dials[self.dial as usize];
                let step = if forward { 0.125 } else { -0.125 };
                let next = (d.size + step).clamp(
                    crate::settings::DIAL_SIZE_MIN,
                    crate::settings::DIAL_SIZE_MAX,
                );
                if (next - d.size).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                d.size = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::DialStyle => {
                let d = &mut s.dials[self.dial as usize];
                d.style = crate::settings::next_dial_style(d.style, self.dial, forward);
                MenuEvent::Changed(Change::Layout)
            }
            Item::DialTilt => {
                let d = &mut s.dials[self.dial as usize];
                let next = (d.tilt_deg + if forward { 5.0 } else { -5.0 })
                    .clamp(crate::settings::TILT_MIN, crate::settings::TILT_MAX);
                if (next - d.tilt_deg).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                d.tilt_deg = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::DialFade => {
                let d = &mut s.dials[self.dial as usize];
                d.stay = match (d.stay, forward) {
                    (None, true) | (Some(false), false) => Some(true),
                    (Some(true), true) | (None, false) => Some(false),
                    (Some(false), true) | (Some(true), false) => None,
                };
                MenuEvent::Changed(Change::Layout)
            }
            Item::CockpitRes => {
                let n = COCKPIT_RES_CHOICES.len();
                let i = COCKPIT_RES_CHOICES
                    .iter()
                    .position(|&x| (x - s.cockpit_res).abs() < 1e-6)
                    .unwrap_or(0);
                let j = if forward {
                    (i + 1) % n
                } else {
                    (i + n - 1) % n
                };
                s.cockpit_res = COCKPIT_RES_CHOICES[j];
                MenuEvent::Changed(Change::Graphics)
            }
            Item::Flare => {
                let step = if forward { 0.25 } else { -0.25 };
                let next = (s.flare + step).clamp(0.0, 2.0);
                if (next - s.flare).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.flare = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::Dust => {
                let step = if forward { 0.25 } else { -0.25 };
                let next = (s.dust + step).clamp(0.0, 2.0);
                if (next - s.dust).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.dust = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::Camera => {
                s.camera_chase = !s.camera_chase;
                MenuEvent::Changed(Change::Layout)
            }
            Item::HoloView => {
                s.holo_view = !s.holo_view;
                MenuEvent::Changed(Change::Layout)
            }
            Item::HoloSize => {
                let step = if forward { 0.04 } else { -0.04 };
                let next = (s.holo_size + step).clamp(
                    crate::settings::HOLO_SIZE_MIN,
                    crate::settings::HOLO_SIZE_MAX,
                );
                if (next - s.holo_size).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.holo_size = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::Sky => {
                let step = if forward { 0.25 } else { -0.25 };
                let next = (s.sky + step).clamp(0.0, 2.0);
                if (next - s.sky).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.sky = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::Nebula => step_f32(&mut s.nebula, forward, 0.25, 0.0, 3.0),
            Item::NebulaSeed => {
                // Wraps: there is always another nebula.
                s.nebula_seed = if forward {
                    (s.nebula_seed + 1) % 100_000
                } else {
                    (s.nebula_seed + 100_000 - 1) % 100_000
                };
                MenuEvent::Changed(Change::Layout)
            }
            Item::NebulaScale => step_f32(&mut s.nebula_scale, forward, 0.5, 1.0, 8.0),
            Item::NebulaDensity => step_f32(&mut s.nebula_density, forward, 0.05, 0.0, 1.0),
            Item::NebulaClouds => {
                let next = if forward {
                    (s.nebula_clouds + 1).min(8)
                } else {
                    (s.nebula_clouds - 1).max(1)
                };
                if next == s.nebula_clouds {
                    return MenuEvent::Nothing;
                }
                s.nebula_clouds = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::NebulaHue => {
                s.nebula_hue =
                    (s.nebula_hue + if forward { 1.0 / 24.0 } else { -1.0 / 24.0 }).rem_euclid(1.0);
                MenuEvent::Changed(Change::Layout)
            }
            Item::NebulaHue2 => {
                s.nebula_hue2 = (s.nebula_hue2 + if forward { 1.0 / 24.0 } else { -1.0 / 24.0 })
                    .rem_euclid(1.0);
                MenuEvent::Changed(Change::Layout)
            }
            Item::NebulaSpread => step_f32(&mut s.nebula_spread, forward, 0.25, 0.25, 3.0),
            Item::FpsFloor => {
                let n = FPS_FLOOR_CHOICES.len();
                let i = FPS_FLOOR_CHOICES
                    .iter()
                    .position(|&x| (x - s.fps_floor).abs() < 1e-6)
                    .unwrap_or(2);
                let j = if forward {
                    (i + 1) % n
                } else {
                    (i + n - 1) % n
                };
                s.fps_floor = FPS_FLOOR_CHOICES[j];
                MenuEvent::Changed(Change::Graphics)
            }
            Item::MapRings => {
                let n = crate::map::RINGS_MAX + 1;
                s.map_rings = if forward {
                    (s.map_rings + 1) % n
                } else {
                    (s.map_rings + n - 1) % n
                };
                MenuEvent::Changed(Change::Layout)
            }
            Item::MapGrid => {
                s.map_grid = !s.map_grid;
                MenuEvent::Changed(Change::Layout)
            }
            Item::HoopSize => {
                let step = if forward { 0.25 } else { -0.25 };
                let next = (s.hoop_size + step).clamp(HOOP_SIZE_MIN, HOOP_SIZE_MAX);
                if (next - s.hoop_size).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.hoop_size = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::Quit | Item::Bind(_) | Item::BindNamed(_) => MenuEvent::Nothing,
        }
    }

    fn keep_cursor_visible(&mut self) {
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + VISIBLE_ITEMS {
            self.scroll = self.cursor + 1 - VISIBLE_ITEMS;
        }
    }

    /// An item's shown value; the per-dial block reads the selected dial.
    fn value_of(&self, item: Item, s: &Settings) -> String {
        let d = s.dials[self.dial as usize];
        match item {
            Item::DialSelect => self.dial.name().to_string(),
            Item::DialSize => format!("{:.2}X", d.size),
            Item::DialStyle => d
                .style
                .map_or("AUTO".to_string(), |st| st.name().to_string()),
            Item::DialFade => match d.stay {
                None => "AUTO",
                Some(true) => "STAY",
                Some(false) => "FADE",
            }
            .to_string(),
            Item::DialTilt => format!("{:+.0} DEG", d.tilt_deg),
            other => other.value(s),
        }
    }

    /// The header row: the pages, the current one bracketed — or, for a
    /// panel of its own, just its name.
    fn header(&self) -> String {
        let mut header = String::new();
        if self.standalone {
            let title = match self.page {
                Page::Ship => "SHIP BAY",
                _ => "WORMHOLE DRIVE",
            };
            header.push_str(&format!("[{}]  {title}", self.page.name()));
        } else {
            // Short names, so four pages fit the row.
            for p in Page::ALL {
                if p == self.page {
                    header.push_str(&format!("[{}]", p.short()));
                } else {
                    header.push_str(&format!(" {} ", p.short()));
                }
            }
        }
        header
    }

    /// One item's row: the cursor mark, the label, the value right-aligned
    /// — always exactly COLS wide.
    fn line(&self, item: Item, selected: bool, s: &Settings) -> String {
        let value = if selected && self.rebinding {
            "PRESS KEY".to_string()
        } else {
            self.value_of(item, s)
        };
        let mark = if selected { ">" } else { " " };
        let label = item.label();
        let pad = COLS.saturating_sub(1 + label.len() + value.len()).max(1);
        format!("{mark}{label}{}{value}", " ".repeat(pad))
    }

    fn footer(&self) -> &'static str {
        if self.rebinding {
            "ESC CANCEL"
        } else {
            match self.page {
                Page::Controls => "TAB PAGE  ENTER BIND  ESC BACK",
                Page::Map => "< > SET  ENTER ENGAGE  M CLOSE",
                Page::Ship => "CLICK A SLOT  DRAG TURN  B CLOSE",
                _ => "TAB PAGE  < > ADJUST  ESC BACK",
            }
        }
    }

    /// The chosen row's top and height in font pixels, for the card's band.
    pub fn cursor_row_px(&self) -> (f32, f32) {
        let row = self.cursor.saturating_sub(self.scroll) + 1;
        ((row * ROW_PX) as f32, ROW_PX as f32)
    }

    /// Draw the menu into the text bitmap.
    pub fn render(&self, text: &mut TextBitmap, s: &Settings) {
        text.clear();
        text.draw(0, 0, &self.header());

        let items = self.items();
        let end = (self.scroll + VISIBLE_ITEMS).min(items.len());
        for (row, idx) in (self.scroll..end).enumerate() {
            let y = (row + 1) * ROW_PX;
            text.draw(0, y, &self.line(items[idx], idx == self.cursor, s));
        }
        // Scroll marks.
        if self.scroll > 0 {
            text.draw(124, ROW_PX, "^");
        }
        if end < items.len() {
            text.draw(124, VISIBLE_ITEMS * ROW_PX, "V");
        }
        text.draw(0, (VISIBLE_ITEMS + 1) * ROW_PX, self.footer());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cockpit::Slot;

    #[test]
    fn escape_closes_and_tab_pages() {
        let mut m = Menu::new();
        let mut s = Settings::default();
        m.toggle();
        assert!(m.open);
        m.key(KeyCode::Tab, &mut s);
        assert_eq!(m.page, Page::Controls);
        assert_eq!(m.key(KeyCode::Escape, &mut s), MenuEvent::Closed);
        assert!(!m.open);
    }

    #[test]
    fn msaa_only_offers_what_the_gpu_supports() {
        let mut m = Menu::new();
        let mut s = Settings::default();
        m.set_msaa_supported(&[1, 4]);
        m.toggle();
        // From 4, forward wraps past 8 (unsupported) to 1.
        assert_eq!(
            m.key(KeyCode::ArrowRight, &mut s),
            MenuEvent::Changed(Change::Graphics)
        );
        assert_eq!(s.msaa, 1);
        assert_eq!(
            m.key(KeyCode::ArrowRight, &mut s),
            MenuEvent::Changed(Change::Graphics)
        );
        assert_eq!(s.msaa, 4);
        // Only one choice: nothing to change.
        m.set_msaa_supported(&[4]);
        assert_eq!(m.key(KeyCode::ArrowRight, &mut s), MenuEvent::Nothing);
    }

    #[test]
    fn right_arrow_adjusts_graphics() {
        let mut m = Menu::new();
        let mut s = Settings::default();
        m.toggle();
        assert_eq!(
            m.key(KeyCode::ArrowRight, &mut s),
            MenuEvent::Changed(Change::Graphics)
        );
        assert_eq!(s.msaa, 8);
        m.key(KeyCode::ArrowDown, &mut s);
        m.key(KeyCode::ArrowLeft, &mut s);
        assert!((s.scale - 0.95).abs() < 1e-6);
    }

    #[test]
    fn rebinding_takes_the_next_key_and_refuses_reserved_ones() {
        let mut m = Menu::new();
        let mut s = Settings::default();
        m.toggle();
        m.key(KeyCode::Tab, &mut s); // controls, cursor on thrust-forward
        assert_eq!(m.key(KeyCode::Enter, &mut s), MenuEvent::Nothing);
        assert!(m.rebinding);
        assert_eq!(m.key(KeyCode::Tab, &mut s), MenuEvent::Nothing); // reserved: ignored
        assert!(m.rebinding);
        assert_eq!(
            m.key(KeyCode::KeyI, &mut s),
            MenuEvent::Changed(Change::Bindings)
        );
        assert_eq!(s.bindings.key_for(Action::ThrustForward), KeyCode::KeyI);
        assert!(!m.rebinding);
    }

    #[test]
    fn cockpit_page_cycles_slots_and_quit_quits() {
        let mut m = Menu::new();
        let mut s = Settings::default();
        m.toggle();
        m.key(KeyCode::Tab, &mut s);
        m.key(KeyCode::Tab, &mut s);
        m.key(KeyCode::Tab, &mut s); // gauges: style, stay, guide, the dial block, the slots
        assert_eq!(m.page, Page::Gauges);
        let items = m.items();
        let at = |it: Item| items.iter().position(|&x| x == it).unwrap();
        // The per-dial block comes before the long list of slots.
        assert!(at(Item::DialSelect) < at(Item::Slot(Instrument::Speed)));
        for _ in 0..at(Item::DialSelect) {
            m.key(KeyCode::ArrowDown, &mut s);
        }
        m.key(KeyCode::ArrowRight, &mut s); // DIAL: speed -> altitude
        assert_eq!(m.dial, Instrument::Altitude);
        m.key(KeyCode::ArrowDown, &mut s);
        m.key(KeyCode::ArrowRight, &mut s); // SIZE
        assert_eq!(s.dials[Instrument::Altitude as usize].size, 1.125);
        for _ in 0..3 {
            m.key(KeyCode::ArrowDown, &mut s);
        }
        assert_eq!(items[m.cursor], Item::DialTilt);
        assert_eq!(
            m.key(KeyCode::ArrowLeft, &mut s),
            MenuEvent::Changed(Change::Layout)
        );
        assert_eq!(s.dials[Instrument::Altitude as usize].tilt_deg, -5.0);
        assert_eq!(m.value_of(Item::DialTilt, &s), "-5 DEG");
        for _ in 0..30 {
            m.key(KeyCode::ArrowRight, &mut s);
        }
        assert_eq!(
            s.dials[Instrument::Altitude as usize].tilt_deg,
            crate::settings::TILT_MAX,
            "tilt stops at the limit"
        );
        assert_eq!(m.key(KeyCode::ArrowRight, &mut s), MenuEvent::Nothing);
        for _ in 0..(at(Item::Slot(Instrument::Speed)) - at(Item::DialTilt)) {
            m.key(KeyCode::ArrowDown, &mut s);
        }
        assert_eq!(
            m.key(KeyCode::ArrowRight, &mut s),
            MenuEvent::Changed(Change::Layout)
        );
        assert_ne!(s.layout.get(Instrument::Speed), Slot::BottomRight);
        m.key(KeyCode::Tab, &mut s); // on to ARMS
        m.key(KeyCode::Tab, &mut s); // and back round to graphics
        for _ in 0..m.items().len() - 1 {
            m.key(KeyCode::ArrowDown, &mut s);
        }
        assert_eq!(m.key(KeyCode::Enter, &mut s), MenuEvent::Quit);
    }

    /// Settings at their longest values, to try the rows' width.
    fn widest_settings() -> Settings {
        let mut s = Settings {
            scale: 1.0,
            fov: FOV_MAX,
            landing_spacing_m: *LANDING_SPACINGS.last().unwrap(),
            hoop_size: HOOP_SIZE_MAX,
            cockpit_glow: 10.0,
            look_sensitivity: 10.0,
            ..Default::default()
        };
        s.layout.set_free(Instrument::Speed, [0.0, 0.0]);
        for d in s.dials.iter_mut() {
            d.size = crate::settings::DIAL_SIZE_MAX;
            d.tilt_deg = crate::settings::TILT_MIN;
            d.style = Some(crate::settings::GaugeStyle::Dial);
            d.stay = Some(false);
        }
        s
    }

    #[test]
    fn every_row_of_every_page_fits_the_panel() {
        for s in [Settings::default(), widest_settings()] {
            for mut m in [Menu::new(), Menu::map_panel(), Menu::ship_panel()] {
                m.toggle();
                for _ in 0..Page::ALL.len() {
                    assert!(
                        m.header().len() <= COLS,
                        "{} page header is too wide: {:?}",
                        m.page.name(),
                        m.header()
                    );
                    assert!(m.footer().len() <= COLS, "{:?}", m.footer());
                    for item in m.items() {
                        for selected in [false, true] {
                            let line = m.line(item, selected, &s);
                            assert_eq!(
                                line.len(),
                                COLS,
                                "{} page, {:?}: {line:?}",
                                m.page.name(),
                                item
                            );
                            // Label and value never run into each other.
                            assert!(line[1 + item.label().len()..].starts_with(' '), "{line:?}");
                        }
                    }
                    m.key(KeyCode::Tab, &mut s.clone());
                }
            }
        }
    }

    #[test]
    fn the_nebula_knobs_step_clamp_and_wrap() {
        let mut m = Menu::new();
        let mut s = Settings::default();
        // Glow steps down to OFF and stops there.
        for _ in 0..4 {
            m.adjust(Item::Nebula, false, &mut s);
        }
        assert_eq!(s.nebula, 0.0);
        assert_eq!(Item::Nebula.value(&s), "OFF");
        assert_eq!(m.adjust(Item::Nebula, false, &mut s), MenuEvent::Nothing);
        // Seed wraps both ways; hue wraps around the wheel.
        s.nebula_seed = 0;
        m.adjust(Item::NebulaSeed, false, &mut s);
        assert_eq!(s.nebula_seed, 99_999);
        m.adjust(Item::NebulaSeed, true, &mut s);
        assert_eq!(s.nebula_seed, 0);
        s.nebula_hue = 0.0;
        m.adjust(Item::NebulaHue, false, &mut s);
        assert!(s.nebula_hue > 0.9);
        // Clouds hold at 1..8.
        s.nebula_clouds = 8;
        assert_eq!(
            m.adjust(Item::NebulaClouds, true, &mut s),
            MenuEvent::Nothing
        );
        s.nebula_clouds = 1;
        assert_eq!(
            m.adjust(Item::NebulaClouds, false, &mut s),
            MenuEvent::Nothing
        );
    }

    #[test]
    fn the_pages_are_categories_that_make_sense() {
        let mut m = Menu::new();
        m.toggle();
        let mut seen: Vec<Item> = Vec::new();
        for page in Page::ALL {
            m.set_page(page);
            let items = m.items();
            assert!(!items.is_empty());
            for it in &items {
                assert!(!seen.contains(it), "{it:?} is on two pages");
            }
            seen.extend(items.iter().copied());
            let on = |it: Item| items.contains(&it);
            match page {
                Page::Graphics => {
                    assert!(on(Item::Msaa) && on(Item::Fov) && on(Item::CockpitRes));
                    assert!(on(Item::FpsFloor));
                    // The nebula block sits together, after the sky knobs.
                    let at = |it: Item| items.iter().position(|i| *i == it).unwrap();
                    assert!(at(Item::Nebula) > at(Item::Flare));
                    assert_eq!(at(Item::NebulaSeed), at(Item::Nebula) + 1);
                    assert_eq!(at(Item::NebulaSpread), at(Item::Nebula) + 7);
                    assert_eq!(*items.last().unwrap(), Item::Quit, "QUIT is the last thing");
                }
                Page::Controls => {
                    assert!(items.iter().all(|i| i.rebindable() || *i == Item::LookSens));
                    // EVERY bind the game answers to is on this page: all
                    // twelve axis actions and all sixteen named controls.
                    // A key that works in game but is missing here is a bug.
                    for a in Action::ALL {
                        assert!(on(Item::Bind(a)), "missing axis bind {a:?}");
                    }
                    for n in Named::ALL {
                        assert!(on(Item::BindNamed(n)), "missing named bind {n:?}");
                    }
                }
                Page::Cockpit => {
                    assert!(on(Item::CockpitFrame) && on(Item::HoopSize));
                    assert!(!on(Item::GaugeStyle) && !on(Item::Guide));
                    // The hoop settings sit together, in one run.
                    let at = |it: Item| items.iter().position(|&x| x == it).unwrap();
                    let hoops = [
                        at(Item::Slot(Instrument::Trajectory)),
                        at(Item::Slot(Instrument::Hoops)),
                        at(Item::Slot(Instrument::HoopSound)),
                        at(Item::HoopSize),
                        at(Item::LandingHoops),
                    ];
                    for w in hoops.windows(2) {
                        assert_eq!(w[1], w[0] + 1, "hoop settings together: {hoops:?}");
                    }
                    assert_eq!(at(Item::HullSound), at(Item::Shield) + 1);
                    assert!(on(Item::CamShake));
                }
                Page::Gauges => {
                    assert!(on(Item::GaugeStyle) && on(Item::GaugesStay) && on(Item::Guide));
                    assert!(on(Item::DialTilt) && on(Item::Slot(Instrument::Gyro)));
                    assert!(
                        !on(Item::Slot(Instrument::Hoops)),
                        "hoops live with the cabin"
                    );
                }
                Page::Arms => {
                    assert!(on(Item::ArmsPower) && on(Item::ArmsGlow));
                    assert!(on(Item::ArmsShards) && on(Item::ArmsShardLife));
                    assert!(on(Item::ArmsScarSize) && on(Item::ArmsScarCool));
                    assert!(
                        on(Item::ArmsOre) && on(Item::MimicsChance) && on(Item::MimicsHostility)
                    );
                    assert!(on(Item::HoldGain) && on(Item::HoldFace));
                    assert!(on(Item::ArmsSight));
                    assert!(items.len() >= 2);
                }
                Page::Map | Page::Ship => unreachable!(),
            }
        }
        // The map's own page has the drive and its look, nothing else.
        m.set_page(Page::Map);
        assert!(m.items().iter().all(|i| matches!(
            i,
            Item::Destination | Item::SafeDist | Item::Engage | Item::MapRings | Item::MapGrid
        )));
    }

    #[test]
    fn the_ship_panel_fits_the_hardpoints_and_never_pages() {
        let mut s = Settings::default();
        let mut m = Menu::ship_panel();
        m.toggle();
        assert!(m.open && m.page == Page::Ship);
        assert_eq!(m.items().len(), Hardpoint::ALL.len());
        assert_eq!(m.bay_selected(), Some(0), "the nose first");
        // Tab is nothing here: the bay is one panel.
        m.key(KeyCode::Tab, &mut s);
        assert_eq!(m.page, Page::Ship);
        // Round the nose's mounts: rail -> empty -> cannon.
        let before = s.mounts[0];
        assert_eq!(
            m.key(KeyCode::ArrowRight, &mut s),
            MenuEvent::Changed(Change::Layout)
        );
        assert_ne!(s.mounts[0], before);
        m.key(KeyCode::ArrowLeft, &mut s);
        assert_eq!(s.mounts[0], before);
        // Down to the belly: the hologram's chosen pip follows.
        m.key(KeyCode::ArrowDown, &mut s);
        m.key(KeyCode::ArrowDown, &mut s);
        m.key(KeyCode::ArrowDown, &mut s);
        assert_eq!(m.bay_selected(), Some(3));
        // A click on a row is the cursor there and a step forward; the
        // header and rows past the list are nothing.
        assert_eq!(m.click(0, &mut s), MenuEvent::Nothing);
        assert_eq!(m.click(9, &mut s), MenuEvent::Nothing);
        let before = s.mounts[1];
        assert_eq!(m.click(2, &mut s), MenuEvent::Changed(Change::Layout));
        assert_eq!(m.bay_selected(), Some(1));
        assert_ne!(s.mounts[1], before);
        assert!(m.header().contains("SHIP BAY"));
        assert_eq!(m.key(KeyCode::Escape, &mut s), MenuEvent::Closed);
        // The look rows live on the GFX page and light no pip: hue wraps,
        // the rest clamp, spin flips, the pointer sizes.
        let mut m = Menu::new();
        m.page = Page::Graphics;
        let at = m.items().iter().position(|&i| i == Item::BayHue).unwrap();
        m.set_cursor(at);
        assert_eq!(m.bay_selected(), None);
        let hue = s.bay_hue;
        for _ in 0..24 {
            m.key(KeyCode::ArrowRight, &mut s);
        }
        assert!(
            (s.bay_hue - hue).abs() < 1e-4,
            "hue wraps round: {}",
            s.bay_hue
        );
        m.key(KeyCode::ArrowDown, &mut s);
        assert_eq!(
            m.key(KeyCode::ArrowRight, &mut s),
            MenuEvent::Nothing,
            "saturation is full"
        );
        m.key(KeyCode::ArrowLeft, &mut s);
        assert!((s.bay_saturation - 0.9).abs() < 1e-6);
        m.key(KeyCode::ArrowDown, &mut s);
        m.key(KeyCode::ArrowLeft, &mut s);
        assert_eq!(s.bay_scanlines, 100.0);
        m.key(KeyCode::ArrowDown, &mut s);
        for _ in 0..40 {
            m.key(KeyCode::ArrowRight, &mut s);
        }
        assert_eq!(s.bay_size, BAY_SIZE_MAX);
        m.key(KeyCode::ArrowDown, &mut s);
        m.key(KeyCode::ArrowRight, &mut s);
        assert!(!s.bay_spin);
        m.key(KeyCode::ArrowDown, &mut s);
        for _ in 0..40 {
            m.key(KeyCode::ArrowRight, &mut s);
        }
        assert_eq!(s.pointer_size, 0.1);
    }

    #[test]
    fn the_drive_panel_sets_the_plan_and_engages_and_never_pages() {
        // The settings menu has no MAP page any more: Tab cycles the three.
        let mut settings_menu = Menu::new();
        let mut s = Settings::default();
        settings_menu.toggle();
        for _ in 0..Page::ALL.len() {
            settings_menu.key(KeyCode::Tab, &mut s);
            assert!(!settings_menu.map_open());
        }
        let mut m = Menu::map_panel();
        m.toggle();
        assert!(m.map_open());
        m.key(KeyCode::Tab, &mut s);
        assert!(m.map_open(), "the drive panel has nowhere to page to");
        let d0 = s.plan.dest;
        assert_eq!(
            m.key(KeyCode::ArrowRight, &mut s),
            MenuEvent::Changed(Change::Layout)
        );
        assert_ne!(s.plan.dest, d0);
        m.key(KeyCode::ArrowDown, &mut s);
        let r0 = s.plan.safe_radii;
        m.key(KeyCode::ArrowRight, &mut s);
        assert!(s.plan.safe_radii > r0);
        m.key(KeyCode::ArrowDown, &mut s);
        assert_eq!(m.key(KeyCode::Enter, &mut s), MenuEvent::Engage);
        assert!(!m.open);
    }

    #[test]
    fn renders_within_the_bitmap() {
        let mut m = Menu::new();
        let s = Settings::default();
        m.toggle();
        m.key(KeyCode::Tab, &mut Settings::default());
        let mut t = TextBitmap::new();
        m.render(&mut t, &s);
        let (w, h) = t.used_extent();
        assert!(w <= 128 && h <= 64, "{w}x{h}");
        assert!(w > 60);
    }
}
