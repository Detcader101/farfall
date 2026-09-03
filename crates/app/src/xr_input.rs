//! Controller input (fable/vr-hands), behind one trait so a bench can
//! run the whole lane without a headset: [`HandSource`] is either
//! [`OpenXrHands`] (a real Valve Index action set — aim pose, grip
//! pose, trigger value, squeeze value, thumbstick, A/B click, haptic
//! output, per hand, suggested for
//! `/interaction_profiles/valve/index_controller` with a
//! `/interaction_profiles/khr/simple_controller` fallback for whatever
//! that profile actually has: grip/aim pose, a boolean "select" mapped
//! onto the trigger action via OpenXR's own click→float conversion, and
//! haptics) or [`SynthHands`] (a deterministic scripted pair of hands,
//! no runtime required at all). Shape ported from openxrs' example
//! (Ralith/openxrs, MIT/Apache-2.0, `openxr/examples/vulkan.rs`) — see
//! `docs/RESEARCH-VR-OSS.md` §1.
//!
//! [`OpenXrHands::new`] must run once, right after [`crate::xr::init`]
//! succeeds and before the event loop ever calls `begin_frame` — OpenXR
//! requires every action set be attached
//! (`Session::attach_action_sets`) before the session leaves its
//! unattached state, and the session does not reach `READY` (the point
//! `xr::XrSession::begin_frame` first calls `session.begin`) until some
//! polling has happened. [`HandSource::sync`] and [`HandSource::hands`]
//! are called every frame from `xr_begin_frame` in `lib.rs`, right
//! beside where `game.vr` itself is set from the eyes — hands and eyes
//! are the same seam, located in the same recentred LOCAL space.
//!
//! ## Which source runs (`FARFALL_VR_HANDS`)
//!
//! `FARFALL_VR_HANDS=synth` forces [`SynthHands`]; `=real` forces
//! [`OpenXrHands`]. Unset, it follows `FARFALL_VR`: `FARFALL_VR=synth`
//! (fable/vr's synthetic Index headset) defaults hands to synthetic
//! too, anything else defaults to real ([`hands_mode`]/[`hands_mode_for`]).
//! `SynthHands` reads its own script from `FARFALL_VR_SCRIPT` — the
//! same variable fable/vr's synthetic head uses for its own motion
//! (`still`/`look`/`lean`/`nod`/`spin`); a value this module doesn't
//! recognise as a hand script (including all of those) is simply
//! [`HandScript::Idle`] here, and vice versa on the head side. Every
//! script is a deterministic pure function of elapsed bench time
//! (`Game::started.elapsed()`), so a run is exactly reproducible, and
//! every state transition (a reach completing, a grab taken, a release,
//! a press) logs one line — `"VR hands: right GRAB stick t=2.1s"` — so
//! a bench harness can assert a script did what it claims without
//! reading pixels.

use glam::{Quat, Vec3};

use crate::{HandPose, VrHands};

// ---------------------------------------------------------------------
// Pure data + maths — runs with no runtime, no loader, no GPU, and is
// unit-tested accordingly (mirrors the split `xr.rs` itself makes).
// ---------------------------------------------------------------------

/// One action's suggested bindings: the path under
/// `/user/hand/{left,right}/` for the Valve Index profile, and for
/// `khr/simple_controller` — `None` where that profile has no
/// equivalent input. Per the OpenXR spec's own interaction-profile
/// tables, `khr/simple_controller` has only a pose pair, one boolean
/// `select`, one boolean `menu`, and haptics — no analog trigger,
/// squeeze, thumbstick or A/B.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindingSpec {
    pub action: &'static str,
    pub index_path: &'static str,
    pub simple_path: Option<&'static str>,
}

pub const BINDINGS: &[BindingSpec] = &[
    BindingSpec {
        action: "aim_pose",
        index_path: "input/aim/pose",
        simple_path: Some("input/aim/pose"),
    },
    BindingSpec {
        action: "grip_pose",
        index_path: "input/grip/pose",
        simple_path: Some("input/grip/pose"),
    },
    // simple_controller's only "press" input is /input/select/click
    // (boolean); OpenXR's implicit action-type conversion turns that
    // into 0.0/1.0 for an f32 action — exactly what a "some controller,
    // no analog trigger" fallback wants.
    BindingSpec {
        action: "trigger",
        index_path: "input/trigger/value",
        simple_path: Some("input/select/click"),
    },
    BindingSpec {
        action: "squeeze",
        index_path: "input/squeeze/value",
        simple_path: None,
    },
    BindingSpec {
        action: "thumbstick",
        index_path: "input/thumbstick",
        simple_path: None,
    },
    BindingSpec {
        action: "a_click",
        index_path: "input/a/click",
        simple_path: None,
    },
    BindingSpec {
        action: "b_click",
        index_path: "input/b/click",
        simple_path: None,
    },
    BindingSpec {
        action: "haptic",
        index_path: "output/haptic",
        simple_path: Some("output/haptic"),
    },
];

pub const HANDS: [&str; 2] = ["left", "right"];

/// This action's full OpenXR path for one hand and one profile's own
/// suffix (`BindingSpec::index_path`/`simple_path`).
pub fn full_path(hand: &str, suffix: &str) -> String {
    format!("/user/hand/{hand}/{suffix}")
}

