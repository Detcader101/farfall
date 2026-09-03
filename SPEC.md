# FARFALL — Living Specification

*Working title. A shader-driven space game: Starfox clarity at planetary scale.*
*Status: v0.2, 2026-08-19. Supersedes docs/RESEARCH-2026-08.md where they conflict (deltas at the bottom).*

---

## 1. Vision

You fly a small ship in near-future Sol space. The world is rendered with deliberately
simple geometry — clean silhouettes, flat and stylized shading, the readability of a
90s rail shooter — while the GPU budget freed by that simplicity is spent on what
cheap geometry can't fake: real distances, real atmospheres, a planet that resolves
from a point of light into a place. Earth is visitable: compact in scale, generic but
recognizable in its landmarks, and eventually landable — the long-term destination is
setting down in a cyberpunk city whose crowds, shops, and traffic are drawn by shader
programs, not asset libraries.

The tone borrows from Dark Souls in ethos rather than mechanics: a quiet, oblique
world that explains little, controls with weight and consequence, and rewards
attention. It borrows from Elite in ambition: choices that alter a simulated world —
but that layer comes after the slice, not before.

This is not a tech demo. Every milestone produces a playable artifact, every system
is built test-first so that any future contributor — human, AI-assisted, or neither —
can verify their change didn't break physics, determinism, or the frame budget.

## 2. Pillars

Each pillar is enforceable. If a proposed change can't satisfy the "verified by"
column, it violates the pillar and needs a spec change, not a quiet exception.

| # | Pillar | Meaning | Verified by |
|---|--------|---------|-------------|
| P1 | **Readability over richness** | Crisp edges, stable image, high contrast at speed. Forward rendering + MSAA. **No TAA as a dependency, no motion blur by default, no upscaler smear.** Effects requiring temporal accumulation are confined to low-frequency buffers (volumetrics) and must degrade to off, never to blur. | Golden-image tests diff at full res; any pass that only works "under TAA" is rejected in review |
| P2 | **Shaders carry the detail** | Geometry stays simple and cheap; distance, atmosphere, lighting, surfaces, and eventually city life come from GPU programs. Asset-light by policy. | Repo asset budget (MVP: < 5 MB of non-code assets); new meshes need justification |
| P3 | **Scale is real, math is generic** | Floating origin, f64 sim / camera-relative f32 render. All physical models parameterized — the same equations run a compact Earth or a real one. | Scale-invariance property tests; jitter golden test at extreme origin offsets |
| P4 | **Runs on anything, uses everything** | Quality floor is WebGL2-class hardware (no compute); ceiling scales to whatever the GPU offers. Graphics options are per-pass, first-class, and honest. | Two-lane rule (§6.4); every feature declares Lane A fallback or an off switch; tier matrix in CI build flags |
| P5 | **Deterministic, headless, authoritative sim** | The simulation runs with no renderer, at a fixed timestep, bit-identical across platforms and runs. The renderer is never authoritative. This is what makes Mac/Windows/VR cross-play possible later. | Determinism + golden-hash tests run on macOS-arm64 AND linux-x86_64 in CI and must produce identical hashes |
| P6 | **Test-driven, contributor-proof** | Every crate ships tests that define its contract. A contributor with no context (and no AI) can run `cargo test` and know if they broke the game. | CI gates: fmt, clippy -D warnings, tests on 2 platforms, shader static validation, license audit |

## 3. Non-goals (MVP era, M0–M3)

- No NPCs, no economy, no narrative system. (Vision-level only; see §9.) Combat came
  early after all: the arms lane (WEAPONS.md) — guns, rocks that break, ship damage
  later — lives in the app beside the belt, outside the sim's hash.
- No multiplayer netcode — only the architectural seam for it (P5).
- No VR implementation — only the `XrBackend` seam (§5.3) and a scheduled spike (M4).
- No landing/touchdown — the slice ends at a low-altitude flyover (§4).
- No ECS — world state is plain data (§7.1); revisit when entity count demands it.
- No public release — private repo, invited collaborators.

## 4. The vertical slice (what M3 ships)

**"First Descent."** ~15–20 minutes. You wake in a high orbit above a compact Earth,
in silence, with a minimal HUD. You learn the weighty 6DOF flight model by doing.
Objectives are environmental, not textual: a decaying orbit you must correct, a
descent corridor marked by light. You take the ship from black-sky orbit through
atmosphere interface — plasma sheath, sky color rising from black through violet to
blue, stars washing out — down to a few km over a stylized but recognizable coastline
(day/night terminator crossing it, city lights on the dark side), and pull up over
a landmark. Fade. Save exists, graphics options exist, it runs at target framerate
on the tier matrix.

