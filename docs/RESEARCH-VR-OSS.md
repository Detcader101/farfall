# FARFALL VR: OpenXR Input & Interaction Research

**Date:** 2026-09-03  
**Purpose:** Find MIT/Apache-licensed Rust VR code for native OpenXR on branch `fable/vr`  
**Scope:** Controller input (Valve Index), hand tracking, 6-DoF locomotion, interaction patterns

---

## 1. OpenXR Input & Controller Code

| Repo | License | Language | What to Take | How |
|------|---------|----------|--------------|-----|
| **openxrs** / [Ralith/openxrs](https://github.com/Ralith/openxrs) | MIT/Apache-2.0 | Rust | `openxr` crate 0.21+: action sets, hand tracking, grip/aim poses, haptics | **Copy/Port**: `openxr/examples/vulkan.rs` shows action set creation (`xr.create_action_set()`) and binding profiles. Extract action loop into `xr_input.rs` binding Valve Index profiles at `/interaction_profiles/valve/index_controller` for `trigger`, `squeeze`, `thumbstick`, `trackpad`, `a/b` buttons, grip/aim poses, haptics. Compiles against 0.21. |
| **bevy_oxr** / [awtterpip/bevy_oxr](https://github.com/awtterpip/bevy_oxr) | MIT/Apache-2.0 | Rust | Bevy integration pattern (skip if direct wgpu), action event system | Skip for FARFALL (Bevy overhead); read for event patterns only. |
| **wgpu-openxr-example** / [philpax/wgpu-openxr-example](https://github.com/philpax/wgpu-openxr-example) | MIT | Rust | wgpu + OpenXR render loop, swapchain binding, multi-view rendering | **Port**: render-to-texture loop (eye crop, per-swapchain output). Vulkan-only; adapt wgpu path. |
| **wgpu-example** / [matthewjberger/wgpu-example](https://github.com/matthewjberger/wgpu-example) | MIT/Apache-2.0 (implied; dual-licensed Rust project) | Rust | Full OpenXR desktop + Android VR, hand tracking via OpenXR, native builds | **Pattern**: Android build recipe, hand tracking initialization. Hand data already in `HandJointLocations`. |
| **OpenXR-SDK-Source / hello_xr** / [KhronosGroup/OpenXR-SDK-Source](https://github.com/KhronosGroup/OpenXR-SDK-Source/tree/main/src/tests/hello_xr) | Apache-2.0 | C++ | Session creation, action set binding, interaction profile selection | **Pattern only** (C++ → Rust): shows `xrSuggestInteractionProfileBindings()` state machine. Port conceptually. |

---

## 2. Hand Tracking & 6-DoF Comfort Code

| Repo | License | Language | Pattern | Notes |
|------|---------|----------|---------|-------|
| **Godot XR Tools** / [GodotVR/godot-xr-tools](https://github.com/GodotVR/godot-xr-tools) | MIT | GDScript | Hand controller abstraction, grip/aim hand models, comfort vignette, snap/smooth turn | **Pattern only** (GDScript). Read `addons/godot-xr-tools/` for state machine design: hand pose tracking, controller hand mesh swap, vignette fade on rotation. |
| **Godot XR Hand Pose Detector** / [Malcolmnixon/GodotXRHandPoseDetector](https://github.com/Malcolmnixon/GodotXRHandPoseDetector) | MIT | GDScript | Hand pose classification (pinch, point, grab) from joint data | **Pattern**: pinch/point detection threshold logic translatable to Rust. |
| **StereoKit-rs** / [MalekiRe/stereokit-rs](https://github.com/MalekiRe/stereokit-rs) | MIT (StereoKit C) | Rust bindings | Hand-first input (hands simulated if unavailable), laser pointer, button interaction | **Pattern only**: shows hand as first-class input (not fallback). |
| **Aether** / [stellan-lee/Aether](https://github.com/stellan-lee/Aether) (rhoninl mirror) | Apache-2.0 | Rust | Hand tracking via `aether-openxr`, comfort settings (recentre) | **Pattern**: modular VR plugin structure. |

---

## 3. Hand & Interaction Code (Grab, Point, Press)

| Repo | License | Language | Implementation | Effort to Port |
|------|---------|----------|-----------------|-----------------|
| **Hotham** / [leetvr/hotham](https://github.com/leetvr/hotham) | Apache-2.0/MIT | Rust | Object grabbing (grab-to-throttle pattern), hand/controller presence detection, haptics | **HIGH**: examples/ has full grab interaction state machine. Port `examples/grab_object.rs` logic into `xr_hand_grab.rs` for virtual stick/throttle. Haptic feedback already wired. |
| **elite-vr-cockpit** / [dantman/elite-vr-cockpit](https://github.com/dantman/elite-vr-cockpit) | (check LICENSE) | C#/C++ | SteamVR overlay: virtual throttle and joystick grab with pose-driven transform, button press zones | **Pattern**: throttle/stick transforms from hand pose (grip position → throttle position interpolation), pressure-zone detection. C# but logic is translatable. |
| **VRTK-Interaction-Component** / [Innoactive/VRTK-Interaction-Component](https://github.com/Innoactive/VRTK-Interaction-Component) | (check LICENSE) | C# | Laser pointer, button press, grab detection | **Pattern only** (C#): laser raycast + UI interaction model. |

**State Machine Pattern (Grab):**
1. Hand within grip radius (`Vector3::distance(hand, throttle) < 0.1`)
2. Squeeze > threshold (60%) → enter grab mode
3. While grabbed: transform throttle to hand position
4. Squeeze < 40% → release (hysteresis)
5. Haptic pulse on grab/release

---

## 4. Full VR Games (Study Structure)

| Repo | License | Language | What It Does Well | Architecture |
|------|---------|----------|-------------------|---------------|
| **LÖVR** / [bjornbytes/lovr](https://github.com/bjornbytes/lovr) | MIT | Lua (C11 engine) | Beginner-friendly VR, fast single-pass stereo, physics, audio spatialization, input abstraction | Input loop trivial; good for understanding VR lifecycle, not Rust-native. |
| **Aether** / [stellan-lee/Aether](https://github.com/stellan-lee/Aether) | Apache-2.0 | Rust | Modular 32-crate workspace (Core, Platform, Social, Safety), XR plugin architecture | Study module boundaries: `aether-openxr` as separate crate. |
| **Hotham** / [leetvr/hotham](https://github.com/leetvr/hotham) | Apache-2.0/MIT | Rust | Standalone VR game toolkit, complete grab+haptic loop, hand presence | **Best single-repo study**. Full example game in `examples/`. Hand tracking already integrated. |
| **Ambient** / [AmbientRun/Ambient](https://github.com/AmbientRun/Ambient) | Apache-2.0 | Rust | Multiplayer game engine, real-time networked ECS, modular design | Study architecture, not VR-specific; networking overkill for cockpit. |
| **Nightshade** / [matthewjberger/nightshade](https://github.com/matthewjberger/nightshade) | MIT/Apache-2.0 | Rust | Data-oriented ECS, `nightshade-openxr` crate integrates PC VR | Study modular crate design; similar philosophy to FARFALL. |

---

## 5. Recommended MVP for FARFALL Controllers

**Priority order** (bottom-up integration):

### **(a) Knuckles Action Bindings + Aim/Grip Poses** — **1 agent-session**

**What:** Render controller geometry (or SDF hands in cabin) driven by `aim` and `grip` poses; bind Index controller paths.

**Source:** `openxrs/examples/vulkan.rs` + `openxr` crate docs ([docs.rs/openxr](https://docs.rs/openxr))

**Exact integration:**
```rust
// crates/app/src/xr_input.rs (NEW)
// - Create action set: xr.create_action_set("index_controls", ...)?
// - Bind paths:
//   - /user/hand/left/input/grip/pose
//   - /user/hand/right/input/grip/pose
//   - /user/hand/left/input/aim/pose
//   - /user/hand/right/input/aim/pose
//   - /user/hand/*/input/trigger/value
//   - /user/hand/*/input/squeeze/value
//   - /interaction_profiles/valve/index_controller
// - Each frame: get_hand_pose() → cam transform, pass to cabin shader
// - Render as: existing SDF hand model or simple cone/capsule geometry
```

**Effort:** ~2–3 hours (openxr loop + pose binding)

**Why first:** Validates OpenXR binding pipeline; arms all downstream layers.

---

### **(b) Laser Point-and-Click (Menu Driver)** — **1 session**

**What:** Raycast from `aim/pose`, intersect UI quads, trigger existing menu callbacks on squeeze.

**Source:** Godot XR Tools pattern (study only; same concept). `openxrs/examples/vulkan.rs` has pose/space queries.

**Integration into existing code:**
```rust
// crates/app/src/xr_input.rs (extend a)
// - Each frame: raycast(aim_pose.position, aim_pose.forward) against menu::quads()
// - On squeeze rise: call menu.on_click(ray_hit.id)
// - Visual: thin line from hand to intersection (existing shader)
```

**Effort:** ~2 hours (ray–AABB test + menu event plumbing)

**Why second:** Unblocks menu use in VR; no hand/throttle complexity yet.

---

### **(c) Virtual Stick+Throttle Grab** — **2 sessions**

**What:** Grab-and-drag cabin-mounted stick/throttle using hand position; drives existing `Controls` struct.

**Source:** Hotham examples + elite-vr-cockpit pattern (both study; C# but state machine translatable).

**Implementation:**
```rust
// crates/app/src/xr_hand_grab.rs (NEW)
pub struct VirtualStick {
    rest_position: Vec3,
    grab_active: bool,
    last_hand_pos: Vec3,
}

impl VirtualStick {
    pub fn update(&mut self, hand: Hand, controls: &mut Controls) {
        let squeeze = hand.squeeze; // 0..1
        if squeeze > 0.6 && !self.grab_active {
            self.grab_active = true;
            self.last_hand_pos = hand.grip_pose.position;
        }
        if self.grab_active {
            let delta = hand.grip_pose.position - self.last_hand_pos;
            controls.pitch += delta.y * 5.0; // sensitivity tune
            controls.roll += delta.x * 5.0;
            self.last_hand_pos = hand.grip_pose.position;
            
            if squeeze < 0.4 {
                self.grab_active = false;
            }
        }
    }
}
```

**Effort:** ~3–4 hours (grab state machine + Controls mapping)

**Why third:** Core flight control; everything after this adds polish.

---

### **(d) Haptics** — **0.5 sessions**

**What:** Pulse on grab/release, click feedback on menu press.

**Source:** `openxr` crate `Haptic` action type; Hotham has full integration already.

**Integration:**
```rust
// In (a) extend: 
// - Create `haptic_action = xr.create_action("haptics", ...)?`
// - Bind: /user/hand/*/output/haptic
// In (c): haptic_action.apply_feedback(grip_hand, intensity=0.5)?
```

**Effort:** ~1 hour (mostly wiring existing OpenXR haptic API)

**Why fourth:** Confirmatory feedback; no blocking dependencies.

---

### **(e) Hand Tracking (Full Skeleton)** — **1.5 sessions**

**What:** Render full hand skeleton (joints) instead of controller models; pinch/point gesture detection.

**Source:** Godot XR Hand Pose Detector (pattern) + `openxr::HandTracker` docs.

**Integration:**
```rust
// crates/app/src/xr_hand_tracker.rs (NEW)
// - Enable ext_hand_tracking on session creation
// - Create: hand_tracker = session.create_hand_tracker(hand)?
// - Each frame: hand_tracker.get_hand_joint_locations(space, time)?
// - Render: joint positions as spheres or line segments (SDF or mesh)
// - Gesture: pinch_distance(thumb, index) < 0.02 → trigger menu action
```

**Effort:** ~2 hours (hand tracker init + joint render loop + pinch threshold)

**Why last:** Polish feature; grabbing already works with controllers. Hand tracking is fallback/accessibility.

---

## 6. License Verification Summary

✅ **MIT/Apache-2.0 (APPROVED):**
- openxrs (MIT/Apache-2.0)
- wgpu-openxr-example (MIT)
- wgpu-example (MIT/Apache-2.0 implied; standard Rust dual license)
- bevy_oxr (MIT/Apache-2.0)
- Hotham (Apache-2.0/MIT)
- Aether (Apache-2.0)
- Nightshade (MIT/Apache-2.0)
- LÖVR (MIT)
- Godot XR Tools (MIT, code; CC0, assets)
- Godot XR Hand Pose Detector (MIT)
- OpenXR-SDK-Source (Apache-2.0)

✅ **Pattern-Only (GPL/Proprietary reads):**
- elite-vr-cockpit (verify in repo; patterns only)
- VRTK-Interaction-Component (verify; patterns only)

---

## 7. Key References

**OpenXR Rust Bindings:**
- Crate: [docs.rs/openxr/0.21](https://docs.rs/openxr/0.21/) (latest stable for 0.21)
- Example: [github.com/Ralith/openxrs/openxr/examples/vulkan.rs](https://github.com/Ralith/openxrs/blob/master/openxr/examples/vulkan.rs)
- Valve Index profile: `/interaction_profiles/valve/index_controller` (OpenXR spec)

**Interaction Profiles (Valve Index via OpenXR):**
- Grip pose: `/user/hand/{left,right}/input/grip/pose`
- Aim pose: `/user/hand/{left,right}/input/aim/pose`
- Trigger: `/user/hand/{left,right}/input/trigger/value`
- Squeeze: `/user/hand/{left,right}/input/squeeze/value`
- Thumbstick: `/user/hand/{left,right}/input/thumbstick/{x,y}`
- Trackpad: `/user/hand/{left,right}/input/trackpad/{x,y}` (if supported)
- Buttons: `/user/hand/{left,right}/input/{a,b}`
- Haptic: `/user/hand/{left,right}/output/haptic`

**Hand Tracking Extension:**
- `XR_EXT_hand_tracking` (enable on instance)
- Requires OpenXR 1.0+, Valve Index Index controller on SteamVR runtime

**Cabin-as-Comfort-Frame Pattern:**
- Keep camera in cockpit reference frame (never rotate world, rotate cabin around camera)
- Snap turn: discrete 45° rotation; vignette fade in/out
- Smooth turn: gradual rotation with reduced vignette

**Flutter-Free Testing:**
- Start with single-controller (left OR right), then both
- Test grab hysteresis (0.6 grab, 0.4 release) to avoid flutter

---

## 8. Next Steps

1. **Week 1:** Land (a) Knuckles bindings; validate pose transforms in cabin.
2. **Week 1:** Land (b) Laser pointer; confirm menu click wiring.
3. **Week 2:** Land (c) Virtual stick grab; map to Controls.
4. **Week 2:** Land (d) Haptics; add pulse on events.
5. **Week 3:** Polish: (e) Hand tracking + pinch detection (if time).

**Commit sequence:**
- `xr-input: bind Valve Index controller profiles`
- `xr-input: add laser pointer for menu drive`
- `xr-hand-grab: virtual stick+throttle grab loop`
- `xr-haptics: add pulse feedback`
- `xr-hand-tracker: optional full skeleton rendering`

---

**Document ID:** VR-OSS-RESEARCH.md | **Last Updated:** 2026-09-03 | **Status:** Ready for implementation