/// An OpenXR pose as `(orientation, position)` — whatever space it was
/// located in. Once that space is the session's current LOCAL space
/// (`xr::XrSession`'s own `space`, already the ship's frame per
/// `xr::try_init`'s doc comment: OpenXR's +X right/+Y up/−Z forward is
/// the ship's frame exactly), this is the ship-frame pose `HandPose`
/// wants — the same convention `VrEye` uses for the head.
pub fn pose_from_openxr(p: openxr::Posef) -> (Quat, Vec3) {
    (
        Quat::from_xyzw(
            p.orientation.x,
            p.orientation.y,
            p.orientation.z,
            p.orientation.w,
        )
        .normalize(),
        Vec3::new(p.position.x, p.position.y, p.position.z),
    )
}

/// Which [`HandSource`] runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandsMode {
    Real,
    Synth,
}

/// [`hands_mode`]'s logic, pure: given `FARFALL_VR_HANDS` and
/// `FARFALL_VR`'s own values (or `None` if unset), which source runs.
/// `FARFALL_VR_HANDS` wins outright when it names either mode
/// explicitly; otherwise a synthetic headset (`FARFALL_VR=synth`)
/// defaults hands to synthetic too, and anything else defaults real.
pub fn hands_mode_for(hands_var: Option<&str>, vr_var: Option<&str>) -> HandsMode {
    match hands_var {
        Some("synth") => return HandsMode::Synth,
        Some("real") => return HandsMode::Real,
        _ => {}
    }
    if vr_var == Some("synth") {
        HandsMode::Synth
    } else {
        HandsMode::Real
    }
}

/// [`hands_mode_for`] against the real environment.
pub fn hands_mode() -> HandsMode {
    hands_mode_for(
        std::env::var("FARFALL_VR_HANDS").ok().as_deref(),
        std::env::var("FARFALL_VR").ok().as_deref(),
    )
}

/// A named, deterministic synthetic hand script (`FARFALL_VR_SCRIPT`,
/// shared with fable/vr's head scripts — see the module doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandScript {
    /// Both hands resting on the lap: tracked, motionless, no squeeze.
    Idle,
    /// The right hand travels from the lap to the stick over
    /// [`REACH_S`] seconds, then holds there without grabbing.
    ReachStick,
    /// The right hand reaches the stick, grabs it, then rolls it back
    /// and forth — the roll axis (`InputState::summed`, `Controls::
    /// torque_body.z`) must show the demand.
    GrabStickRoll,
    /// The left hand reaches the throttle, grabs it, then pushes it
    /// from 0 to 80% over the run.
    ThrottlePush,
    /// The right hand's aim ray sweeps across the open menu and, at the
    /// end, presses — the pointer/click path must register it.
    LaserMenu,
}

impl HandScript {
    /// `FARFALL_VR_SCRIPT`'s value, or `None` for a name this module
    /// does not recognise as a hand script — a head-only name
    /// (`still`/`look`/`lean`/`nod`/`spin`) or an unknown one, both of
    /// which fall back to [`HandScript::Idle`] at the call site.
    pub fn from_env_value(v: &str) -> Option<HandScript> {
        match v {
            "idle" => Some(HandScript::Idle),
            "reach-stick" => Some(HandScript::ReachStick),
            "grab-stick-roll" => Some(HandScript::GrabStickRoll),
            "throttle-push" => Some(HandScript::ThrottlePush),
            "laser-menu" => Some(HandScript::LaserMenu),
            _ => None,
        }
    }
}

/// Which beat of a script time `t` falls in — logged once per change
/// (not every frame) by [`SynthHands::hands`], and the harness's own
/// hook: a script's own timeline is fully determined by
/// [`synth_hands`], so asserting "did REACH stick complete by t=2.2s"
/// needs nothing but this log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Beat {
    Idle,
    ReachStick,
    HoldStick,
    GrabStick,
    RollStick,
    ReachThrottle,
    GrabThrottle,
    PushThrottle,
    ReachLaser,
    PointLaser,
    LaserPress,
}

impl Beat {
    /// The word(s) after `"VR hands: {hand} "` a log line carries — see
    /// [`SynthHands::hands`]. Chosen to match the harness's own greps
    /// verbatim: `"VR hands: right GRAB stick"`, `"VR hands: .*(REACH|
    /// reach).*stick"`, `"VR hands: .*(GRAB|PUSH|grab|push).*throttle"`,
    /// `"VR hands: .*(LASER|POINT|laser|point)"` — every laser beat
    /// below carries both `LASER` and `laser`/`POINT` so any one of
    /// them alone still satisfies that last grep.
    fn label(self) -> &'static str {
        match self {
            Beat::Idle => "IDLE",
            Beat::ReachStick => "REACH stick",
            Beat::HoldStick => "HOLD stick",
            Beat::GrabStick => "GRAB stick",
            Beat::RollStick => "ROLL stick",
            Beat::ReachThrottle => "REACH throttle",
            Beat::GrabThrottle => "GRAB throttle",
            Beat::PushThrottle => "PUSH throttle",
            Beat::ReachLaser => "REACH laser",
            Beat::PointLaser => "POINT laser",
            Beat::LaserPress => "LASER press",
        }
    }
}