Done means: a person who has never seen the project plays it start to finish on a
MacBook M1 native and in Chrome via wasm, without instructions, at ≥ 60 fps (native,
1440p, tier "high") / ≥ 60 fps (web, 1080p, tier "medium").

## 5. Architecture

### 5.1 Crates

```
crates/
  sim      farfall-sim      Pure Rust. World state, flight model, gravity,
                            atmosphere, fixed-step integrator, state hash.
                            NO gpu/window/asset deps. Compiles to wasm32 trivially.
  render   farfall-render   wgpu 30. Frame passes, cameras, shader loading,
                            quality tiers. Knows nothing about winit or input.
  app      farfall-app      winit shell: window, input, frame loop, sim<->render
                            wiring. Native entry point. (wasm entry: M2.)
shaders/                    All WGSL. Statically validated by tests (naga).
```

Dependency rule: `app → {render, sim}`, `render → (nothing of ours)`, `sim → (nothing of ours)`.
`render` never imports `sim`; the app translates sim state into render-facing structs
(camera pose, body positions). This keeps the sim headless (P5) and lets the renderer
be replaced (or run twice — flat + XR) without touching physics.

### 5.2 Authority rule (the cross-play constraint)

The sim is the only authority on world state. Input → sim → snapshot → renderer.
When multiplayer arrives, "input" becomes "inputs from N players via a server" and
nothing else changes shape. Two consequences now:
- Sim state must be serializable and hashable from day one (it is: §7.4).
- The frame loop treats the renderer as a *view* driven by interpolated snapshots,
  never as a place where gameplay state lives.

### 5.3 The XR seam

As shipped, the seam is a data type, not a trait: [`app::VrEye`] (an eye's
orientation, its seat in the ship's frame, and its frustum's four tangents
— left, right, up, down, all positive) and `Option<VrView>` on `Game`.
When it is `None` the whole app is exactly the flat game it always was;
when it is `Some`, `Game::pose()` turns the active eye into a *symmetric*
camera wide enough to hold its true asymmetric frustum
(`VrEye::symmetric`), and `Game::head()` returns the eye's orientation
with no freelook and no helmet-camera shake — a headset *is* the head.
Everything downstream of `pose()`/`head()` (every world pass, the cabin,
the sight) is unchanged by which kind of session set `game.vr`; the two
XR backends differ only in how they populate it and in how they get the
finished stereo pair back out to a display:

- **WebXR** (`crates/app/src/web.rs::xr_frame`, `web/xr.js`): the page
  owns the session and drives the whole frame loop from JavaScript. Each
  browser frame it hands `xr_frame` both eyes' pose and tangents (11
  floats each, straight off `XRView`), resizes the canvas to the pair's
  width, and lets `redraw` render each eye — full width, symmetric FOV —
  into its own half of that canvas. A small WebGL2 compositor
  (`web/xr.js`) then reads the canvas back as a texture and, for each
  eye, samples a UV rectangle computed from the tangents
  (`tangents()` + the quad it draws) — the true asymmetric field cropped
  back out of the wider symmetric render — into the headset's real
  framebuffer. The browser's compositor is the crop; FARFALL's own code
  never has to know it exists.
- **Native OpenXR** (`crates/app/src/xr.rs`, Windows/SteamVR today):
  there is no browser to crop for us, so this module does it as a real
  GPU pass. `xr::init` is a start-up choice (VR HEADSET / `FARFALL_VR`):
  it stands up an OpenXR session whose Vulkan device it also hands to
  wgpu, through wgpu-hal's `from_raw`/`expose_adapter`/`device_from_raw`
  — the runtime must approve the device before wgpu ever touches it, not
  the other way round — and falls back to the flat renderer, logging
  why, on any failure. Each frame `xr::XrSession::begin_frame` waits for
  and opens the runtime's frame, locates both eyes in a `LOCAL` space
  (gravity-level, seated at session start; OpenXR's own +X right/+Y
  up/−Z forward is the ship's frame exactly, so no fix-up rotation is
  needed), and hands back `game.vr` before `game.tick()` runs — the
  predicted pose drives the frame it was predicted for. `redraw` renders
  the pair into an offscreen texture exactly as the WebXR path renders
  it into the canvas, then a small pass (`shaders/xrblit.wgsl`,
  `crates/render/src/blit_xr.rs`) crops each eye's true field out of it
  — the identical UV-rectangle maths as `xr.js`'s compositor, as a pure,
  tested Rust function (`xr::cutout_uv`) — into that eye's OpenXR
  swapchain image, and a plain aspect-fit crop of the left eye mirrors
  into the actual window (present mode forced to `NoVsync`, so a 60 Hz
  monitor never paces a 90 Hz headset). VR RECENTRE (default key HOME)
  re-seats the `LOCAL` space on the current head's yaw and position —
  pitch and roll are never touched, since the space is already
  gravity-level and a recentre must not tilt the floor
  (`xr::recentre_pose`).

