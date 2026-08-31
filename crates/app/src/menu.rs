//! The in-game menu — GFX / KEYS / CABIN / DIALS / ARMS / MAP / SHIP /
//! HELP — a flat card on the screen while the sim is paused.
//!
//! Rendered through the HUD text pass (a bit mask the GPU draws on the
//! canopy), driven by the keyboard, editing [`Settings`] in place. Esc
//! opens it and closes it; while it is open the sim is paused and the
//! flight keys are released, because a pilot reading a menu is not flying.
//!
//! Eight pages, Tab between them. Up/Down moves, Left/Right changes a
//! value, Enter starts a key rebind (the next key pressed takes it; Esc
//! cancels). Every change is applied at once and written to the settings
//! file; there is no "save" — the file is the state.
//!
//! The card is laid out here in font pixels, under test: every tab fits
//! the header, every row is exactly the card's width with its full value,
//! the list scrolls with a bar and a `ROW n / m` count, and the chosen
//! row's one-line description sits in the footer — a stranger can read
//! it at 800x600 and at 2880x1800, because the card is the same size in
//! canopy units on both.

use crate::bay::Hardpoint;
use crate::cockpit::Instrument;
use crate::input::{is_reserved, key_name, Action, Named};
use crate::settings::{
    Settings, COCKPIT_RES_CHOICES, FOV_MAX, FOV_MIN, FPS_FLOOR_CHOICES, HOOP_SIZE_MAX,
    HOOP_SIZE_MIN, LANDING_SPACINGS, MSAA_CHOICES,
};
use crate::settings::{BAY_SCANLINES_MAX, BAY_SIZE_MAX, BAY_SIZE_MIN, EXPOSURE_MAX, EXPOSURE_MIN};
use farfall_render::hud::Scrollbar;
use farfall_render::text::{
    block_height, block_width, wrap, TextBitmap, LINE, MENU_COLS, PANEL_COLS,
};
use winit::keyboard::KeyCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Graphics,
    Controls,
    Cockpit,
    Gauges,
    Arms,
    /// The wormhole drive's plan and the map's look. Also the DRIVE
    /// panel beside the map (M), standalone.
    Map,
    /// The ship's fit and the bay hologram's look. Also the SHIP bay's
    /// own card (B), standalone, with the fit alone.
    Ship,
    /// Every control by group, with what it does.
    Help,
}

impl Page {
    /// The settings menu's pages, in tab order.
    pub const ALL: [Page; 8] = [
        Page::Graphics,
        Page::Controls,
        Page::Cockpit,
        Page::Gauges,
        Page::Arms,
        Page::Map,
        Page::Ship,
        Page::Help,
    ];