/// Where a resting hand sits, ship frame — clear of the stick and the
/// throttle both, so "idle" is visibly distinct from either grab.
pub const LAP_LEFT: Vec3 = Vec3::new(-0.22, -1.05, -0.15);
pub const LAP_RIGHT: Vec3 = Vec3::new(0.22, -1.05, -0.15);

/// How long a reach travels, and how long a grab's squeeze takes to
/// ramp in, seconds — generous enough to read clearly in a capture.
pub const REACH_S: f32 = 2.0;
pub const GRAB_RAMP_S: f32 = 0.6;
const THROTTLE_REACH_S: f32 = 1.5;
const THROTTLE_PUSH_S: f32 = 6.0;
const AIM_REACH_S: f32 = 1.0;
const AIM_SWEEP_S: f32 = 3.0;
const AIM_PRESS_AT_S: f32 = 3.5;
const AIM_PRESS_DUR_S: f32 = 0.15;
const AIM_POS: Vec3 = Vec3::new(0.18, -0.35, -0.35);

/// Smoothstep: a natural ease in/out for a scripted reach, `f` clamped
/// to 0..1 first so an out-of-range call still returns a sane 0..1.
fn ease(f: f32) -> f32 {
    let f = f.clamp(0.0, 1.0);
    f * f * (3.0 - 2.0 * f)
}

/// The aim ray's own downward pitch for every script that isn't
/// `laser-menu` (radians, about +X): pointed at the console rather than
/// forward at the pilot's own eye height, so `xr_laser::ray_hits_glass`
/// naturally misses the virtual glass and no beam draws — a synthetic
/// hand reaching for the stick or throttle is not also trying to point
/// at a panel, and a beam terminating somewhere in the dial cluster
/// (this pitch's own -Z-forward default would have hit roughly there)
/// reads as "the hand target is wrong" even when it is the unrelated
/// laser doing that, not the grip position.
const AIM_DOWN_PITCH_RAD: f32 = -1.3;

fn pose_at(pos: Vec3, trigger: f32, squeeze: f32) -> HandPose {
    HandPose {
        aim: (Quat::from_rotation_x(AIM_DOWN_PITCH_RAD), pos),
        grip: (Quat::IDENTITY, pos),
        trigger,
        squeeze,
        thumbstick: (0.0, 0.0),
        a: false,
        b: false,
    }
}

fn idle_pose(pos: Vec3) -> HandPose {
    pose_at(pos, 0.0, 0.0)
}

fn reach_stick_pose(t: f32, grab_after: bool) -> (HandPose, Beat) {
    if t < REACH_S {
        let pos = LAP_RIGHT.lerp(crate::xr_grab::STICK_REST, ease(t / REACH_S));
        return (idle_pose(pos), Beat::ReachStick);
    }
    if !grab_after {
        return (idle_pose(crate::xr_grab::STICK_REST), Beat::HoldStick);
    }
    let since_arrive = t - REACH_S;
    if since_arrive < GRAB_RAMP_S {
        let squeeze = (since_arrive / GRAB_RAMP_S).clamp(0.0, 1.0);
        return (
            pose_at(crate::xr_grab::STICK_REST, 0.0, squeeze),
            Beat::GrabStick,
        );
    }
    // Rolling: oscillate the grip's X position around the stick's own
    // rest, +/-8cm at 0.5Hz — past `xr_grab`'s dead zone and enough of
    // its sensitivity to read as a clear, repeating roll demand rather
    // than a twitch.
    let roll_t = since_arrive - GRAB_RAMP_S;
    let x = (roll_t * std::f32::consts::TAU * 0.5).sin() * 0.08;
    let pos = crate::xr_grab::STICK_REST + Vec3::new(x, 0.0, 0.0);
    (pose_at(pos, 0.0, 0.9), Beat::RollStick)
}

fn throttle_pose(t: f32) -> (HandPose, Beat) {
    if t < THROTTLE_REACH_S {
        let pos = LAP_LEFT.lerp(crate::xr_grab::THROTTLE_REST, ease(t / THROTTLE_REACH_S));
        return (idle_pose(pos), Beat::ReachThrottle);
    }
    let since_arrive = t - THROTTLE_REACH_S;
    if since_arrive < GRAB_RAMP_S {
        let squeeze = (since_arrive / GRAB_RAMP_S).clamp(0.0, 1.0);
        return (
            pose_at(crate::xr_grab::THROTTLE_REST, 0.0, squeeze),
            Beat::GrabThrottle,
        );
    }
    let push_t = since_arrive - GRAB_RAMP_S;
    let f = (push_t / THROTTLE_PUSH_S).clamp(0.0, 1.0);
    // 0..80% of full deflection: an axis of 0.8 needs this much
    // displacement past `xr_grab`'s own dead zone and sensitivity.
    let target = -(crate::xr_grab::DEAD_ZONE_M + 0.8 / crate::xr_grab::SENSITIVITY_PER_M);
    let pos = crate::xr_grab::THROTTLE_REST + Vec3::new(0.0, 0.0, target * f);
    (pose_at(pos, 0.0, 0.9), Beat::PushThrottle)
}