Everything above `game.vr`/`VrEye` — the sim, the render passes, the
cockpit, the HUD — was written once and needs no XR-specific branch;
getting *this* seam wrong (letting VR-specific state leak past `pose()`
and `head()` into gameplay code) would have been the expensive mistake,
and both backends were built to avoid it.

### 5.3b Hands and interactions

Built on branch `fable/vr-hands` (off `fable/vr`, native OpenXR only —
WebXR has no controller input in this codebase and none is added here).
Four seams, each following §5.3's own pattern of a data type on `Game`
that the flat/WebXR paths simply never populate:

- **Action set + poses** (`crates/app/src/xr_input.rs`): a `HandSource`
  trait behind which the controller source runs — `OpenXrHands`, a real
  action set `"flight"` (aim pose, grip pose, trigger value, squeeze
  value, thumbstick, A/B click, haptic output, per hand) suggested for
  `/interaction_profiles/valve/index_controller` with a
  `/interaction_profiles/khr/simple_controller` fallback for whatever
  that profile actually has (grip/aim pose, trigger via the profile's
  boolean `select/click` through OpenXR's own click→float conversion,
  haptics; no analog squeeze, thumbstick or A/B), or `SynthHands`, a
  deterministic scripted pair of hands with no runtime at all —
  selected by `FARFALL_VR_HANDS` (defaulting to synthetic under
  fable/vr's own `FARFALL_VR=synth`), so a bench exercises this whole
  lane headless and 4-up on the desktop. `OpenXrHands::new` attaches
  the set to the session once, right after `xr::init` succeeds and
  before the event loop's first `begin_frame` — OpenXR requires every
  action set be attached before the session leaves its unattached
  state. Synced and located each frame from `xr_begin_frame`, in the
  same recentred LOCAL space and predicted display time the eyes are
  (a synthetic source ignores both, reading `Game::started.elapsed()`
  instead — its one clock, driving `FARFALL_VR_SCRIPT`'s named scripts:
  `idle`, `reach-stick`, `grab-stick-roll`, `throttle-push`,
  `laser-menu`), landing on `Game::vr_hands: VrHands { left, right:
  Option<HandPose> }` — the ship frame, the same convention `VrEye`
  uses. Every script logs one line per state transition (`"VR hands:
  right GRAB stick t=2.1s"`) so a bench harness can assert what
  happened without reading pixels.
- **Hand glyphs** (`crates/render/src/hands.rs`, `shaders/hands.wgsl`):
  a capsule-and-ring SDF raymarch per tracked hand at its grip pose,
  drawn in the ship pass right after the cabin, in the game's existing
  TRON/JET emissive vocabulary rather than a photoreal controller model.
  Per-eye parallax through the same `with_eye`-shifted-position
  convention `cabin`/`ghost`/`shield` already use — a hand a third of a
  metre from the eye shows real stereo disparity a distant cabin rarely
  does, and this pass must not repeat that zero-disparity mistake. No
  depth buffer exists to test a hand against the cabin's own raymarch,
  so `hands::dash_occlusion` approximates it: full brightness a few
  centimetres proud of the dash's own plane (`cabin::DASH_C`/`DASH_N`),
  fading to nothing a few centimetres behind it.
- **Laser point-and-press** (`crates/app/src/xr_laser.rs`): the right
  hand's aim ray intersects a virtual glass `VR_GLASS_M` (1m) in front
  of the current eye's own forward axis — the exact plane that eye's
  own symmetric render already treats as its screen — landing on the
  same screen NDC `Game::cursor_screen` speaks. `cursor_screen` prefers
  that hit (behind setting `vr.beam`) and falls through to the real
  mouse whenever the beam has nothing to report, so a desk-side mouse
  on the mirror window still works with VR up. `vr_laser_tick` (once a
  frame, before `game.tick()`) does trigger rising-edge detection and
  resolves a click against the CONTROLS card, the bay card, the menu
  and the map through the exact row/col hit-test and `MenuEvent` path
  the mouse's own left-click uses — deliberately narrower than the
  mouse handler, since drag and fire-on-release stay the physical
  mouse/HOTAS's own job. `hands.wgsl` also draws the beam itself, a
  thin capsule from the hand to the hit point, and the hit shows as the
  existing mouse pointer glyph (now VR-aware for free).
- **Virtual stick and throttle** (`crates/app/src/xr_grab.rs`): a grab
  state machine ported from Hotham's pattern (`examples/grab_object.rs`,
  Apache-2.0/MIT) — squeeze past 60% within 12cm of the stick's or
  throttle's own rest position (`cockpit.wgsl`'s own grip/lever
  geometry) takes it; squeeze below 40% releases it, the gap deliberate
  hysteresis against flutter at a shared threshold. Held displacement
  maps to pitch/roll (the stick, XZ) and yaw (the grip's own twist) or
  thrust (the throttle, Z alone) through a dead zone, a linear
  sensitivity, and a clamp. Feeds `InputState::set_vr_stick`, summed
  into `Controls` the same way the physical stick's own axes are — but
  the physical stick, the instant it moves past a small override dead
  zone, wins outright rather than fighting the virtual one
  (`InputState::summed`). The cabin's own physical stick/lever model
  already reads `Controls` for its lean, so it visibly follows a VR
  grab with no extra wiring. A held hand's glyph brightens and tightens
  (`HandGlyph::held`, read back from `GrabRig::holder`).
- **Haptics**: a pulse on a click (confirmatory, light), on taking a
  grab (firmer, a touch longer — there is something to feel taking
  hold of), and on letting one go (lightest of all) — `xr_input::
  XrInput::pulse` through `lib.rs`'s `pulse_hand`, silently a no-op
  wherever there is no native session or no bound haptic action.

Settings `vr.hands` and `vr.beam` (default on) gate the glyphs and the
beam independently; both follow the menu's existing VR HEADSET/VR
RENDER SCALE rows.

Not yet built: `XR_EXT_hand_tracking` pinch as an alternative to a
controller's squeeze (SPEC's own MVP order item (f)) — left for a later
pass if a headset without controllers is ever the target. Nothing here
has been worn: the pure geometry (binding tables, pose conversion,
ray/plane intersection, the grab state machine, dash occlusion) is
tested without a headset, and the whole thing builds and runs flat/
WebXR-side-effect-free with VR HEADSET off, but Jay Jay's Index pass —
same as §5.3's own eyes-only seam — is owed before any of this earns
`complete` in `features.yaml`.