    fn short(self) -> &'static str {
        match self {
            Page::Graphics => "GFX",
            Page::Controls => "KEYS",
            Page::Cockpit => "CABIN",
            Page::Gauges => "DIALS",
            Page::Arms => "ARMS",
            Page::Map => "MAP",
            Page::Ship => "SHIP",
            Page::Help => "HELP",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Page::Graphics => "GRAPHICS",
            Page::Controls => "CONTROLS",
            Page::Cockpit => "COCKPIT",
            Page::Gauges => "DIALS",
            Page::Arms => "ARMS",
            Page::Map => "MAP",
            Page::Ship => "SHIP",
            Page::Help => "HELP",
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

/// A control as the HELP page lists it: an axis, a named control, or one
/// of the few fixed keys the menu keeps for itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Axis(Action),
    Named(Named),
    /// (key, name) of a control that cannot be rebound.
    Fixed(&'static str, &'static str),
}

/// One line of the HELP page: the control, a short gloss for the row and
/// a sentence for the footer.
#[derive(Debug, Clone, Copy)]
pub struct HelpEntry {
    pub control: Control,
    pub gloss: &'static str,
    pub what: &'static str,
}

/// A group of controls on the HELP page.
#[derive(Debug, Clone, Copy)]
pub struct HelpGroup {
    pub name: &'static str,
    pub blurb: &'static str,
    pub entries: &'static [HelpEntry],
}

const fn axis(a: Action, gloss: &'static str, what: &'static str) -> HelpEntry {
    HelpEntry {
        control: Control::Axis(a),
        gloss,
        what,
    }
}

const fn named(n: Named, gloss: &'static str, what: &'static str) -> HelpEntry {
    HelpEntry {
        control: Control::Named(n),
        gloss,
        what,
    }
}

const fn fixed(
    key: &'static str,
    name: &'static str,
    gloss: &'static str,
    what: &'static str,
) -> HelpEntry {
    HelpEntry {
        control: Control::Fixed(key, name),
        gloss,
        what,
    }
}

/// Every control the game answers to, by group. The tests hold this to
/// the truth: every axis and every named bind is here exactly once.
pub const HELP: &[HelpGroup] = &[
    HelpGroup {
        name: "FLIGHT",
        blurb: "FLYING THE SHIP: THRUST, ATTITUDE, THE FLIGHT COMPUTER.",
        entries: &[
            axis(
                Action::ThrustForward,
                "MAIN ENGINES AHEAD",
                "MAIN ENGINES: ACCELERATE ALONG THE NOSE. HOLD BOOST WITH IT FOR A FULL BURN.",
            ),
            axis(
                Action::ThrustBack,
                "THRUST ASTERN",
                "THRUST ASTERN: SLOW DOWN, OR BACK AWAY FROM SOMETHING.",
            ),
            axis(
                Action::StrafeLeft,
                "SLIDE LEFT",
                "SIDE THRUSTERS: SLIDE LEFT WITHOUT TURNING THE NOSE.",
            ),
            axis(
                Action::StrafeRight,
                "SLIDE RIGHT",
                "SIDE THRUSTERS: SLIDE RIGHT WITHOUT TURNING THE NOSE.",
            ),
            axis(
                Action::ThrustUp,
                "RISE",
                "BELLY THRUSTERS: RISE STRAIGHT UP, THE WAY THE CANOPY POINTS.",
            ),
            axis(
                Action::ThrustDown,
                "SINK",
                "BACK THRUSTERS: PUSH STRAIGHT DOWN, THE WAY THE BELLY POINTS.",
            ),
            axis(Action::PitchUp, "NOSE UP", "PITCH: RAISE THE NOSE."),
            axis(Action::PitchDown, "NOSE DOWN", "PITCH: LOWER THE NOSE."),
            axis(Action::YawLeft, "NOSE LEFT", "YAW: SWING THE NOSE LEFT."),
            axis(Action::YawRight, "NOSE RIGHT", "YAW: SWING THE NOSE RIGHT."),
            axis(
                Action::RollLeft,
                "LEFT WING DOWN",
                "ROLL: BANK LEFT ABOUT THE NOSE.",
            ),
            axis(
                Action::RollRight,
                "RIGHT WING DOWN",
                "ROLL: BANK RIGHT ABOUT THE NOSE.",
            ),
            named(
                Named::Boost,
                "FULL BURN (HOLD)",
                "HOLD WITH A THRUST KEY FOR THE ENGINES' FULL POWER. HOT, LOUD, AND FAST.",
            ),
            named(
                Named::Brake,
                "AIR BRAKE (HOLD)",
                "HOLD TO OPEN THE AIR BRAKE AND SHED SPEED IN AN ATMOSPHERE.",
            ),
            named(
                Named::Despin,
                "KILL ANY SPIN",
                "THE EMERGENCY GYRO: KILLS EVERY ROTATION ON A FIXED TIME, WHATEVER THE SHIP IS DOING.",
            ),
            named(
                Named::Assist,
                "FLIGHT COMPUTER",
                "FLIGHT ASSIST ON OR OFF: THE COMPUTER DAMPS SPIN AND HOLDS ATTITUDE WITH THE KEYS UP.",
            ),
            named(
                Named::Hold,
                "HOLD A TARGET",
                "LOCK ON THE TARGET UNDER THE SIGHT AND HOLD THE RANGE; HOLD FACING KEEPS THE NOSE ON.",
            ),
        ],
    },
    HelpGroup {
        name: "DRIVES",
        blurb: "THE DRIVES THAT CROSS THE SYSTEM, AND THE MAP THEY ARE PLANNED ON.",
        entries: &[
            named(
                Named::Hyper,
                "CHAOS DRIVE (HOLD)",
                "HOLD TO CHARGE THE CHAOS DRIVE: SPEED CLIMBS WITH THE ENTROPY; THE SLIP THROWS YOU.",
            ),
            named(
                Named::WarpStop,
                "DROP OUT OF WARP",
                "CUT THE DRIVE AT ONCE, LEAVING AN AFTER-IMAGE OF THE SHIP BEHIND.",
            ),
            named(
                Named::Engage,
                "FIRE WORMHOLE DRIVE",
                "ENGAGE THE WORMHOLE DRIVE AT THE PLAN ON THE MAP PAGE: DESTINATION AND SAFE DISTANCE.",
            ),
            named(
                Named::Map,
                "SYSTEM MAP",
                "OPEN THE 3D SYSTEM MAP WITH ITS DRIVE PANEL. DRAG TO TURN IT, WHEEL OR + - TO ZOOM.",
            ),
        ],
    },
    HelpGroup {
        name: "VIEW",
        blurb: "LOOKING ROUND THE CABIN AND OUT OF IT.",
        entries: &[
            fixed(
                "RMB",
                "FREELOOK",
                "LOOK AROUND (HOLD)",
                "HOLD THE RIGHT MOUSE BUTTON AND MOVE THE MOUSE TO LOOK ROUND THE CABIN.",
            ),
            named(
                Named::LookLock,
                "LOCK THE FREELOOK",
                "KEEP THE FREELOOK ON WITHOUT HOLDING THE BUTTON. CLICK WHILE LOOKING TO DRAG A PANEL.",
            ),
            named(
                Named::Chase,
                "CHASE CAMERA",
                "SWAP THE VIEW FOR A CAMERA OUTSIDE THE SHIP, AND BACK.",
            ),
            named(
                Named::Holo,
                "SHIP HOLOGRAM",
                "THE HOLO3PP: A HOLOGRAM OF THE SHIP AND ITS SURROUNDINGS ON THE DASH. MARKS: OTHER SHIPS.",
            ),
            named(
                Named::HoloOut,
                "HOLOGRAM WIDER",
                "THE HOLOGRAM SHOWS MORE SPACE ROUND THE SHIP (THE SHIP SHRINKS). THE WHEEL DOES IT TOO.",
            ),
            named(
                Named::HoloIn,
                "HOLOGRAM CLOSER",
                "THE HOLOGRAM SHOWS LESS OF THE SPACE ROUND THE SHIP (THE SHIP GROWS).",
            ),
            named(
                Named::Appearance,
                "ATMOSPHERE LOOK",
                "CYCLE THE PLANET'S ATMOSPHERE THROUGH ITS PRESETS: DENSITY, CLOUD COVER, CLOUD DECK.",
            ),
            named(
                Named::Design,
                "LAY OUT THE DASH",
                "DESIGN MODE: THE MOUSE DRAGS DIALS; A CARD SHOWS THE DIAL UNDER IT AND ITS OWN KEYS.",
            ),
            named(
                Named::Capture,
                "SCREENSHOT",
                "SAVE A SCREENSHOT OF THE FRAME TO THE TEMP FOLDER. F12 DOES THE SAME.",
            ),
            named(
                Named::ScaleDown,
                "RENDER SCALE DOWN",
                "DRAW THE WORLD A STEP SMALLER FOR SPEED; THE GLASS AND THE TEXT STAY SHARP.",
            ),
            named(
                Named::ScaleUp,
                "RENDER SCALE UP",
                "DRAW THE WORLD A STEP LARGER, UP TO FULL SIZE.",
            ),
        ],
    },
    HelpGroup {
        name: "ARMS",
        blurb: "THE GUNS, THE ROCKS, AND THE SHIPS HIDING IN THEM.",
        entries: &[
            fixed(
                "LMB",
                "FIRE",
                "FIRE THE GUN",
                "FIRE THE SELECTED WEAPON AT THE SIGHT. THE CANNON FIRES WHILE HELD; THE RAIL CHARGES.",
            ),
            named(
                Named::Weapon1,
                "SELECT THE CANNON",
                "SELECT THE CANNON: FAST SLUGS FROM THE WINGS, HOT WHEN HELD.",
            ),
            named(
                Named::Weapon2,
                "SELECT THE RAIL",
                "SELECT THE RAIL: ONE HEAVY SLUG FROM THE NOSE, CHARGED BEFORE IT GOES.",
            ),
            named(
                Named::NextWeapon,
                "NEXT WEAPON",
                "CYCLE TO THE NEXT WEAPON THE SHIP IS FITTED WITH.",
            ),
        ],
    },
    HelpGroup {
        name: "PANELS",
        blurb: "THE SHIP'S OWN SCREENS AND THE MENU.",
        entries: &[
            fixed(
                "ESC",
                "MENU",
                "SETTINGS MENU",
                "OPEN OR CLOSE THIS MENU. EVERYTHING IN IT IS SAVED AS YOU CHANGE IT.",
            ),
            fixed(
                "TAB",
                "NEXT PAGE",
                "NEXT MENU PAGE",
                "STEP TO THE NEXT PAGE OF THE MENU.",
            ),
            fixed(
                "F1",
                "CONTROLS CARD",
                "THE CONTROLS CARD",
                "SHOW THE CONTROLS CARD AGAIN: THE ESSENTIAL KEYS ON ONE SCREEN.",
            ),
            named(
                Named::Bay,
                "SHIP BAY",
                "OPEN THE BAY: A HOLOGRAM OF THE SHIP WHERE EACH HARDPOINT'S MOUNT IS FITTED.",
            ),
            named(
                Named::Landing,
                "LANDING MODE",
                "THE HOOPS CLOSE UP ALONG THE PATH AND THE READOUT JUDGES THE TOUCHDOWN.",
            ),
            named(
                Named::Trajectory,
                "PATH ON / OFF",
                "SHOW OR HIDE THE PREDICTED PATH ON THE GLASS.",
            ),
            fixed(
                "+ -",
                "PANE ZOOM",
                "ZOOM A PANE",
                "ZOOM THE MAP OR THE BAY FROM THE KEYBOARD, FOR A MOUSE WITH NO WHEEL.",
            ),
        ],
    },
];

/// The HELP page's entries, flat, in page order.
fn help_entries() -> Vec<&'static HelpEntry> {
    HELP.iter().flat_map(|g| g.entries.iter()).collect()
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
    /// The picture: the post pass's bloom, exposure, curve and glass rim.
    Bloom,
    Exposure,
    TonemapCurve,
    Fringe,
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
    /// The DIALS page's per-dial block: which dial, and its own numbers.
    DialSelect,
    DialSize,
    DialStyle,
    DialFade,
    DialTilt,
    Camera,
    HoloView,
    HoloSize,
    HoloRange,
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
    MimicsSize,
    MinersCount,
    MinersGrowth,
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
    SafeEdge,
    /// The HELP page: a group heading, a control, the card setting.
    Heading(usize),
    Help(usize),
    ControlsCard,
}

impl Item {
    fn label(self) -> String {
        match self {
            Item::Bind(a) => a.name().to_string(),
            Item::BindNamed(n) => n.name().to_string(),
            Item::Slot(i) => i.name().to_string(),
            Item::Mount(h) => h.name().to_string(),
            Item::Heading(g) => HELP[g].name.to_string(),
            Item::Help(i) => {
                let e = help_entries()[i];
                match e.control {
                    Control::Axis(a) => a.name().to_string(),
                    Control::Named(n) => n.name().to_string(),
                    Control::Fixed(_, name) => name.to_string(),
                }
            }
            other => other.fixed_label().to_string(),
        }
    }