fn laser_menu_pose(t: f32) -> (HandPose, Beat) {
    if t < AIM_REACH_S {
        let pos = LAP_RIGHT.lerp(AIM_POS, ease(t / AIM_REACH_S));
        return (idle_pose(pos), Beat::ReachLaser);
    }
    let since = t - AIM_REACH_S;
    let yaw = if since < AIM_SWEEP_S {
        (since / AIM_SWEEP_S * std::f32::consts::PI).sin() * 0.3
    } else {
        0.0
    };
    let rot = Quat::from_rotation_y(yaw);
    let pressing = (AIM_PRESS_AT_S..AIM_PRESS_AT_S + AIM_PRESS_DUR_S).contains(&t);
    let pose = HandPose {
        aim: (rot, AIM_POS),
        grip: (rot, AIM_POS),
        trigger: if pressing { 1.0 } else { 0.0 },
        squeeze: 0.0,
        thumbstick: (0.0, 0.0),
        a: false,
        b: false,
    };
    (
        pose,
        if pressing {
            Beat::LaserPress
        } else {
            Beat::PointLaser
        },
    )
}

/// Both hands and their beats (index 0 left, 1 right) at time `t`
/// (seconds since the script started) — the whole of what
/// [`SynthHands`] needs, and fully pure: no runtime, no clock of its
/// own, exercised by the pure tests below.
pub fn synth_hands(script: HandScript, t: f32) -> (VrHands, [Beat; 2]) {
    let t = t.max(0.0);
    match script {
        HandScript::Idle => (
            VrHands {
                left: Some(idle_pose(LAP_LEFT)),
                right: Some(idle_pose(LAP_RIGHT)),
            },
            [Beat::Idle, Beat::Idle],
        ),
        HandScript::ReachStick => {
            let (right, beat) = reach_stick_pose(t, false);
            (
                VrHands {
                    left: Some(idle_pose(LAP_LEFT)),
                    right: Some(right),
                },
                [Beat::Idle, beat],
            )
        }
        HandScript::GrabStickRoll => {
            let (right, beat) = reach_stick_pose(t, true);
            (
                VrHands {
                    left: Some(idle_pose(LAP_LEFT)),
                    right: Some(right),
                },
                [Beat::Idle, beat],
            )
        }
        HandScript::ThrottlePush => {
            let (left, beat) = throttle_pose(t);
            (
                VrHands {
                    left: Some(left),
                    right: Some(idle_pose(LAP_RIGHT)),
                },
                [beat, Beat::Idle],
            )
        }
        HandScript::LaserMenu => {
            let (right, beat) = laser_menu_pose(t);
            (
                VrHands {
                    left: Some(idle_pose(LAP_LEFT)),
                    right: Some(right),
                },
                [Beat::Idle, beat],
            )
        }
    }
}

/// One frame's controller state, from whichever source is live —
/// object-safe, so `Gpu::xr_input` can hold `Box<dyn HandSource>`
/// chosen once at startup by [`hands_mode`].
pub trait HandSource {
    /// Sync this frame's action states (real: `sync_actions`; synth:
    /// a no-op). Call once a frame, before [`Self::hands`].
    fn sync(&self) -> openxr::Result<()>;

    /// Both hands' state this frame. `locate`, when there is a session
    /// to locate poses against (its current LOCAL space and the
    /// predicted display time), is what a real source reads — `None`
    /// on a headless run (no session at all, real or the sibling's
    /// synthetic one) leaves a real source with nothing to report.
    /// `bench_t_s` (`Game::started.elapsed()`) is a synthetic source's
    /// one and only clock; it ignores `locate` either way.
    fn hands(&mut self, locate: Option<(&openxr::Space, openxr::Time)>, bench_t_s: f32) -> VrHands;

    /// A short haptic pulse on one hand (0 left, 1 right). `amplitude`
    /// 0..1, `duration_s` seconds. A real source drops the error
    /// (log-and-continue: a haptic is confirmatory, never load-bearing)
    /// rather than panicking; a synthetic one just logs what it would
    /// have felt.
    fn pulse(&self, hand: usize, amplitude: f32, duration_s: f32);
}

// ---------------------------------------------------------------------
// The runtime: everything below touches a real OpenXR session and
// cannot run in a unit test; exercised by `cargo check` and, eventually,
// a headset — the same split `xr.rs` itself makes.
// ---------------------------------------------------------------------

/// The attached action set plus the per-hand action spaces it created —
/// the OpenXR side of hand input, alongside `xr::XrSession`'s eyes.
/// Holds its own clone of the session (cheap — an `Arc` internally, per
/// the `openxr` crate) so [`HandSource`]'s methods need no session
/// parameter of their own; `space`/`time` still come in fresh each
/// frame from `HandSource::hands`, since the LOCAL space changes on
/// every VR RECENTRE.
pub struct OpenXrHands {
    session: openxr::Session<openxr::Vulkan>,
    action_set: openxr::ActionSet,
    // Kept alive alongside `aim_space` (the action that created it must
    // outlive the space); not read directly today, only through the
    // space it created.
    #[allow(dead_code)]
    aim_pose: openxr::Action<openxr::Posef>,
    grip_pose: openxr::Action<openxr::Posef>,
    trigger: openxr::Action<f32>,
    squeeze: openxr::Action<f32>,
    thumbstick: openxr::Action<openxr::Vector2f>,
    a_click: openxr::Action<bool>,
    b_click: openxr::Action<bool>,
    haptic: openxr::Action<openxr::Haptic>,
    hand_paths: [openxr::Path; 2],
    aim_space: [openxr::Space; 2],
    grip_space: [openxr::Space; 2],
}

