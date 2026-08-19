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

- No combat, no NPCs, no economy, no narrative system. (Vision-level only; see §9.)
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

`render` exposes one trait boundary where flat and VR differ:

```rust
trait ViewProvider {           // sketch — final signature in code
    fn views(&self) -> &[ViewPose];      // 1 for flat, 2 for stereo
    fn begin_frame(&mut self) -> FrameTargets;
    fn submit(&mut self, ...);
}
```

M0–M3 implement only `FlatView`. M4's spike implements `OpenXrView` (native,
Windows/SteamVR) and evaluates `WebXrView` (browser, WebGL2 lane — see research
doc for why WebGPU-in-XR isn't shippable yet). Everything above the trait is
written once. **Getting this trait wrong is the most expensive mistake available
in M1 — it gets a design review before M1 closes.**

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

### 6.6 The city, eventually (direction, not commitment)

The end-state city (M5+) is the ultimate test of P1+P2: dense, alive, and readable.
Direction: instanced shader-generated buildings (SDF/parametric facades), crowd and
traffic as instanced impostors driven by compute (Lane B) with sparse-instance
fallback (Lane A), lighting as emissive-first (neon reads crisply without deferred
G-buffers). Everything here must obey the two-lane rule and the no-smear policy —
that's *why* those rules exist from M0.

## 7. Simulation doctrine

### 7.1 State is plain data

```rust
WorldParams { planet: PlanetParams, ship: ShipParams }   // immutable per scenario
WorldState  { time_s: f64, ship: ShipState }             // the whole mutable world
ShipState   { pos_m: DVec3, vel_mps: DVec3, orient: DQuat, ang_vel: DVec3 }
Controls    { thrust_body: DVec3, torque_body: DVec3 }   // each component in [-1,1]
```
Planet-centered inertial frame, SI units, f64. No ECS until entity counts demand it
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
- **M5+ — Vision lane**: landing + touchdown; the shader city; background simulation
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
- Repo: private GitHub, invited collaborators. Conventional commits, PRs after M0.

## 11. Open questions (each with the experiment that settles it)

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