## 6. Rendering doctrine

### 6.1 Image policy (P1 made concrete)

- Forward rendering. MSAA 4x default (2x/off as tier knobs). Resolve, then post.
- Post chain is minimal and sharp: exposure, filmic-ish tonemap, hash dithering
  (kills banding without temporal noise). No TAA, no motion blur, no chromatic
  aberration, no depth-of-field in gameplay.
- Volumetric/low-frequency effects (nebulae later, clouds later) render at ¼ res
  with temporal reprojection *contained in their own buffer*, composited under the
  crisp forward image. If reprojection artifacts exceed threshold: feature drops to
  a cheaper analytic form, never smears the full frame.
- Reverse-Z, f32 depth, infinite far plane. Camera-relative rendering (translation
  never leaves f64 until subtraction against camera position).

### 6.2 Shader policy

- WGSL is the single shader source of truth. naga translates everywhere we ship.
- Every `.wgsl` in `shaders/` parses + validates in `cargo test` (no GPU needed).
- Quality tiers reach shaders as **pipeline-overridable constants** (`override
  STAR_DENSITY: f32`), not preprocessor forks. One source, N specializations.
- Shaders are documented like code: header comment stating pass, inputs, lanes
  supported, and cost class.

### 6.3 Quality tiers (P4)

| Tier | Floor hardware | Lane | MSAA | Starfield | Atmosphere | Terrain (M2) |
|------|----------------|------|------|-----------|------------|--------------|
| low  | WebGL2-class iGPU | A | off | density 0.5 | analytic gradient | low-freq heightfield |
| medium | web/WebGPU, older dGPU | A/B | 2x | 1.0 | LUT (precomputed) | + noise detail |
| high | M1 native, mid dGPU | B | 4x | 1.5 | LUT + aerial perspective | + landmark SDFs |
| ultra | desktop dGPU | B | 4x + supersample knob | 2.0 | + multiple scattering | + shadows |

Tiers are defaults over individual per-pass knobs, all runtime-switchable; a config
file (`graphics.toml`) persists them.

### 6.4 Two-lane rule

**Lane A** = vertex+fragment only (WebGL2-expressible). **Lane B** = compute,
storage buffers, and friends. Every visual feature declares its lane; every Lane B
feature ships either a Lane A fallback or an off switch that doesn't break the scene.
(Example: atmosphere LUTs are generated in compute on Lane B, generated in a
fragment-shader pass into an offscreen target on Lane A — same LUT, slower bake.)

### 6.5 Pass roadmap

M0: `starfield` (procedural, fullscreen, octahedral cell hashing, Milky Way band).
M1: `planet` (analytic sphere impostor → shaded globe with continent mask, day/night,
city-lights emissive on night side), `hud` (crisp 2D). M2: `atmosphere` (Bruneton-lite
transmittance + single-scattering LUTs, aerial perspective), `terrain` (cube-sphere
chunked LOD heightfield, analytic noise + small authored landmark masks), `entry-fx`.