impl OpenXrHands {
    /// Create the action set, suggest both profiles' bindings from
    /// [`BINDINGS`], attach it to the session, and create the aim/grip
    /// action spaces. Call once, right after `xr::init` succeeds.
    pub fn new(
        instance: &openxr::Instance,
        session: &openxr::Session<openxr::Vulkan>,
    ) -> openxr::Result<OpenXrHands> {
        let action_set = instance.create_action_set("flight", "Flight Controls", 0)?;
        let hand_paths = [
            instance.string_to_path("/user/hand/left")?,
            instance.string_to_path("/user/hand/right")?,
        ];
        let aim_pose =
            action_set.create_action::<openxr::Posef>("aim_pose", "Aim Pose", &hand_paths)?;
        let grip_pose =
            action_set.create_action::<openxr::Posef>("grip_pose", "Grip Pose", &hand_paths)?;
        let trigger = action_set.create_action::<f32>("trigger", "Trigger", &hand_paths)?;
        let squeeze = action_set.create_action::<f32>("squeeze", "Squeeze", &hand_paths)?;
        let thumbstick = action_set.create_action::<openxr::Vector2f>(
            "thumbstick",
            "Thumbstick",
            &hand_paths,
        )?;
        let a_click = action_set.create_action::<bool>("a_click", "A Button", &hand_paths)?;
        let b_click = action_set.create_action::<bool>("b_click", "B Button", &hand_paths)?;
        let haptic = action_set.create_action::<openxr::Haptic>("haptic", "Haptic", &hand_paths)?;

        let mut index_bindings: Vec<openxr::Binding> = Vec::new();
        let mut simple_bindings: Vec<openxr::Binding> = Vec::new();
        for hand in HANDS {
            for b in BINDINGS {
                let path = instance.string_to_path(&full_path(hand, b.index_path))?;
                index_bindings.push(match b.action {
                    "aim_pose" => openxr::Binding::new(&aim_pose, path),
                    "grip_pose" => openxr::Binding::new(&grip_pose, path),
                    "trigger" => openxr::Binding::new(&trigger, path),
                    "squeeze" => openxr::Binding::new(&squeeze, path),
                    "thumbstick" => openxr::Binding::new(&thumbstick, path),
                    "a_click" => openxr::Binding::new(&a_click, path),
                    "b_click" => openxr::Binding::new(&b_click, path),
                    "haptic" => openxr::Binding::new(&haptic, path),
                    _ => unreachable!("BINDINGS action names are fixed above"),
                });
                if let Some(simple) = b.simple_path {
                    let path = instance.string_to_path(&full_path(hand, simple))?;
                    simple_bindings.push(match b.action {
                        "aim_pose" => openxr::Binding::new(&aim_pose, path),
                        "grip_pose" => openxr::Binding::new(&grip_pose, path),
                        "trigger" => openxr::Binding::new(&trigger, path),
                        "haptic" => openxr::Binding::new(&haptic, path),
                        _ => unreachable!("only pose/trigger/haptic have a simple_path"),
                    });
                }
            }
        }
        let index_profile =
            instance.string_to_path("/interaction_profiles/valve/index_controller")?;
        instance.suggest_interaction_profile_bindings(index_profile, &index_bindings)?;
        let simple_profile =
            instance.string_to_path("/interaction_profiles/khr/simple_controller")?;
        instance.suggest_interaction_profile_bindings(simple_profile, &simple_bindings)?;

        // Attach before the session is ever begun (see the module doc):
        // this runs synchronously as part of Gpu construction, well
        // before the event loop's first `xr_begin_frame`.
        session.attach_action_sets(&[&action_set])?;

        let aim_space = [
            aim_pose.create_space(session, hand_paths[0], openxr::Posef::IDENTITY)?,
            aim_pose.create_space(session, hand_paths[1], openxr::Posef::IDENTITY)?,
        ];
        let grip_space = [
            grip_pose.create_space(session, hand_paths[0], openxr::Posef::IDENTITY)?,
            grip_pose.create_space(session, hand_paths[1], openxr::Posef::IDENTITY)?,
        ];

        Ok(OpenXrHands {
            session: session.clone(),
            action_set,
            aim_pose,
            grip_pose,
            trigger,
            squeeze,
            thumbstick,
            a_click,
            b_click,
            haptic,
            hand_paths,
            aim_space,
            grip_space,
        })
    }

