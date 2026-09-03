//! Valve Index controller input (fable/vr-hands): a single "flight"
//! action set — aim pose, grip pose, trigger value, squeeze value,
//! thumbstick, A/B click, haptic output, per hand — suggested for
//! `/interaction_profiles/valve/index_controller` with a
//! `/interaction_profiles/khr/simple_controller` fallback for whatever
//! that profile actually has (grip/aim pose, a boolean "select" mapped
//! onto the trigger action via OpenXR's own click→float conversion, and
//! haptics). Shape ported from openxrs' example (Ralith/openxrs,
//! MIT/Apache-2.0, `openxr/examples/vulkan.rs`) — see
//! `docs/RESEARCH-VR-OSS.md` §1.
//!
//! [`XrInput::new`] must run once, right after [`crate::xr::init`]
//! succeeds and before the event loop ever calls `begin_frame` — OpenXR
//! requires every action set be attached
//! (`Session::attach_action_sets`) before the session leaves its
//! unattached state, and the session does not reach `READY` (the point
//! `xr::XrSession::begin_frame` first calls `session.begin`) until some
//! polling has happened. [`XrInput::sync`] and [`XrInput::hands`] are
//! called every frame from `xr_begin_frame` in `lib.rs`, right beside
//! where `game.vr` itself is set from the eyes — hands and eyes are the
//! same seam, located in the same recentred LOCAL space.

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

// ---------------------------------------------------------------------
// The runtime: everything below touches a real OpenXR session and
// cannot run in a unit test; exercised by `cargo check` and, eventually,
// a headset — the same split `xr.rs` itself makes.
// ---------------------------------------------------------------------

/// The attached action set plus the per-hand action spaces it created —
/// the OpenXR side of hand input, alongside `xr::XrSession`'s eyes.
pub struct XrInput {
    action_set: openxr::ActionSet,
    // Kept alive alongside `aim_space` (the action that created it must
    // outlive the space); not read directly today, only through the
    // space it created. `#[allow]`ed rather than dropped: a later commit
    // in this same sequence (SPEC §5.3b) reads `is_active` off it the
    // same way `grip_pose` is read below.
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

impl XrInput {
    /// Create the action set, suggest both profiles' bindings from
    /// [`BINDINGS`], attach it to the session, and create the aim/grip
    /// action spaces. Call once, right after `xr::init` succeeds.
    pub fn new(
        instance: &openxr::Instance,
        session: &openxr::Session<openxr::Vulkan>,
    ) -> openxr::Result<XrInput> {
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

        Ok(XrInput {
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

    /// Sync this frame's action states. Call once a frame, before
    /// [`Self::hands`] or any `.state()` read.
    pub fn sync(&self, session: &openxr::Session<openxr::Vulkan>) -> openxr::Result<()> {
        session.sync_actions(&[openxr::ActiveActionSet::from(&self.action_set)])
    }

    /// Both hands' controller state this frame, located in `space` (the
    /// session's current, recentred LOCAL space — the ship's frame) at
    /// `time` (the frame's predicted display time). A hand with no
    /// tracked grip pose (asleep, out of view, unbound) is `None`.
    pub fn hands(
        &self,
        session: &openxr::Session<openxr::Vulkan>,
        space: &openxr::Space,
        time: openxr::Time,
    ) -> VrHands {
        VrHands {
            left: self.hand(session, space, time, 0),
            right: self.hand(session, space, time, 1),
        }
    }

    fn hand(
        &self,
        session: &openxr::Session<openxr::Vulkan>,
        space: &openxr::Space,
        time: openxr::Time,
        i: usize,
    ) -> Option<HandPose> {
        let hand_path = self.hand_paths[i];
        if !self
            .grip_pose
            .is_active(session, hand_path)
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
            a.state(session, hand_path)
                .map(|s| s.current_state)
                .unwrap_or(0.0)
        };
        let bool_of = |a: &openxr::Action<bool>| {
            a.state(session, hand_path)
                .map(|s| s.current_state)
                .unwrap_or(false)
        };
        let thumbstick = self
            .thumbstick
            .state(session, hand_path)
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

    /// A short haptic pulse on one hand (0 left, 1 right). `amplitude`
    /// 0..1, `duration_s` seconds. Logs and drops the error rather than
    /// panicking — a haptic is confirmatory, never load-bearing. Called
    /// from `lib.rs::pulse_hand` on a grab, a release, and a laser
    /// click (SPEC §5.3b(e)).
    pub fn pulse(
        &self,
        session: &openxr::Session<openxr::Vulkan>,
        hand: usize,
        amplitude: f32,
        duration_s: f32,
    ) {
        let Some(&path) = self.hand_paths.get(hand.min(1)) else {
            return;
        };
        let event = openxr::HapticVibration::new()
            .amplitude(amplitude.clamp(0.0, 1.0))
            .frequency(openxr::FREQUENCY_UNSPECIFIED)
            .duration(openxr::Duration::from_nanos(
                (duration_s.max(0.0) * 1.0e9) as i64,
            ));
        if let Err(e) = self.haptic.apply_feedback(session, path, &event) {
            log::warn!("VR: haptic pulse: {e}");
        }
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
}