### 6.5.1 Instruments on the dash

The cockpit instruments are passes of their own (`gauge`, `gvec`, `gyro`, `horizon`,
`trajectory`), each SDF-drawn with constant-width strokes and one pixel of anti-aliasing.
Four styles, WARTHOG the default: WARTHOG (A-10 steam gauges — black plate, white
markings, red warning arc, the gyro a real ball), TRON (holograms on the glass), JET
(holograms over thin rings, the ball gyro), DIAL (period instruments on the dash).
**Nothing is ever hollowed into the dash for an instrument** — no wells, bowls, recesses
or lit sockets: a face plate sits proud of the metal in a thin bezel, the ball stands out
of it, and the instrument itself is what shows. The dash's real surface is 4 cm above
its nominal plane (the slab's rounding); every seat is measured from the surface.

### 6.5a Other ships (app-side, never sim-side)

Mimics and miners (WEAPONS.md, "Other ships") live in `crates/app`: derived
from rock hashes plus app state, stepped after the belt each fixed step, drawn
by the `mimic` pass from the shared fighter SDF at each hull's own pose and
size. They shove and shoot the ship through impulses after `sim::step`, like a
strike; the golden hash does not know they exist. A miner is the same ship
class as ours that grows through tiers as it mines the ring.

### 6.5b The walker (EVA, app-side, never sim-side)

DISEMBARK, landed, leaves the seat: a first-person walker standing beside the
ship, and the keyboard and mouse — reserved for the on-foot controller since
the stick took the whole cockpit — are its controls. The rocks' rule holds
here too: the walker is the app's, not the sim's. Its feet, velocity and gaze
live in `crates/app` as state hung off the game, stepped after `sim::step`
each fixed step, and the golden hash does not know anyone stepped out. The
sim keeps flying the ship, which is LANDED and stays put; while someone is on
foot the ship's controls are zeroed, so nothing can lift it off from under
them.

The ground is the ground the gear stands on: the body's analytic sphere at
`radius_m` (§6.7 — the planet is computed, not stored; there is no terrain
mesh to query, and the shader's visual relief has no CPU twin yet). The
walker works in the body's own frame — feet measured from the body's centre,
velocity over its ground — so a moving body carries its walker exactly as it
carries its landed ship. The translation binds walk the tangent, BOOST's key
runs, BRAKE's key jumps, gravity is the body's own μ/r², and the sphere-exact
ground resolution puts the feet back on the surface and takes the inward
velocity. The mouse look is always engaged on foot — no held button — yawing
about local up and pitching short of the poles. ESC still menus.

The camera is a view pose like the cockpit's and the chase rig's: the eye at
the feet plus eye height, expressed in the ship's frame — exact while the
ship is LANDED and still, which on foot it always is; the assumption is
revisited the day the ship can move with nobody aboard. An eye off the ship
already shows the hull (the jet pass), so the parked fighter stands there to
walk around; the cabin, the dash and the glass stay in the cockpit. The
readout swaps to the suit's lines — the ship's distance, and the DISEMBARK
key reading as BOARD within boarding range at the hull. The same key that
walked out walks back in.
### 6.5c The pilot's own craft: FIGHTER or HELICOPTER

The SHIP page's first row is **CRAFT**: the airframe the pilot's own ship
wears — FIGHTER (stock) or HELICOPTER — persisted as `ship.craft` in the
settings file, like the fit. It is a *parameter choice, never sim state*:
the app hands the sim the chosen `ShipParams` and silhouette exactly the
way the pad helicopters do (§6.5a's rule — the golden hash belongs to
`WorldState` and does not know the craft exists), and the world file is
untouched: the craft rides the settings, the orbit rides the save.

The HELICOPTER is FARFALL-native, not one of the cold-war practice hulls
on the pads: our own silhouette (`sd_heli_exterior`, in the shader
prelude beside `sd_fighter_exterior`, defined once) with a main-rotor
disc, ring tail, stub wings carrying the same four hardpoints, and the
same carved cabin the fighter has — the dash, dials, consoles and
column are shared, never forked. Every lane that draws the pilot's own
ship selects the silhouette by one craft flag in its uniforms: the
cabin (hull round the pilot), the chase/jet view, the SHIP bay's
hologram, the holo3PP miniature and the map's dart. Mimics, miners and
the ghost stay fighters; the pad helicopters stay cold-war hulls.

Flight model: the collective/cyclic/anti-torque routing the pad
helicopters proved (`heli::route_controls`), with one difference — this
is *your* ship, so the drives come with it: the hyper field and the
wormhole drive work, boost does not (a rotor has no afterburner). The
forward axis (W/S, the stick's throttle) is the collective, up the mast
only; the REFORGER HELI stick profile flies it as-is. The console's
throttle lever and the readout both show the collective while a
helicopter is flown.

### 6.6 The city, eventually (direction, not commitment)

The end-state city (M5+) is the ultimate test of P1+P2: dense, alive, and readable.
Direction: instanced shader-generated buildings (SDF/parametric facades), crowd and
traffic as instanced impostors driven by compute (Lane B) with sparse-instance
fallback (Lane A), lighting as emissive-first (neon reads crisply without deferred
G-buffers). Everything here must obey the two-lane rule and the no-smear policy —
that's *why* those rules exist from M0.


### 6.7 The universe is computed, not stored

A skybox is a *picture of a sky*. What this game needs is a *universe with real
positions in it*: stars you can fly toward, that grow, that turn out to be
somewhere. The current starfield hashes ray **direction**, which places every
star at infinity — correct for the distant sky, useless as a destination. The
fix is not a bigger texture; it is to hash **position** instead, so a star has
coordinates.

Real stellar density (~0.004 stars/ly³) makes the split obvious:

| Radius | Stars | Parallax over a journey | Representation |
|---|---|---|---|
| 100 ly | ~17,000 | large | **Near field**: real 3D positions, instanced points |
| 1,000 ly | ~17,000,000 | small | crossover |
| 5,000 ly | ~2,000,000,000 | none | **Far field**: direction-hashed shader |

So three bands, chosen per frame by distance and angular size:

- **Far field.** Direction-hashed fullscreen shader — today's `starfield.wgsl`.
  Not a compromise: at these distances parallax is physically unobservable, so
  a function of direction *is* the correct model. Zero storage, zero draw calls,
  billions of stars.
- **Near field (~0.1–1000 ly).** Stars as genuine 3D positions produced by
  hashing integer cell coordinates. Drawn as instanced points in camera-relative
  space, so parallax is free — it falls out of the vertex transform, no special
  case. ~17k instances covers 100 ly, which is nothing for a GPU.
- **Local (in-system).** The star resolves into a body: analytic sphere →
  shaded globe → terrain, with planets and rocks from the same seed. The
  distance-banded ladder of §6.4.

**The determinism contract.** All content is a pure function of integer cell
coordinates and the universe seed: `content_at(cell) -> Option<Body>`, identical
on every machine, in every session, from any distance. This is what makes the
rest work — you arrive at *the star you aimed at*, two players see the same sky
without shipping a universe database, and storage stays at zero. It is the same
discipline as the sim's determinism (§7.3), applied to space instead of time.

**Promotion and demotion.** Crossing a band threshold changes a body's
*representation*, never its identity or position. A star demoted to the far
field is still the same star, still at the same coordinates, still there when
you come back. This is the property FTL depends on: pick a point of light,
travel, and find the thing you picked.

**Consequence for FTL.** Faster-than-light travel is what makes the near field
visible at all — at orbital speeds nothing outside the local system shows
measurable parallax, so real 3D stars would look identical to a skybox. FTL is
therefore not a feature bolted on later; it is the reason the near field must
exist, and the two are designed together.

## 7. Simulation doctrine

### 7.1 State is plain data

```rust
WorldParams { planet: PlanetParams, ship: ShipParams }   // immutable per scenario
WorldState  { time_s: f64, ship: ShipState }             // the whole mutable world
ShipState   { pos_m: DVec3, vel_mps: DVec3, orient: DQuat, ang_vel: DVec3 }
Controls    { thrust_body: DVec3, torque_body: DVec3 }   // each component in [-1,1]
```
Planet-centered inertial frame, SI units, f64. Body frame is **right-handed:
+X right, +Y up, −Z forward (the nose)** — the glam/OpenGL convention, shared
by the sim and the camera so no fix-up rotation exists to get a sign wrong.
Declaring "+Z forward" beside "+X right, +Y up" describes a *left*-handed frame
and silently mirrors yaw, roll, and strafe; that shipped once and was caught by
flying it, not by a test, which is why `sim_directions` now asserts all six. No ECS until entity counts demand it
(revisit at first milestone that needs > ~100 dynamic entities).

### 7.2 Integration

Fixed timestep **dt = 1/120 s**, accumulator pattern in the app, interpolated
rendering. Symplectic (semi-implicit) Euler — bounded energy error on orbits, cheap,
and deterministic. Physics: point-mass gravity `a = -μ·r/|r|³`; exponential
atmosphere `ρ = ρ₀·e^(−h/H)`; quadratic drag opposing velocity; thrust/torque from
controls with per-ship maxima.

### 7.3 Determinism policy (P5)

- f64 arithmetic only via IEEE ops (`+ − × ÷ sqrt`) — deterministic across
  x86-64/aarch64.
- **All transcendentals through the `libm` crate** (pure-Rust, bit-stable), never
  `std::f64::sin/exp/...` (which call platform libm and differ).
- No `HashMap` iteration in sim logic, no time/randomness except seeded PRNG
  (when one is added: PCG, seed in scenario).
- Enforced by the cross-platform golden-hash CI gate.

### 7.4 State hash

FNV-1a 64 over the bit patterns of every state field in defined order. Used by:
determinism tests, cross-platform golden tests, and later, netcode desync detection.

### 7.5 World scale

Compact-Earth preset: R = 63.71 km (1:100), μ chosen to keep surface gravity ≈ 9.81,
atmosphere scale height exaggerated (H = 2 km) for visual depth. Low orbit ≈ 790 m/s,
period ≈ 8.5 min — an orbit is a gameplay beat, not an afternoon. The *numbers* are
presets; the *models* are scale-free, and the scale-invariance test proves it.

### 7.6 Persistence — the world file

The sim state is plain data and hashable (§7.1, §7.4); persistence is the
cheapest consequence of that and the M3 slice's "save exists" line item. The
game writes **`~/.farfall/world.cfg`** (the browser: localStorage `farfall.world`)
on quit, on window close, on the page being hidden, and every 30 s of sim time;
on launch it restores it, so you wake where you left off — in the belt, on the
Moon, mid-decaying-orbit.

- **Format**: `key = value` text like the settings file, no format crate.
  Floats are written in the shortest form that parses back to the same bits, so
  `parse(render(w)) == w` bit for bit — proven by the state hash.
- **Sealed by the hash**: the file carries `sim::state_hash` of the world it
  holds. A file whose hash does not match its contents — hand-edited, truncated,
  from a different build of the physics — is refused *whole*, never half-applied;
  the stock orbit stands and the log says why.
- **What it holds**: the whole `WorldState` (which alone regenerates the belt,
  the ring's phase, every body's position and which rocks are ships), plus the
  app-side ledgers that are not a function of it: the flight computer, the
  atmosphere, the chaos drive's entropy *and its hidden slip threshold*, the
  guns' ammo/heat/jams, the HAUL, hull integrity, the dead and wounded rocks,
  revealed mimics and the live ships, the game's one PRNG seed, the odometer.
- **What it never holds**: anything wall-clock (fades, shake, after-images), a
  warp in flight, a HOLD lock, a touchdown prediction, the settings (their own
  file). Those reconverge or re-acquire within a second of resuming.
- **Determinism**: a world resumed from its file runs on bit for bit as the
  uninterrupted one would have — the invariant test for this feature.
- **Never during a bench**: `FARFALL_BENCH*` runs neither read nor write it, so
  scene captures and golden images stay reproducible. `RESUME` (menu, `game.resume`)
  off or `FARFALL_RESUME=0` does the same for a session. `NEW GAME` (menu)
  forgets the file and stands the ship at the stock orbit.

## 8. Testing doctrine (P6)

| Layer | What | Where it runs |
|---|---|---|
| Unit/invariant | Circular orbit stays circular (radius/speed/energy bounds), drag decays speed, controls clamp, scale invariance | every `cargo test`, both CI platforms |
| Determinism | Same scenario twice → identical hash; **golden hash constant identical on macOS-arm64 and linux-x86_64** | CI matrix — this is the cross-play insurance |
| Shader static | Every `.wgsl` parses + validates via naga, all entry points present | every `cargo test`, no GPU needed |
| Golden image (M1+) | Offscreen render of fixed scenes vs reference, SSIM threshold; catches jitter (P3) and smear (P1) | native, feature-gated `render-tests` |
| Perf gates (M2+) | Frame-time budget assertions per pass on reference hardware | dev machine + doc'd manual gate per milestone |

Rules: new sim feature lands with its invariant test in the same PR. Golden hash
changes must be explained in the PR description (they mean physics changed).

## 9. Milestones

Each milestone ends with a demonstrable artifact and its acceptance tests green.

- **M0 — Bedrock** *(this repo, now)*: workspace, CI (fmt/clippy/tests × 2 platforms,
  wasm32 check, license audit), deterministic sim with orbit/drag/determinism/scale
  tests, WGSL validation harness, native window rendering the procedural starfield
  at MSAA 4x with a ship ticking in orbit. Accept: all CI green; starfield at
  ≥ 120 fps 1440p on M1.
- **M1 — Orbit**: camera-relative pipeline proven (jitter golden test at ≥ 10⁹ m
  offsets), flight model + input, Earth globe pass (continents, day/night, night
  lights), HUD v0, `ViewProvider` trait reviewed & frozen. Accept: fly around the
  planet by hand; goldens green.
- **M2 — Skyfall**: atmosphere LUTs + aerial perspective, entry effects, cube-sphere
  terrain LOD to 2 km altitude, quality tiers + `graphics.toml`, wasm/WebGPU build
  polished with Lane A fallbacks. Accept: orbit→2 km flyover on native and web at
  tier targets (§4).
- **M3 — First Descent** (the slice, §4): scenario scripting-lite, save, audio bed,
  options UI. Accept: the §4 "done means" paragraph.
- **M4 — Heads-in**: XR spike (native OpenXR on Windows; WebXR/WebGL2 evaluation on
  Quest browser) against the frozen `ViewProvider` seam; go/no-go per target with
  measured frame times. Accept: written verdict + at minimum the native path
  rendering the M2 scene in stereo at 90 Hz on Index-class hardware.
- **M5+ — Vision lane**: landing + touchdown (the sim's LANDED state, the
  DISEMBARK bind and the walk-out itself exist now — the ship settles on its gear
  and stays, and DISEMBARK leaves the seat for the surface on foot, §6.5b; what
  remains here is somewhere worth walking to); the shader city; background simulation
  (Elite-style faction states) + narrative layer (Yarn Spinner rust port, verified
  MIT OR Apache-2.0) — the "choices matter" layer from the original research, which
  remains the destination.

## 10. Licensing & repo policy

- Code: **MIT OR Apache-2.0** (dual, Rust-ecosystem standard; MIT alone was the
  stated wish — dual gives every downstream user the MIT branch *and* keeps patent
  grant compatibility with the dependency tree).
- No GPL/LGPL/AGPL anywhere in the tree — enforced by `cargo-deny` in CI.
- GPL games (Naev, Endless Sky, Pioneer, Oolite) are design references only; nobody
  reads their source while writing ours. Orbiter (MIT) may be ported from with
  attribution.
- No real star-catalog data yet (HYG/AT-HYG are CC-BY-SA; fine later as a clearly
  separated data package, but procedural stars avoid the question).
- Repo: **local git for now** (no remote); private GitHub when a remote is wanted.
  Milestones are annotated tags (`v0.1.0-m0`). Short-lived branches per task,
  merged into `main` with `--no-ff` so each task stays a reviewable unit.
  `core.hooksPath=.githooks` gates commits on fmt + clippy + tests.

## 11. Open questions (each with the experiment that settles it)

- **Multiplayer (direction, not commitment):** peer-to-peer sessions on the deterministic sim (the golden hash IS the anti-desync contract), with a rendezvous/relay node per region — the family's ShedNet estate can host the Europe relay. Experiment that settles it: two clients lock-stepping the sim over a relay for 10 minutes with identical hashes. Shareable HUD files (see ui.* export) double as the first player-to-player artefact.

1. **Octahedral star-cell distortion** — visible star-size variation near seams?
   → M0 ship it, judge by eye; fallback is 3-plane cube hashing. (cheap)
2. **Golden hash across compilers** — does a rustc upgrade change hashes? → pin
   toolchain in `rust-toolchain.toml`; CI failure on bump = answer. (free)
3. **Lane A LUT bake cost** — fragment-pass LUT generation fast enough on iGPU?
   → time it in the M2 fallback implementation. (small)
4. **Netcode model** — server-authoritative vs deterministic lockstep? P5 keeps both
   open; decide at multiplayer spike with a latency prototype. (deferred)
5. **Compact scale value** — is 1:100 right for pacing? → playtest at M1 with the
   scale as a debug slider; it's one parameter. (free)
6. **WebXR/WebGPU binding timeline** — revisit the spec's Editor's-Draft status at
   M4; if Quest Browser ships it stable, `WebXrView` may skip the WebGL2 lane.
7. **Multiplayer, by artefacts first** — the shareable HUD file (`.fhud`,
   `crates/app/src/hud_file.rs`) is the first player-to-player artefact: layouts
   travel between players today, by hand. The direction is more artefacts
   (ship fits, flight recordings) before live netcode; question 4 picks the
   transport when the spike comes.

## 12. Deltas from the 2026-08-19 research doc (docs/RESEARCH-2026-08.md)

- **Custom wgpu engine** replaces the Bevy option (owner's call; raises doc/test bar,
  accepted).
- **Readability pillar (P1)** is new — it now constrains all rendering choices.
- **Starfox-style visual economy** replaces "raymarch everything ambitions"; this
  dissolves the old R3 frame-budget risk for VR.
- **BGS + narrative move from M3 to M5+**: the slice is flight + Earth, not choices.
  The "choices matter" goal is unchanged, resequenced.
- **Cross-play (P5) is a new day-one constraint** from the owner; determinism policy
  upgraded accordingly.
- VR remains post-slice (M4), consistent with the research's risk table; the
  research's spike list R1/R2/R6 collapses into M4, R4 into M1 goldens, R5 into M2.