    fn hand(&self, space: &openxr::Space, time: openxr::Time, i: usize) -> Option<HandPose> {
        let hand_path = self.hand_paths[i];
        if !self
            .grip_pose
            .is_active(&self.session, hand_path)
            .unwrap_or(false)
        {
            return None;
        }
        let located = |s: &openxr::Space| -> Option<(Quat, Vec3)> {
            let loc = s.locate(space, time).ok()?;
            let tracked = openxr::SpaceLocationFlags::POSITION_VALID
                | openxr::SpaceLocationFlags::ORIENTATION_VALID;
            if !loc.location_flags.contains(tracked) {
                return None;
            }
            Some(pose_from_openxr(loc.pose))
        };
        let grip = located(&self.grip_space[i])?;
        // The aim ray is nice-to-have for the laser; if it briefly isn't
        // located, the grip pose is a reasonable stand-in rather than
        // losing the hand entirely.
        let aim = located(&self.aim_space[i]).unwrap_or(grip);
        let f32_of = |a: &openxr::Action<f32>| {
            a.state(&self.session, hand_path)
                .map(|s| s.current_state)
                .unwrap_or(0.0)
        };
        let bool_of = |a: &openxr::Action<bool>| {
            a.state(&self.session, hand_path)
                .map(|s| s.current_state)
                .unwrap_or(false)
        };
        let thumbstick = self
            .thumbstick
            .state(&self.session, hand_path)
            .map(|s| (s.current_state.x, s.current_state.y))
            .unwrap_or((0.0, 0.0));
        Some(HandPose {
            aim,
            grip,
            trigger: f32_of(&self.trigger),
            squeeze: f32_of(&self.squeeze),
            thumbstick,
            a: bool_of(&self.a_click),
            b: bool_of(&self.b_click),
        })
    }
}

impl HandSource for OpenXrHands {
    fn sync(&self) -> openxr::Result<()> {
        self.session
            .sync_actions(&[openxr::ActiveActionSet::from(&self.action_set)])
    }

    fn hands(
        &mut self,
        locate: Option<(&openxr::Space, openxr::Time)>,
        _bench_t_s: f32,
    ) -> VrHands {
        let Some((space, time)) = locate else {
            return VrHands::default();
        };
        VrHands {
            left: self.hand(space, time, 0),
            right: self.hand(space, time, 1),
        }
    }

    fn pulse(&self, hand: usize, amplitude: f32, duration_s: f32) {
        let Some(&path) = self.hand_paths.get(hand.min(1)) else {
            return;
        };
        let event = openxr::HapticVibration::new()
            .amplitude(amplitude.clamp(0.0, 1.0))
            .frequency(openxr::FREQUENCY_UNSPECIFIED)
            .duration(openxr::Duration::from_nanos(
                (duration_s.max(0.0) * 1.0e9) as i64,
            ));
        if let Err(e) = self.haptic.apply_feedback(&self.session, path, &event) {
            log::warn!("VR: haptic pulse: {e}");
        }
    }
}

/// A deterministic synthetic pair of hands: no OpenXR session, no
/// runtime, no headset — [`synth_hands`] driven by `Game::started.
/// elapsed()`, so a bench (`FARFALL_BENCH=1 FARFALL_VR_HANDS=synth
/// FARFALL_VR_SCRIPT=...`) exercises this whole lane deaf and 4-up on
/// the desktop. Logs one line per [`Beat`] change per hand.
pub struct SynthHands {
    script: HandScript,
    last_beat: [Option<Beat>; 2],
}

impl SynthHands {
    pub fn new() -> Self {
        let script = std::env::var("FARFALL_VR_SCRIPT")
            .ok()
            .and_then(|v| HandScript::from_env_value(&v))
            .unwrap_or(HandScript::Idle);
        log::info!("VR hands: synthetic, script {script:?}");
        SynthHands {
            script,
            last_beat: [None; 2],
        }
    }
}

impl Default for SynthHands {
    fn default() -> Self {
        Self::new()
    }
}

impl HandSource for SynthHands {
    fn sync(&self) -> openxr::Result<()> {
        Ok(())
    }

    fn hands(
        &mut self,
        _locate: Option<(&openxr::Space, openxr::Time)>,
        bench_t_s: f32,
    ) -> VrHands {
        let (hands, beats) = synth_hands(self.script, bench_t_s);
        for (i, beat) in beats.into_iter().enumerate() {
            if self.last_beat[i] != Some(beat) {
                self.last_beat[i] = Some(beat);
                let hand_name = if i == 0 { "left" } else { "right" };
                log::info!("VR hands: {hand_name} {} t={bench_t_s:.1}s", beat.label());
            }
        }
        hands
    }

    fn pulse(&self, hand: usize, amplitude: f32, duration_s: f32) {
        let hand_name = if hand == 0 { "left" } else { "right" };
        log::info!(
            "VR hands: {hand_name} PULSE amplitude={amplitude:.2} duration={duration_s:.3}s (synthetic)"
        );
    }
}

#[cfg(test)]
mod pure_tests {
    use super::*;

    #[test]
    fn every_action_has_a_nonempty_name_and_a_well_formed_index_path() {
        assert_eq!(BINDINGS.len(), 8);
        for b in BINDINGS {
            assert!(!b.action.is_empty());
            assert!(
                b.index_path.starts_with("input/") || b.index_path.starts_with("output/"),
                "{}: {}",
                b.action,
                b.index_path
            );
        }
    }

    #[test]
    fn grip_aim_and_haptics_fall_back_to_the_simple_controller() {
        for name in ["grip_pose", "aim_pose", "haptic"] {
            let b = BINDINGS.iter().find(|b| b.action == name).unwrap();
            assert!(
                b.simple_path.is_some(),
                "{name} should bind on khr/simple_controller too"
            );
        }
    }