    fn fixed_label(self) -> &'static str {
        match self {
            Item::Msaa => "MSAA",
            Item::Scale => "RENDER SCALE",
            Item::AutoScale => "AUTO SCALE",
            Item::Vsync => "VSYNC",
            Item::Quit => "QUIT GAME",
            Item::HoopSize => "HOOP SIZE",
            Item::LandingHoops => "LANDING HOOPS",
            Item::CockpitFrame => "CABIN FRAME",
            Item::CockpitGlow => "CABIN GLOW",
            Item::CockpitHull => "CABIN METAL",
            Item::CockpitRes => "CABIN DETAIL",
            Item::FpsFloor => "FPS FLOOR",
            Item::Sky => "SKY",
            Item::Flare => "LENS FLARE",
            Item::Bloom => "BLOOM",
            Item::Exposure => "EXPOSURE",
            Item::TonemapCurve => "TONEMAP",
            Item::Fringe => "FRINGE",
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
            Item::HoloRange => "HOLO RANGE",
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
            Item::MimicsSize => "MIMIC SIZE",
            Item::MinersCount => "MINERS",
            Item::MinersGrowth => "MINER GROWTH",
            Item::HoldGain => "HOLD GAIN",
            Item::HoldFace => "HOLD FACING",
            Item::BayHue => "HOLO HUE",
            Item::BaySaturation => "HOLO COLOUR",
            Item::BayScanlines => "SCANLINES",
            Item::BaySize => "BAY HOLO SIZE",
            Item::BaySpin => "BAY HOLO SPIN",
            Item::PointerSize => "POINTER SIZE",
            Item::SafeEdge => "SAFE EDGE",
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
            Item::ControlsCard => "CARD AT START",
            Item::Bind(_)
            | Item::BindNamed(_)
            | Item::Slot(_)
            | Item::Mount(_)
            | Item::Heading(_)
            | Item::Help(_) => "",
        }
    }

    /// One line on what the row does, for the footer. Every row has one:
    /// a stranger reads the menu, not the source.
    fn describe(self) -> String {
        match self {
            Item::Bind(a) => help_entries()
                .into_iter()
                .find(|e| e.control == Control::Axis(a))
                .map_or(String::new(), |e| e.what.to_string()),
            Item::BindNamed(n) => help_entries()
                .into_iter()
                .find(|e| e.control == Control::Named(n))
                .map_or(String::new(), |e| e.what.to_string()),
            Item::Help(i) => help_entries()[i].what.to_string(),
            Item::Heading(g) => HELP[g].blurb.to_string(),
            Item::Slot(i) => match i {
                Instrument::Speed => "THE SPEED DIAL: WHERE ON THE DASH IT SITS, OR OFF.".to_string(),
                Instrument::Altitude => "THE ALTIMETER: HEIGHT OVER THE NEAREST BODY. WHERE IT SITS, OR OFF.".to_string(),
                Instrument::Gyro => "THE GYRO BALL: THE SHIP'S ATTITUDE AGAINST THE WORLD. WHERE IT SITS, OR OFF.".to_string(),
                Instrument::GForce => "THE G METER: THE LOAD ON THE PILOT. WHERE IT SITS, OR OFF.".to_string(),
                Instrument::GVector => "THE G VECTOR: WHICH WAY THE LOAD PULLS, ON A CROSS-PLOT. WHERE IT SITS, OR OFF.".to_string(),
                Instrument::Horizon => "THE HORIZON LINE ON THE GLASS.".to_string(),
                Instrument::Ladder => "THE PITCH LADDER ON THE BORESIGHT.".to_string(),
                Instrument::Trajectory => "THE PREDICTED PATH DRAWN AHEAD OF THE SHIP.".to_string(),
                Instrument::Hoops => "THE HOOPS ALONG THE PATH, A KILOMETRE APART.".to_string(),
                Instrument::HoopSound => "THE WOMP A HOOP MAKES AS YOU PASS THROUGH IT.".to_string(),
                Instrument::BodyTags => "FINDER RINGS ROUND THE MOON, THE SUN AND URANUS.".to_string(),
                Instrument::Readout => "THE TEXT READOUT ON THE GLASS: FRAME RATE, ALTITUDE, SPEED, THE COMPUTER'S STATE.".to_string(),
                Instrument::Map => "THE SYSTEM MAP IN MINIATURE, A SMALL PANE ON THE GLASS. M OPENS THE FULL MAP.".to_string(),
            },
            Item::Mount(h) => format!("{}: A CANNON, A RAIL, OR NOTHING.", h.name()),
            other => other.fixed_description().to_string(),
        }
    }

    fn fixed_description(self) -> &'static str {
        match self {
            Item::Msaa => "MULTISAMPLE ANTI-ALIASING: SMOOTHER EDGES FOR MORE GPU WORK.",
            Item::Bloom => "HOW MUCH THE BRIGHT THINGS GLOW: THE SUN, THE FLASHES, THE BRIGHTEST STARS.",
            Item::Exposure => "THE PICTURE'S BRIGHTNESS IN STOPS; THE EYE DRIFTS SLOWLY ABOUT IT.",
            Item::TonemapCurve => "HOW RADIANCE BECOMES THE SCREEN: AGX ROLLS HIGHLIGHTS TO WHITE, OFF CLIPS.",
            Item::Fringe => "A HAIR OF COLOUR SPLIT AT THE GLASS RIM.",
            Item::Dust => "SPACE DUST AND CABIN MOTES: MORE OF THEM, OR NONE.",
            Item::MimicsSize => "HOW BIG A MIMIC SHIP IS NEXT TO YOURS; 100% IS YOUR OWN SIZE.",
            Item::MinersCount => "HOW MANY MINER SHIPS WORK THE BELT.",
            Item::MinersGrowth => "HOW FAST A MINER GROWS THROUGH ITS TIERS AS IT HAULS.",
            Item::Scale => "THE WORLD IS DRAWN AT THIS SHARE OF THE SCREEN'S SIZE; THE GLASS STAYS SHARP.",
            Item::AutoScale => "LET THE RENDER SCALE GOVERN ITSELF DOWN TO HOLD THE FPS FLOOR.",
            Item::Vsync => "WAIT FOR THE DISPLAY EACH FRAME: NO TEARING, A LITTLE LAG.",
            Item::Quit => "LEAVE THE GAME. EVERYTHING IS ALREADY SAVED.",
            Item::HoopSize => "HOW BIG THE PATH'S HOOPS ARE.",
            Item::LandingHoops => "HOW FAR APART THE HOOPS SIT IN LANDING MODE.",
            Item::CockpitFrame => "DRAW THE CABIN ROUND YOU AT ALL.",
            Item::CockpitGlow => "HOW BRIGHT THE CABIN'S LIGHT LINES ARE.",
            Item::CockpitHull => "HOW OPAQUE THE CABIN'S METAL IS.",
            Item::CockpitRes => "THE CABIN IS DRAWN AT THIS SHARE OF THE SCENE'S SIZE.",
            Item::FpsFloor => "THE FRAME RATE THE CABIN GIVES UP DETAIL TO HOLD; OFF FOR NO FLOOR.",
            Item::Sky => "THE DAYTIME SKY'S STRENGTH LOW OVER A PLANET.",
            Item::Flare => "THE SUN'S LENS FLARE ON THE GLASS.",
            Item::Nebula => "THE NEBULA'S GLOW ACROSS THE SKY; OFF FOR NONE.",
            Item::NebulaSeed => "WHICH NEBULA: THE SEED PICKS WHERE THE CLOUDS SIT AND THEIR SHAPES.",
            Item::NebulaScale => "HOW FINE THE GAS IS: BROAD VEILS TO KNOTS.",
            Item::NebulaDensity => "HOW MUCH OF A CLOUD IS GAS: THIN WISPS TO SOLID BANKS.",
            Item::NebulaClouds => "HOW MANY CLOUDS THERE ARE.",
            Item::NebulaHue => "THE FIRST OF THE TWO HUES THE GAS DRIFTS BETWEEN, DEGREES ROUND THE WHEEL.",
            Item::NebulaHue2 => "THE SECOND HUE THE GAS DRIFTS BETWEEN.",
            Item::NebulaSpread => "HOW FAR EACH CLOUD SPREADS: A KNOT TO THE WHOLE SKY.",
            Item::Camera => "THE VIEW: FROM THE COCKPIT, OR A CHASE CAMERA OUTSIDE THE SHIP.",
            Item::HoloView => "THE HOLO3PP: A HOLOGRAM OF THE SHIP AND ITS SURROUNDINGS ON THE DASH.",
            Item::HoloSize => "HOW BIG THE HOLOGRAM STANDS ON THE DASH.",
            Item::HoloRange => "HOW MUCH SPACE THE HOLOGRAM SHOWS ROUND THE SHIP. WIDER: MORE ROOM, A SMALLER SHIP.",
            Item::Fov => "THE VERTICAL FIELD OF VIEW. THE GLASS AND ITS DIALS DO NOT CHANGE WITH IT.",
            Item::GaugeStyle => "HOW THE DIALS ARE MADE: HOLOGRAMS, JET BOWLS, FLUSH DIALS, OR WARTHOG STEAM GAUGES.",
            Item::GaugesStay => "GAUGES STAY LIT, OR FADE WHEN THEY HAVE NOTHING TO SAY.",
            Item::Guide => "THE DESIGN GUIDE: THE SLOTS AND THE SAFE EDGE DRAWN ON THE GLASS.",
            Item::HullSound => "THE HULL'S OWN VOICES: CREAK AND CRACKLE UNDER SPEED, STRIKES.",
            Item::Shield => "HOW BRIGHT THE FORCE FIELD FLARES ON A STRIKE; OFF FOR NONE.",
            Item::ArmsPower => "THE REACTOR'S SHARE FOR THE GUNS: MORE RATE, LESS FOR THE DRIVES.",
            Item::ArmsGlow => "HOW BRIGHT THE MUZZLE FLASH, THE TRACERS AND THE BURSTS ARE.",
            Item::ArmsSight => "THE GUN SIGHT ON THE GLASS: OFF, OR HOW BRIGHT.",
            Item::CamShake => "THE CAMERA ON YOUR HEAD: SWAY UNDER LOAD, TREMOR UNDER THRUST, JOLTS FROM THE GUNS.",
            Item::DriveShake => "HOW HARD THE CHAOS DRIVE JOSTLES THE SHIP ON THE WAY TO THE SLIP.",
            Item::ArmsShards => "HOW MANY SHARDS A BROKEN ROCK THROWS; OFF FOR NONE.",
            Item::ArmsShardLife => "HOW LONG THE SHARDS LAST.",
            Item::ArmsScarSize => "THE CRATERS A HIT LEAVES ON A ROCK; OFF FOR NONE.",
            Item::ArmsScarCool => "HOW LONG A CRATER TAKES TO COOL FROM WHITE TO DARK.",
            Item::ArmsOre => "WHAT THE GUNS BRING IN OFF THE ROCKS; OFF FOR NONE.",
            Item::MimicsChance => "THE SHARE OF ROCKS THAT ARE SHIPS IN A SHROUD.",
            Item::MimicsHostility => "THE SHARE OF THOSE SHIPS THAT SHOOT RATHER THAN HAIL.",
            Item::HoldGain => "HOW HARD THE HOLD LOCK PULLS THE SHIP TO ITS TARGET.",
            Item::HoldFace => "THE HOLD KEEPS THE NOSE ON THE TARGET TOO.",
            Item::BayHue => "THE BAY HOLOGRAM'S HUE, DEGREES ROUND THE WHEEL.",
            Item::BaySaturation => "HOW COLOURED THE BAY HOLOGRAM IS.",
            Item::BayScanlines => "HOW MANY SCANLINES THE BAY HOLOGRAM SHOWS; 0 FOR NONE.",
            Item::BaySize => "HOW BIG THE SHIP IS DRAWN IN THE BAY.",
            Item::BaySpin => "THE BAY HOLOGRAM TURNS BY ITSELF WHEN YOUR HAND IS OFF IT.",
            Item::PointerSize => "THE MOUSE POINTER'S SIZE ON THE PANELS.",
            Item::SafeEdge => "A MARGIN KEPT CLEAR AT THE RIM FOR A DISPLAY WHOSE EDGES ARE HIDDEN OR BENT.",
            Item::DialSelect => "WHICH DIAL THE ROWS BELOW SET.",
            Item::DialSize => "THIS DIAL'S SIZE, AS A MULTIPLE OF THE STOCK DIAL.",
            Item::DialStyle => "THIS DIAL'S OWN STYLE, OR THE COCKPIT'S (AUTO).",
            Item::DialFade => "THIS DIAL STAYS LIT, FADES, OR DOES AS THE COCKPIT DOES (AUTO).",
            Item::DialTilt => "THIS DIAL LEANED TOWARD YOU ABOUT ITS OWN AXIS, DEGREES.",
            Item::MapRings => "RINGS DRAWN ROUND EACH BODY ON THE MAP.",
            Item::MapGrid => "THE MAP'S REFERENCE GRID.",
            Item::LookSens => "HOW FAR THE HEAD TURNS PER MOUSE MOVEMENT.",
            Item::Destination => "WHERE THE WORMHOLE DRIVE TAKES YOU.",
            Item::SafeDist => "HOW FAR OUT FROM THE DESTINATION YOU ARRIVE, IN ITS RADII.",
            Item::Engage => "CLOSE THE MENU AND FIRE THE WORMHOLE DRIVE AT THIS PLAN.",
            Item::ControlsCard => "SHOW THE CONTROLS CARD AT EVERY START. F1 SHOWS IT ANY TIME.",
            Item::Bind(_)
            | Item::BindNamed(_)
            | Item::Slot(_)
            | Item::Mount(_)
            | Item::Heading(_)
            | Item::Help(_) => "",
        }
    }

    /// The settings-file keys this row edits (none for a heading, an
    /// action row, or a help line). The coverage test walks these.
    #[cfg(test)]
    fn keys(self) -> Vec<String> {
        let one = |k: &str| vec![k.to_string()];
        match self {
            Item::Msaa => one("graphics.msaa"),
            Item::Bloom => one("graphics.bloom"),
            Item::Exposure => one("graphics.exposure"),
            Item::TonemapCurve => one("graphics.tonemap"),
            Item::Fringe => one("graphics.fringe"),
            Item::Dust => one("graphics.dust"),
            Item::MimicsSize => one("mimics.size"),
            Item::MinersCount => one("miners.count"),
            Item::MinersGrowth => one("miners.growth"),
            Item::Scale => one("graphics.scale"),
            Item::AutoScale => one("graphics.auto-scale"),
            Item::Vsync => one("graphics.vsync"),
            Item::Bind(a) => vec![format!("control.{}", a.key())],
            Item::BindNamed(n) => vec![format!("control.{}", n.key())],
            Item::Slot(i) => vec![format!("ui.{}", i.key())],
            Item::HoopSize => one("ui.hoop-size"),
            Item::LandingHoops => one("ui.landing-hoops"),
            Item::CockpitFrame => one("cockpit.frame"),
            Item::CockpitGlow => one("cockpit.glow"),
            Item::CockpitHull => one("cockpit.hull"),
            Item::CockpitRes => one("cockpit.res"),
            Item::FpsFloor => one("graphics.fps-floor"),
            Item::Sky => one("graphics.sky"),
            Item::Flare => one("graphics.flare"),
            Item::Nebula => one("graphics.nebula"),
            Item::NebulaSeed => one("graphics.nebula-seed"),
            Item::NebulaScale => one("graphics.nebula-scale"),
            Item::NebulaDensity => one("graphics.nebula-density"),
            Item::NebulaClouds => one("graphics.nebula-clouds"),
            Item::NebulaHue => one("graphics.nebula-hue"),
            Item::NebulaHue2 => one("graphics.nebula-hue2"),
            Item::NebulaSpread => one("graphics.nebula-spread"),
            Item::Fov => one("graphics.fov"),
            Item::GaugeStyle => one("ui.gauge-style"),
            Item::GaugesStay => one("ui.gauges"),
            Item::Guide => one("ui.guide"),
            Item::HullSound => one("sound.hull"),
            Item::Shield => one("ui.shield"),
            Item::DialSize => Instrument::ALL
                .iter()
                .map(|i| format!("ui.{}.size", i.key()))
                .collect(),
            Item::DialStyle => Instrument::ALL
                .iter()
                .map(|i| format!("ui.{}.style", i.key()))
                .collect(),
            Item::DialFade => Instrument::ALL
                .iter()
                .map(|i| format!("ui.{}.fade", i.key()))
                .collect(),
            Item::DialTilt => Instrument::ALL
                .iter()
                .map(|i| format!("ui.{}.tilt", i.key()))
                .collect(),
            Item::Camera => one("camera.chase"),
            Item::HoloView => one("holo.view"),
            Item::HoloSize => one("holo.size"),
            Item::HoloRange => one("holo.range"),
            Item::MapRings => one("map.rings"),
            Item::MapGrid => one("map.grid"),
            Item::LookSens => one("control.look-sens"),
            Item::Destination => one("warp.destination"),
            Item::SafeDist => one("warp.safe-radii"),
            Item::ArmsPower => one("arms.power"),
            Item::ArmsGlow => one("arms.glow"),
            Item::ArmsShards => one("arms.shards"),
            Item::ArmsShardLife => one("arms.shard-life"),
            Item::ArmsScarSize => one("arms.scar-size"),
            Item::ArmsScarCool => one("arms.scar-cool"),
            Item::ArmsOre => one("arms.ore"),
            Item::MimicsChance => one("mimics.chance"),
            Item::MimicsHostility => one("mimics.hostility"),
            Item::HoldGain => one("hold.gain"),
            Item::HoldFace => one("hold.face"),
            Item::ArmsSight => one("arms.sight"),
            Item::CamShake => one("cam.shake"),
            Item::DriveShake => one("cam.drive-shake"),
            Item::Mount(h) => vec![format!("ship.hardpoint.{}", h as usize)],
            Item::BayHue => one("ship.holo-hue"),
            Item::BaySaturation => one("ship.holo-saturation"),
            Item::BayScanlines => one("ship.holo-scanlines"),
            Item::BaySize => one("ship.holo-size"),
            Item::BaySpin => one("ship.holo-spin"),
            Item::PointerSize => one("ui.pointer-size"),
            Item::SafeEdge => one("ui.safe-edge"),
            Item::ControlsCard => one("ui.controls-card"),
            Item::Quit | Item::DialSelect | Item::Engage | Item::Heading(_) | Item::Help(_) => {
                vec![]
            }
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
            Item::HoopSize => format!("{:.2}X", s.hoop_size),
            Item::LandingHoops => format!("{:.0} M", s.landing_spacing_m),
            Item::CockpitFrame => if s.cockpit_frame { "ON" } else { "OFF" }.to_string(),
            Item::CockpitGlow => format!("{:.2}X", s.cockpit_glow),
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
            Item::NebulaHue => format!("{:.0} DEG", s.nebula_hue * 360.0),
            Item::NebulaHue2 => format!("{:.0} DEG", s.nebula_hue2 * 360.0),
            Item::NebulaSpread => format!("{:.2}X", s.nebula_spread),
            Item::Flare => {
                if s.flare > 0.0 {
                    format!("{:.0}%", s.flare * 100.0)
                } else {
                    "OFF".to_string()
                }
            }
            Item::Bloom => {
                if s.bloom > 0.0 {
                    format!("{:.0}%", s.bloom * 100.0)
                } else {
                    "OFF".to_string()
                }
            }
            Item::Exposure => {
                let ev = s.exposure.max(1e-3).log2();
                if ev.abs() < 0.01 {
                    "0 EV".to_string()
                } else {
                    format!("{ev:+.2} EV")
                }
            }
            Item::TonemapCurve => s.tonemap.name().to_string(),
            Item::Fringe => {
                if s.fringe > 0.0 {
                    format!("{:.0}%", s.fringe * 100.0)
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
                    format!("{:.0} FPS", s.fps_floor)
                } else {
                    "OFF".to_string()
                }
            }
            Item::Fov => format!("{:.0} DEG", s.fov),
            Item::Camera => if s.camera_chase { "CHASE" } else { "COCKPIT" }.to_string(),
            Item::HoloView => if s.holo_view { "ON" } else { "OFF" }.to_string(),
            Item::HoloSize => format!("{:.0}%", s.holo_size * 100.0),
            Item::HoloRange => format!("{:.1}X", s.holo_range),
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
            Item::DialSelect
            | Item::DialSize
            | Item::DialStyle
            | Item::DialFade
            | Item::DialTilt => String::new(),
            Item::MapRings => s.map_rings.to_string(),
            Item::MapGrid => if s.map_grid { "ON" } else { "OFF" }.to_string(),
            Item::LookSens => format!("{:.2}X", s.look_sensitivity),
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
            Item::MimicsSize => format!("{:.0}%", s.mimics_size * 100.0),
            Item::MinersCount => {
                if s.miners_count > 0 {
                    format!("{}", s.miners_count)
                } else {
                    "NONE".to_string()
                }
            }
            Item::MinersGrowth => format!("{:.0}%", s.miners_growth * 100.0),
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
            Item::BayHue => format!("{:.0} DEG", s.bay_hue * 360.0),
            Item::BaySaturation => format!("{:.0}%", s.bay_saturation * 100.0),
            Item::BayScanlines => format!("{:.0}", s.bay_scanlines),
            Item::BaySize => format!("{:.0}%", s.bay_size * 100.0),
            Item::BaySpin => if s.bay_spin { "ON" } else { "OFF" }.to_string(),
            Item::PointerSize => format!("{:.0}%", s.pointer_size / 0.045 * 100.0),
            Item::SafeEdge => {
                if s.layout.safe_edge > 0.0 {
                    format!("{:.0}%", s.layout.safe_edge * 100.0)
                } else {
                    "OFF".to_string()
                }
            }
            Item::ControlsCard => if s.controls_card { "ON" } else { "OFF" }.to_string(),
            Item::Heading(_) => String::new(),
            Item::Help(i) => match help_entries()[i].control {
                Control::Axis(a) => key_name(s.bindings.key_for(a)).to_string(),
                Control::Named(n) => key_name(s.bindings.named(n)).to_string(),
                Control::Fixed(key, _) => key.to_string(),
            },
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

/// Rows of items the card shows at once.
pub const VISIBLE_ITEMS: usize = 12;
/// Font pixels per row: the text's line pitch.
pub const ROW_PX: usize = LINE;
/// The card's rows: the header, the items, the footer, two lines of
/// description.
pub const CARD_LINES: usize = 1 + VISIBLE_ITEMS + 1 + 2;
/// The description's lines.
const DESCRIPTION_LINES: usize = 2;
/// The gap the HELP page keeps for the gloss column.
const HELP_KEY_COLS: usize = 10;
const HELP_NAME_COLS: usize = 16;

#[derive(Debug, Clone, Copy)]
pub struct Menu {
    pub open: bool,
    /// A one-page panel (the DRIVE panel beside the map, the bay's
    /// card): no paging, its own header, the narrow width.
    standalone: bool,
    page: Page,
    cursor: usize,
    scroll: usize,
    /// Waiting for a key to bind to the item under the cursor.
    rebinding: bool,
    /// Which of MSAA_CHOICES this GPU can render at (set at start).
    msaa_ok: [bool; 4],
    /// The dial the DIALS page's per-dial block edits.
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
                Item::FpsFloor,
                Item::Fov,
                Item::Camera,
                Item::HoloView,
                Item::HoloSize,
                Item::HoloRange,
                Item::CockpitRes,
                Item::Sky,
                Item::Flare,
                Item::Bloom,
                Item::Exposure,
                Item::TonemapCurve,
                Item::Fringe,
                Item::Dust,
                Item::Nebula,
                Item::NebulaSeed,
                Item::NebulaScale,
                Item::NebulaDensity,
                Item::NebulaClouds,
                Item::NebulaHue,
                Item::NebulaHue2,
                Item::NebulaSpread,
                Item::PointerSize,
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
            // hoops together, the camera on the head, the safe edge.
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
                Item::SafeEdge,
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
                Item::MimicsSize,
                Item::MinersCount,
                Item::MinersGrowth,
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
            // The bay's own card is the fit alone (the hologram is the
            // picture); the menu's SHIP page adds the hologram's look.
            Page::Ship => {
                let mut v: Vec<Item> = Hardpoint::ALL.iter().map(|&h| Item::Mount(h)).collect();
                if !self.standalone {
                    v.extend([
                        Item::BayHue,
                        Item::BaySaturation,
                        Item::BayScanlines,
                        Item::BaySize,
                        Item::BaySpin,
                    ]);
                }
                v
            }
            Page::Help => {
                let mut v = Vec::new();
                let mut n = 0;
                for (g, group) in HELP.iter().enumerate() {
                    v.push(Item::Heading(g));
                    for _ in group.entries {
                        v.push(Item::Help(n));
                        n += 1;
                    }
                }
                v.push(Item::ControlsCard);
                v
            }
        }
    }

    /// Restrict the MSAA choices to what the GPU supports.
    pub fn set_msaa_supported(&mut self, supported: &[u32]) {
        for (i, n) in MSAA_CHOICES.iter().enumerate() {
            self.msaa_ok[i] = supported.contains(n);
        }
    }

    /// The DRIVE panel is showing (the settings menu's MAP page is not it).
    #[cfg(test)]
    pub fn map_open(&self) -> bool {
        self.open && self.standalone && self.page == Page::Map
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

    /// Open on a page.
    #[cfg(test)]
    pub fn open_on(&mut self, page: Page) {
        self.open = true;
        self.set_page(page);
    }

    fn set_page(&mut self, page: Page) {
        self.page = page;
        self.cursor = 0;
        self.scroll = 0;
        self.rebinding = false;
    }

    /// The card's width in characters: the menu's, or a side panel's.
    pub fn cols(&self) -> usize {
        if self.standalone {
            PANEL_COLS
        } else {
            MENU_COLS
        }
    }

    /// The rows an item may use: two characters short of the card for
    /// the scrollbar's gutter.
    fn item_cols(&self) -> usize {
        self.cols() - 2
    }

    /// How many item rows the card shows: the menu's twelve, or every
    /// row of a side panel (they are short).
    fn visible(&self) -> usize {
        if self.standalone {
            self.items().len().max(1)
        } else {
            VISIBLE_ITEMS
        }
    }

    /// The card's size in font pixels: fixed for the menu, so it does
    /// not breathe as the pilot scrolls; a side panel is its rows.
    pub fn extent(&self) -> (usize, usize) {
        let lines = if self.standalone {
            1 + self.visible() + 1 + DESCRIPTION_LINES
        } else {
            CARD_LINES
        };
        (block_width(self.cols()), block_height(lines))
    }

    /// The header's tab under character column `col`, if any.
    fn tab_at(&self, col: usize) -> Option<Page> {
        if self.standalone {
            return None;
        }
        let mut at = 0;
        for p in Page::ALL {
            // The current tab wears its brackets; the rest are bare.
            let w = p.short().len() + if p == self.page { 2 } else { 0 };
            if col >= at && col < at + w {
                return Some(p);
            }
            at += w + 1;
        }
        None
    }

    /// A click on the card's row `row` (0 is the header) at character
    /// column `col`: the cursor goes there and the item is adjusted
    /// forward — the pointer's way through a menu. A click on one of the
    /// header's tabs pages to it.
    pub fn click(&mut self, row: usize, col: usize, settings: &mut Settings) -> MenuEvent {
        let items = self.items();
        if row == 0 {
            if let Some(p) = self.tab_at(col) {
                self.set_page(p);
            }
            return MenuEvent::Nothing;
        }
        if row > self.visible() {
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

    /// A key press while the menu is open.
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
            KeyCode::PageUp => {
                self.cursor = self.cursor.saturating_sub(self.visible());
                self.keep_cursor_visible();
                MenuEvent::Nothing
            }
            KeyCode::PageDown => {
                self.cursor = (self.cursor + self.visible()).min(items.len() - 1);
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
            Item::MimicsSize => {
                let step = if forward { 0.25 } else { -0.25 };
                let next = (s.mimics_size + step).clamp(0.5, 3.0);
                if (next - s.mimics_size).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.mimics_size = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::MinersCount => {
                let max = crate::miner::MAX_MINERS as u32;
                let next = if forward {
                    (s.miners_count + 1).min(max)
                } else {
                    s.miners_count.saturating_sub(1)
                };
                if next == s.miners_count {
                    return MenuEvent::Nothing;
                }
                s.miners_count = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::MinersGrowth => {
                let step = if forward { 0.25 } else { -0.25 };
                let next = (s.miners_growth + step).clamp(0.25, 4.0);
                if (next - s.miners_growth).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.miners_growth = next;
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
            Item::Bloom => {
                let step = if forward { 0.25 } else { -0.25 };
                let next = (s.bloom + step).clamp(0.0, 2.0);
                if (next - s.bloom).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.bloom = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::Exposure => {
                // A quarter of a stop a step, either way.
                let k = if forward {
                    2f32.powf(0.25)
                } else {
                    2f32.powf(-0.25)
                };
                let next = (s.exposure * k).clamp(EXPOSURE_MIN, EXPOSURE_MAX);
                if (next - s.exposure).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.exposure = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::TonemapCurve => {
                s.tonemap = s.tonemap.next(forward);
                MenuEvent::Changed(Change::Layout)
            }
            Item::Fringe => {
                let step = if forward { 0.25 } else { -0.25 };
                let next = (s.fringe + step).clamp(0.0, 2.0);
                if (next - s.fringe).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.fringe = next;
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
            Item::HoloRange => step_f32(
                &mut s.holo_range,
                forward,
                crate::settings::HOLO_RANGE_STEP,
                crate::settings::HOLO_RANGE_MIN,
                crate::settings::HOLO_RANGE_MAX,
            ),
            Item::SafeEdge => {
                let before = s.layout.safe_edge;
                let next = (before + if forward { 0.02 } else { -0.02 })
                    .clamp(0.0, crate::cockpit::SAFE_EDGE_MAX);
                if (next - before).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.layout.set_safe_edge(next);
                MenuEvent::Changed(Change::Layout)
            }
            Item::ControlsCard => {
                s.controls_card = !s.controls_card;
                MenuEvent::Changed(Change::Layout)
            }
            Item::Quit | Item::Bind(_) | Item::BindNamed(_) | Item::Heading(_) | Item::Help(_) => {
                MenuEvent::Nothing
            }
        }
    }

    fn keep_cursor_visible(&mut self) {
        let visible = self.visible();
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + visible {
            self.scroll = self.cursor + 1 - visible;
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
            for (i, p) in Page::ALL.iter().enumerate() {
                if i > 0 {
                    header.push(' ');
                }
                if *p == self.page {
                    header.push_str(&format!("[{}]", p.short()));
                } else {
                    header.push_str(p.short());
                }
            }
        }
        header
    }

    /// One item's row: the cursor mark, the label, the value right-aligned
    /// — always exactly `item_cols` wide. A HELP line is the key, the
    /// control, and its gloss; a heading is its name alone.
    fn line(&self, item: Item, selected: bool, s: &Settings) -> String {
        let cols = self.item_cols();
        let mark = if selected { "\u{25C6}" } else { " " };
        match item {
            Item::Heading(_) => {
                let label = item.label();
                format!("{mark}{label:<w$}", w = cols - 1)
            }
            Item::Help(i) => {
                let e = help_entries()[i];
                let key = item.value(s);
                let name = item.label();
                let line = format!(
                    "{mark}{key:<k$}{name:<n$}{}",
                    e.gloss,
                    k = HELP_KEY_COLS,
                    n = HELP_NAME_COLS
                );
                format!("{line:<w$}", w = cols)
            }
            _ => {
                let value = if selected && self.rebinding {
                    "PRESS A KEY".to_string()
                } else {
                    self.value_of(item, s)
                };
                let label = item.label();
                let pad = cols
                    .saturating_sub(1 + label.chars().count() + value.chars().count())
                    .max(1);
                format!("{mark}{label}{}{value}", " ".repeat(pad))
            }
        }
    }

    /// The footer: where the cursor is in the list, and the keys.
    fn footer(&self) -> String {
        if self.rebinding {
            return "PRESS THE KEY TO BIND   ESC CANCEL".to_string();
        }
        let keys = match self.page {
            _ if self.standalone && self.page == Page::Map => "< > SET  ENTER ENGAGE  M CLOSE",
            _ if self.standalone => "CLICK A SLOT  DRAG TURN  B CLOSE",
            Page::Controls => "TAB PAGE  ENTER BIND  ESC BACK",
            Page::Help => "TAB PAGE  UP DOWN READ  ESC BACK",
            _ => "TAB PAGE  < > ADJUST  ESC BACK",
        };
        if self.standalone {
            keys.to_string()
        } else {
            format!("ROW {}/{}  {keys}", self.cursor + 1, self.items().len())
        }
    }

    /// The chosen row's description, wrapped to the card.
    fn description(&self) -> Vec<String> {
        let items = self.items();
        let item = items[self.cursor.min(items.len() - 1)];
        let mut lines = wrap(&item.describe(), self.cols());
        lines.truncate(DESCRIPTION_LINES);
        lines
    }

    /// The chosen row's top and height in font pixels, for the card's band.
    pub fn cursor_row_px(&self) -> (f32, f32) {
        let row = self.cursor.saturating_sub(self.scroll) + 1;
        ((row * ROW_PX) as f32, ROW_PX as f32)
    }

    /// The scrollbar beside the rows, font px, when the list is longer
    /// than the card.
    pub fn scrollbar(&self) -> Option<Scrollbar> {
        let n = self.items().len();
        let visible = self.visible();
        if n <= visible {
            return None;
        }
        let top = ROW_PX as f32;
        let bottom = ((1 + visible) * ROW_PX) as f32 - 2.0;
        let span = bottom - top;
        let len = (span * visible as f32 / n as f32).max(4.0);
        let at = top + (span - len) * self.scroll as f32 / (n - visible) as f32;
        Some(Scrollbar {
            track: (top, bottom),
            thumb: (at, at + len),
        })
    }

    /// The rules: under the header, over the footer.
    pub fn rules(&self) -> [Option<f32>; 2] {
        let visible = self.visible();
        [
            Some(ROW_PX as f32 - 1.5),
            Some(((1 + visible) * ROW_PX) as f32 - 1.5),
        ]
    }

    /// Draw the menu into the text bitmap.
    pub fn render(&self, text: &mut TextBitmap, s: &Settings) {
        text.clear();
        text.draw_line(0, 0, &self.header());

        let items = self.items();
        let visible = self.visible();
        let end = (self.scroll + visible).min(items.len());
        for (row, idx) in (self.scroll..end).enumerate() {
            text.draw_line(0, row + 1, &self.line(items[idx], idx == self.cursor, s));
        }
        let footer_line = 1 + visible;
        text.draw_line(0, footer_line, &self.footer());
        for (i, line) in self.description().iter().enumerate() {
            text.draw_line(0, footer_line + 1 + i, line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cockpit::Slot;
    use crate::settings::{key_matches, DRAGGED_KEYS, KEYS};
    use farfall_render::text::{has_glyph, COLS, ROWS};

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
        assert!(m.footer().contains("ESC CANCEL"));
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
        m.key(KeyCode::Tab, &mut s); // dials: style, stay, guide, the dial block, the slots
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
        for _ in 0..(Page::ALL.len() - 3) {
            m.key(KeyCode::Tab, &mut s); // round through ARMS, MAP, SHIP, HELP
        }
        assert_eq!(m.page, Page::Graphics);
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
            holo_range: crate::settings::HOLO_RANGE_MAX,
            ..Default::default()
        };
        s.layout.set_free(Instrument::Speed, [0.0, 0.0]);
        s.layout.set_safe_edge(0.3);
        for d in s.dials.iter_mut() {
            d.size = crate::settings::DIAL_SIZE_MAX;
            d.tilt_deg = crate::settings::TILT_MIN;
            d.style = Some(crate::settings::GaugeStyle::Dial);
            d.stay = Some(false);
        }
        // The longest key names on every bind.
        let long = [
            KeyCode::Backquote,
            KeyCode::Backslash,
            KeyCode::BracketLeft,
            KeyCode::BracketRight,
            KeyCode::NumpadEnter,
            KeyCode::ContextMenu,
        ];
        for (i, a) in Action::ALL.iter().enumerate() {
            s.bindings.bind(*a, long[i % long.len()]);
        }
        for (i, n) in Named::ALL.iter().enumerate() {
            s.bindings.bind_named(*n, long[(i + 3) % long.len()]);
        }
        s
    }

    /// Nothing cut off, ever: every header, every row with its full
    /// value, every footer and every description fits the card, on
    /// every page, for the menu and both side panels — and the card
    /// itself fits the bitmap.
    #[test]
    fn every_row_of_every_page_fits_the_card() {
        for s in [Settings::default(), widest_settings()] {
            for mut m in [Menu::new(), Menu::map_panel(), Menu::ship_panel()] {
                m.toggle();
                for _ in 0..Page::ALL.len() {
                    let cols = m.cols();
                    assert!(
                        m.header().chars().count() <= cols,
                        "{} page header is too wide: {:?}",
                        m.page.name(),
                        m.header()
                    );
                    let items = m.items();
                    for (idx, item) in items.iter().enumerate() {
                        m.set_cursor(idx);
                        assert!(
                            m.footer().chars().count() <= cols,
                            "{} footer: {:?}",
                            m.page.name(),
                            m.footer()
                        );
                        let desc = item.describe();
                        assert!(!desc.trim().is_empty(), "{item:?} has no description");
                        let lines = wrap(&desc, cols);
                        assert!(
                            lines.len() <= DESCRIPTION_LINES,
                            "{item:?}'s description needs {} lines: {desc:?}",
                            lines.len()
                        );
                        for c in desc.chars() {
                            assert!(
                                c == ' ' || has_glyph(c),
                                "{item:?}'s description uses {c:?}, which has no glyph"
                            );
                        }
                        for selected in [false, true] {
                            let line = m.line(*item, selected, &s);
                            assert_eq!(
                                line.chars().count(),
                                m.item_cols(),
                                "{} page, {:?}: {line:?}",
                                m.page.name(),
                                item
                            );
                            // Label and value never run into each other.
                            let label = item.label();
                            if !matches!(item, Item::Help(_) | Item::Heading(_)) {
                                let after: String =
                                    line.chars().skip(1 + label.chars().count()).collect();
                                assert!(after.starts_with(' '), "{line:?}");
                                let value = m.value_of(*item, &s);
                                assert!(line.ends_with(&value), "{line:?} lost {value:?}");
                            }
                        }
                    }
                    let (w, h) = m.extent();
                    assert!(w <= COLS && h <= ROWS, "{} card {w}x{h}", m.page.name());
                    m.key(KeyCode::Tab, &mut s.clone());
                }
            }
        }
    }

    /// The card is the same size in canopy units on every screen, and
    /// at the smallest supported window (800x600) it fits inside the
    /// screen with its whole width and height.
    #[test]
    fn the_card_fits_the_smallest_window() {
        let m = Menu::new();
        let (w, h) = m.extent();
        let px = crate::panel::px_canopy(600.0);
        let aspect = 800.0 / 600.0;
        let width_ndc = w as f32 * px / aspect;
        let height_ndc = h as f32 * px;
        assert!(width_ndc < 1.9, "card {width_ndc} of 2 wide");
        assert!(height_ndc < 1.9, "card {height_ndc} of 2 tall");
        // And not a postage stamp: it is the screen's main thing.
        assert!(
            width_ndc > 0.9 && height_ndc > 0.6,
            "{width_ndc}x{height_ndc}"
        );
        // The same card at the real display's size takes the same share.
        assert!((crate::panel::px_canopy(1800.0) - px).abs() < 1e-6);
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
                    assert!(on(Item::FpsFloor) && on(Item::HoloRange));
                    // The nebula block sits together, after the sky knobs.
                    let at = |it: Item| items.iter().position(|i| *i == it).unwrap();
                    assert!(at(Item::Nebula) > at(Item::Flare));
                    // The picture block — bloom, exposure, curve, rim —
                    // sits together between the flare and the nebula.
                    assert_eq!(at(Item::Bloom), at(Item::Flare) + 1);
                    assert_eq!(at(Item::Exposure), at(Item::Bloom) + 1);
                    assert_eq!(at(Item::TonemapCurve), at(Item::Bloom) + 2);
                    assert_eq!(at(Item::Fringe), at(Item::Bloom) + 3);
                    assert!(at(Item::Nebula) > at(Item::Fringe));
                    assert_eq!(at(Item::NebulaSeed), at(Item::Nebula) + 1);
                    assert_eq!(at(Item::NebulaSpread), at(Item::Nebula) + 7);
                    assert_eq!(*items.last().unwrap(), Item::Quit, "QUIT is the last thing");
                }
                Page::Controls => {
                    assert!(items.iter().all(|i| i.rebindable() || *i == Item::LookSens));
                    // EVERY bind the game answers to is on this page: all
                    // twelve axis actions and every named control. A key
                    // that works in game but is missing here is a bug.
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
                    assert!(on(Item::CamShake) && on(Item::SafeEdge));
                }
                Page::Gauges => {
                    assert!(on(Item::GaugeStyle) && on(Item::GaugesStay) && on(Item::Guide));
                    assert!(on(Item::DialTilt) && on(Item::Slot(Instrument::Gyro)));
                    assert!(on(Item::Slot(Instrument::Map)), "the mini map is a gauge");
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
                    assert!(on(Item::MinersCount) && on(Item::MinersGrowth));
                    assert!(on(Item::MimicsSize));
                    assert!(on(Item::ArmsSight));
                    assert!(items.len() >= 2);
                }
                Page::Map => {
                    assert!(on(Item::Destination) && on(Item::SafeDist) && on(Item::Engage));
                    assert!(on(Item::MapRings) && on(Item::MapGrid));
                }
                Page::Ship => {
                    for h in Hardpoint::ALL {
                        assert!(on(Item::Mount(h)));
                    }
                    assert!(on(Item::BayHue) && on(Item::BaySpin));
                }
                Page::Help => {
                    assert_eq!(items[0], Item::Heading(0));
                    assert_eq!(*items.last().unwrap(), Item::ControlsCard);
                }
            }
        }
    }

    /// The HELP page lists every control the game answers to exactly
    /// once, by group, each with a gloss that fits its column and a
    /// sentence for the footer — and every glyph it needs exists.
    #[test]
    fn the_help_page_lists_every_control_once_with_what_it_does() {
        let entries = help_entries();
        for a in Action::ALL {
            let n = entries
                .iter()
                .filter(|e| e.control == Control::Axis(a))
                .count();
            assert_eq!(n, 1, "{a:?} is on the HELP page {n} times");
        }
        for nm in Named::ALL {
            let n = entries
                .iter()
                .filter(|e| e.control == Control::Named(nm))
                .count();
            assert_eq!(n, 1, "{nm:?} is on the HELP page {n} times");
        }
        // The fixed keys the game keeps: the menu, its page, the card,
        // the mouse buttons, the pane zoom.
        for key in ["ESC", "TAB", "F1", "LMB", "RMB", "+ -"] {
            assert!(
                entries
                    .iter()
                    .any(|e| matches!(e.control, Control::Fixed(k, _) if k == key)),
                "fixed key {key} is not on the HELP page"
            );
        }
        let gloss_cols = MENU_COLS - 2 - 1 - HELP_KEY_COLS - HELP_NAME_COLS;
        for e in &entries {
            assert!(
                e.gloss.chars().count() <= gloss_cols,
                "gloss too long: {:?}",
                e.gloss
            );
            assert!(e.what.ends_with('.'), "a sentence: {:?}", e.what);
            for c in e.gloss.chars().chain(e.what.chars()) {
                assert!(c == ' ' || has_glyph(c), "{c:?} in {:?}", e.what);
            }
        }
        for g in HELP {
            assert!(!g.entries.is_empty());
            assert!(g.name.chars().count() <= 12);
        }
        // Reading the page: the cursor walks the lines, the value column
        // shows the live key, and nothing changes.
        let mut m = Menu::new();
        let mut s = Settings::default();
        m.open_on(Page::Help);
        assert_eq!(m.key(KeyCode::ArrowRight, &mut s), MenuEvent::Nothing);
        m.key(KeyCode::ArrowDown, &mut s);
        assert_eq!(m.items()[m.cursor], Item::Help(0));
        assert!(m.line(Item::Help(0), true, &s).starts_with("\u{25C6}W "));
        assert_eq!(m.key(KeyCode::Enter, &mut s), MenuEvent::Nothing);
        assert_eq!(s, Settings::default());
    }

    /// Every settings key has a row somewhere in the menu (the dragged
    /// anchors excepted): a setting only the file can reach is a
    /// setting a pilot cannot find.
    #[test]
    fn every_settings_key_has_a_menu_row() {
        let mut m = Menu::new();
        m.toggle();
        let mut claimed: Vec<String> = Vec::new();
        for page in Page::ALL {
            m.set_page(page);
            for it in m.items() {
                claimed.extend(it.keys());
            }
        }
        // Every key the file writes is claimed by a row.
        let mut s = Settings::default();
        for d in s.dials.iter_mut() {
            d.size = 1.5;
        }
        for line in s.render().lines() {
            let Some((k, _)) = line.split_once('=') else {
                continue;
            };
            let k = k.trim();
            if DRAGGED_KEYS.contains(&k) {
                continue;
            }
            assert!(claimed.iter().any(|c| c == k), "no menu row edits {k}");
        }
        // And every listed name or pattern is claimed by a row.
        for p in KEYS {
            assert!(
                claimed.iter().any(|c| key_matches(p, c)),
                "no menu row edits {p}"
            );
        }
    }

    /// The list scrolls under a bar that shows where you are, and the
    /// footer counts the rows.
    #[test]
    fn long_pages_scroll_with_a_bar_and_a_count() {
        let mut m = Menu::new();
        let mut s = Settings::default();
        m.open_on(Page::Controls);
        let n = m.items().len();
        assert!(n > VISIBLE_ITEMS);
        let bar = m.scrollbar().expect("a long page has a bar");
        assert_eq!(bar.thumb.0, bar.track.0, "at the top");
        assert!(m.footer().starts_with(&format!("ROW 1/{n}")));
        for _ in 0..VISIBLE_ITEMS {
            m.key(KeyCode::ArrowDown, &mut s);
        }
        assert_eq!(m.scroll, 1, "the cursor pushes the list up one");
        let bar2 = m.scrollbar().unwrap();
        assert!(bar2.thumb.0 > bar.thumb.0, "the thumb moves down");
        assert!(bar2.thumb.1 <= bar2.track.1);
        m.key(KeyCode::ArrowUp, &mut s);
        m.key(KeyCode::ArrowUp, &mut s);
        m.key(KeyCode::ArrowUp, &mut s);
        m.key(KeyCode::ArrowUp, &mut s);
        m.key(KeyCode::ArrowUp, &mut s);
        m.key(KeyCode::ArrowUp, &mut s);
        m.key(KeyCode::ArrowUp, &mut s);
        m.key(KeyCode::ArrowUp, &mut s);
        m.key(KeyCode::ArrowUp, &mut s);
        m.key(KeyCode::ArrowUp, &mut s);
        m.key(KeyCode::ArrowUp, &mut s);
        m.key(KeyCode::ArrowUp, &mut s);
        m.key(KeyCode::ArrowUp, &mut s);
        assert_eq!(m.cursor, n - 1, "up from the top wraps to the end");
        assert_eq!(m.scroll, n - VISIBLE_ITEMS);
        let bar3 = m.scrollbar().unwrap();
        assert!((bar3.thumb.1 - bar3.track.1).abs() < 1e-3, "at the bottom");
        assert!(m.footer().starts_with(&format!("ROW {n}/{n}")));
        m.key(KeyCode::PageUp, &mut s);
        assert_eq!(m.cursor, n - 1 - VISIBLE_ITEMS);
        // A short page has no bar.
        m.open_on(Page::Map);
        assert!(m.scrollbar().is_none());
        // The chosen row's band sits on its row of the card.
        m.set_cursor(2);
        assert_eq!(m.cursor_row_px(), ((3 * ROW_PX) as f32, ROW_PX as f32));
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
        assert_eq!(m.click(0, 3, &mut s), MenuEvent::Nothing);
        assert_eq!(m.click(9, 0, &mut s), MenuEvent::Nothing);
        let before = s.mounts[1];
        assert_eq!(m.click(2, 0, &mut s), MenuEvent::Changed(Change::Layout));
        assert_eq!(m.bay_selected(), Some(1));
        assert_ne!(s.mounts[1], before);
        assert!(m.header().contains("SHIP BAY"));
        assert_eq!(m.cols(), PANEL_COLS);
        assert_eq!(m.key(KeyCode::Escape, &mut s), MenuEvent::Closed);
        // The look rows live on the menu's SHIP page and light no pip:
        // hue wraps, the rest clamp, spin flips; the pointer sizes on GFX.
        let mut m = Menu::new();
        m.open_on(Page::Ship);
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
        m.open_on(Page::Graphics);
        let at = m
            .items()
            .iter()
            .position(|&i| i == Item::PointerSize)
            .unwrap();
        m.set_cursor(at);
        for _ in 0..40 {
            m.key(KeyCode::ArrowRight, &mut s);
        }
        assert_eq!(s.pointer_size, 0.1);
    }

    #[test]
    fn the_drive_panel_sets_the_plan_and_engages_and_never_pages() {
        // The settings menu's MAP page is a page, not the DRIVE panel:
        // Tab cycles past it and the map is not drawn under it.
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

    /// The header is a row of tabs: a click on one pages to it, a click
    /// between them is nothing, and the tabs sit where the header draws
    /// them.
    #[test]
    fn a_click_on_a_tab_pages_to_it() {
        let mut m = Menu::new();
        let mut s = Settings::default();
        m.toggle();
        let header = m.header();
        for p in Page::ALL {
            let col = header.find(p.short()).unwrap();
            assert_eq!(m.click(0, col, &mut s), MenuEvent::Nothing);
            assert_eq!(m.page, p, "clicked {} at column {col}", p.short());
        }
        let last = Page::ALL[Page::ALL.len() - 1];
        assert_eq!(m.click(0, MENU_COLS + 5, &mut s), MenuEvent::Nothing);
        assert_eq!(m.page, last, "a click past the tabs pages nowhere");
        let mut side = Menu::map_panel();
        side.toggle();
        side.click(0, 1, &mut s);
        assert_eq!(side.page, Page::Map, "a side panel has no tabs");
    }

    #[test]
    fn the_hologram_range_and_the_card_have_rows() {
        let mut m = Menu::new();
        let mut s = Settings::default();
        m.open_on(Page::Graphics);
        let at = m
            .items()
            .iter()
            .position(|&i| i == Item::HoloRange)
            .unwrap();
        m.set_cursor(at);
        assert_eq!(
            m.key(KeyCode::ArrowRight, &mut s),
            MenuEvent::Changed(Change::Layout)
        );
        assert_eq!(s.holo_range, 1.5);
        for _ in 0..20 {
            m.key(KeyCode::ArrowRight, &mut s);
        }
        assert_eq!(s.holo_range, crate::settings::HOLO_RANGE_MAX);
        assert_eq!(m.key(KeyCode::ArrowRight, &mut s), MenuEvent::Nothing);
        m.open_on(Page::Help);
        m.set_cursor(m.items().len() - 1);
        assert_eq!(m.items()[m.cursor], Item::ControlsCard);
        m.key(KeyCode::Enter, &mut s);
        assert!(s.controls_card);
        m.open_on(Page::Cockpit);
        let at = m.items().iter().position(|&i| i == Item::SafeEdge).unwrap();
        m.set_cursor(at);
        m.key(KeyCode::ArrowRight, &mut s);
        assert!((s.layout.safe_edge - 0.02).abs() < 1e-6);
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
        let (cw, ch) = m.extent();
        assert!(w <= cw && h <= ch, "{w}x{h} in {cw}x{ch}");
        assert!(w > cw / 2);
        // Every row the card promises is drawn: the header, twelve items,
        // the footer and a description of one or two lines.
        assert!(
            h <= block_height(CARD_LINES) && h >= block_height(CARD_LINES - 1),
            "{h}"
        );
    }
}