    #[test]
    fn trigger_falls_back_to_the_simple_controllers_select_click() {
        let b = BINDINGS.iter().find(|b| b.action == "trigger").unwrap();
        assert_eq!(b.simple_path, Some("input/select/click"));
    }

    #[test]
    fn analog_and_button_only_index_inputs_have_no_simple_fallback() {
        for name in ["squeeze", "thumbstick", "a_click", "b_click"] {
            let b = BINDINGS.iter().find(|b| b.action == name).unwrap();
            assert!(
                b.simple_path.is_none(),
                "{name} has no khr/simple_controller equivalent"
            );
        }
    }

    #[test]
    fn no_two_actions_share_a_name() {
        for (i, a) in BINDINGS.iter().enumerate() {
            for b in &BINDINGS[i + 1..] {
                assert_ne!(a.action, b.action);
            }
        }
    }

    #[test]
    fn full_path_prefixes_the_hand() {
        assert_eq!(
            full_path("left", "input/aim/pose"),
            "/user/hand/left/input/aim/pose"
        );
        assert_eq!(
            full_path("right", "output/haptic"),
            "/user/hand/right/output/haptic"
        );
    }

    #[test]
    fn identity_pose_converts_to_identity_quat_and_zero_position() {
        let (q, v) = pose_from_openxr(openxr::Posef::IDENTITY);
        assert!(q.angle_between(Quat::IDENTITY) < 1e-6);
        assert_eq!(v, Vec3::ZERO);
    }

    #[test]
    fn a_translated_yawed_pose_round_trips() {
        let yaw = Quat::from_rotation_y(0.4);
        let src = openxr::Posef {
            orientation: openxr::Quaternionf {
                x: yaw.x,
                y: yaw.y,
                z: yaw.z,
                w: yaw.w,
            },
            position: openxr::Vector3f {
                x: 1.0,
                y: 2.0,
                z: -3.0,
            },
        };
        let (q, v) = pose_from_openxr(src);
        assert!(q.angle_between(yaw) < 1e-6);
        assert_eq!(v, Vec3::new(1.0, 2.0, -3.0));
    }

    #[test]
    fn hands_var_wins_outright_over_vr_var() {
        assert_eq!(hands_mode_for(Some("synth"), Some("1")), HandsMode::Synth);
        assert_eq!(hands_mode_for(Some("real"), Some("synth")), HandsMode::Real);
    }

    #[test]
    fn a_synthetic_headset_defaults_hands_synthetic_too() {
        assert_eq!(hands_mode_for(None, Some("synth")), HandsMode::Synth);
    }

    #[test]
    fn otherwise_hands_default_real() {
        assert_eq!(hands_mode_for(None, Some("1")), HandsMode::Real);
        assert_eq!(hands_mode_for(None, None), HandsMode::Real);
    }

    #[test]
    fn a_head_only_script_name_is_idle_for_hands() {
        for name in ["still", "look", "lean", "nod", "spin", "", "unknown"] {
            assert_eq!(HandScript::from_env_value(name), None, "{name}");
        }
    }

    #[test]
    fn every_hand_script_name_round_trips() {
        for (name, script) in [
            ("idle", HandScript::Idle),
            ("reach-stick", HandScript::ReachStick),
            ("grab-stick-roll", HandScript::GrabStickRoll),
            ("throttle-push", HandScript::ThrottlePush),
            ("laser-menu", HandScript::LaserMenu),
        ] {
            assert_eq!(HandScript::from_env_value(name), Some(script));
        }
    }

    /// The harness greps a running bench's log for these four patterns
    /// verbatim; a label wording change that breaks one of them breaks
    /// the harness silently, so it is pinned here.
    #[test]
    fn beat_labels_match_the_harnesss_own_greps() {
        // "VR hands: right GRAB stick"
        assert_eq!(Beat::GrabStick.label(), "GRAB stick");
        // "VR hands: .*(REACH|reach).*stick"
        let reach_stick = Beat::ReachStick.label();
        assert!(reach_stick.contains("REACH") && reach_stick.contains("stick"));
        // "VR hands: .*(GRAB|PUSH|grab|push).*throttle"
        let grab_throttle = Beat::GrabThrottle.label();
        assert!(grab_throttle.contains("GRAB") && grab_throttle.contains("throttle"));
        let push_throttle = Beat::PushThrottle.label();
        assert!(push_throttle.contains("PUSH") && push_throttle.contains("throttle"));
        // "VR hands: .*(LASER|POINT|laser|point)"
        for beat in [Beat::ReachLaser, Beat::PointLaser, Beat::LaserPress] {
            let label = beat.label();
            assert!(
                label.contains("LASER")
                    || label.contains("POINT")
                    || label.contains("laser")
                    || label.contains("point"),
                "{label}"
            );
        }
    }

    /// Every non-laser script's aim points at the console, not the
    /// glass — a grab-focused script must never spuriously show the
    /// laser beam (SPEC §5.3b(c)) just because `pose_at`'s aim happened
    /// to line up with the virtual glass 1m ahead.
    #[test]
    fn non_laser_scripts_aim_away_from_the_virtual_glass() {
        for script in [
            HandScript::Idle,
            HandScript::ReachStick,
            HandScript::GrabStickRoll,
            HandScript::ThrottlePush,
        ] {
            let (hands, _) = synth_hands(script, 3.0);
            for hand in [hands.left, hands.right] {
                let Some(h) = hand else { continue };
                let dir = h.aim.0 * Vec3::NEG_Z;
                let hit = crate::xr_laser::ray_hits_glass(
                    h.aim.1,
                    dir,
                    Vec3::ZERO,
                    Quat::IDENTITY,
                    0.7,
                    1.5,
                );
                assert!(hit.is_none(), "{script:?} hand aims at the glass: {dir:?}");
            }
        }
    }

    #[test]
    fn idle_keeps_both_hands_motionless_on_the_lap() {
        let (a, beats) = synth_hands(HandScript::Idle, 0.0);
        let (b, _) = synth_hands(HandScript::Idle, 50.0);
        assert_eq!(beats, [Beat::Idle, Beat::Idle]);
        assert_eq!(a.left.unwrap().grip.1, LAP_LEFT);
        assert_eq!(a.right.unwrap().grip.1, LAP_RIGHT);
        assert_eq!(a.left.unwrap().grip.1, b.left.unwrap().grip.1, "motionless");
        assert_eq!(a.left.unwrap().squeeze, 0.0);
    }

    #[test]
    fn reach_stick_travels_from_the_lap_and_then_holds_without_grabbing() {
        let (start, beat0) = synth_hands(HandScript::ReachStick, 0.0);
        assert_eq!(beat0[1], Beat::ReachStick);
        assert_eq!(start.right.unwrap().grip.1, LAP_RIGHT);
        let (mid, beat_mid) = synth_hands(HandScript::ReachStick, REACH_S * 0.5);
        assert_eq!(beat_mid[1], Beat::ReachStick);
        let d0 = (start.right.unwrap().grip.1 - crate::xr_grab::STICK_REST).length();
        let dm = (mid.right.unwrap().grip.1 - crate::xr_grab::STICK_REST).length();
        assert!(dm < d0, "still closing on the target");
        let (arrived, beat_end) = synth_hands(HandScript::ReachStick, REACH_S + 1.0);
        assert_eq!(beat_end[1], Beat::HoldStick);
        assert_eq!(arrived.right.unwrap().grip.1, crate::xr_grab::STICK_REST);
        assert_eq!(arrived.right.unwrap().squeeze, 0.0, "holding, not grabbing");
    }

    #[test]
    fn grab_stick_roll_grabs_then_oscillates_the_roll_axis() {
        let (before, beat) = synth_hands(HandScript::GrabStickRoll, REACH_S + GRAB_RAMP_S * 0.5);
        assert_eq!(beat[1], Beat::GrabStick);
        assert!(before.right.unwrap().squeeze > 0.0 && before.right.unwrap().squeeze < 1.0);
        let (rolling_a, beat_a) =
            synth_hands(HandScript::GrabStickRoll, REACH_S + GRAB_RAMP_S + 0.5);
        assert_eq!(beat_a[1], Beat::RollStick);
        let (rolling_b, _) = synth_hands(HandScript::GrabStickRoll, REACH_S + GRAB_RAMP_S + 1.5);
        assert!(
            rolling_a.right.unwrap().grip.1.x != rolling_b.right.unwrap().grip.1.x,
            "the grip's own x oscillates once rolling"
        );
        assert!(rolling_a.right.unwrap().squeeze > 0.6, "still gripped");
    }

    #[test]
    fn throttle_push_ramps_from_zero_toward_eighty_percent() {
        let (grabbed, beat0) = synth_hands(
            HandScript::ThrottlePush,
            THROTTLE_REACH_S + GRAB_RAMP_S + 0.01,
        );
        assert_eq!(beat0[0], Beat::PushThrottle);
        let start_z = grabbed.left.unwrap().grip.1.z;
        assert!(
            (start_z - crate::xr_grab::THROTTLE_REST.z).abs() < 1e-3,
            "no push yet: {start_z}"
        );
        let (pushed, _) = synth_hands(
            HandScript::ThrottlePush,
            THROTTLE_REACH_S + GRAB_RAMP_S + THROTTLE_PUSH_S,
        );
        let end_z = pushed.left.unwrap().grip.1.z;
        assert!(
            end_z < crate::xr_grab::THROTTLE_REST.z,
            "pushed forward (−Z): {end_z}"
        );
    }

    #[test]
    fn laser_menu_sweeps_then_presses_once() {
        let (aiming, beat) = synth_hands(HandScript::LaserMenu, AIM_REACH_S + 1.0);
        assert_eq!(beat[1], Beat::PointLaser);
        assert_eq!(aiming.right.unwrap().trigger, 0.0);
        let (pressing, beat_p) = synth_hands(HandScript::LaserMenu, AIM_PRESS_AT_S + 0.05);
        assert_eq!(beat_p[1], Beat::LaserPress);
        assert_eq!(pressing.right.unwrap().trigger, 1.0);
        let (after, beat_after) = synth_hands(
            HandScript::LaserMenu,
            AIM_PRESS_AT_S + AIM_PRESS_DUR_S + 0.1,
        );
        assert_eq!(beat_after[1], Beat::PointLaser, "the press is momentary");
        assert_eq!(after.right.unwrap().trigger, 0.0);
    }
}
