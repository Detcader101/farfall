# TASKS — current milestone breakdown

Convention: every task states its **tests-first** step. A task is done when its tests
pass in CI on both platforms, not when it "works on my machine". Tasks are written to
be executable cold — by a person or an AI session with no prior context beyond
SPEC.md. Read SPEC.md first; section references (§) point there.

## M0 — Bedrock

- [x] Cargo workspace: `crates/{sim,render,app}`, `shaders/`, pinned toolchain
- [x] `farfall-sim`: WorldParams/WorldState/Controls (§7.1), symplectic Euler @ 1/120 s (§7.2),
      gravity + exponential atmosphere + drag, libm-only transcendentals (§7.3), FNV-1a state hash (§7.4)
- [x] Sim tests: circular-orbit invariants, drag decay, determinism (run-twice),
      scale invariance, golden hash constant
- [x] Shader harness: naga parse+validate every `shaders/*.wgsl` in `cargo test`
- [x] `starfield.wgsl`: fullscreen pass, octahedral cell hash stars, Milky Way band,
      exposure + dither, `override STAR_DENSITY` (§6.2, §6.5)
- [x] `farfall-render`: StarfieldPass + FrameUniforms + MSAA 4x target management
- [x] `farfall-app`: winit 0.30 shell, fixed-timestep accumulator, ship ticking in orbit,
      camera at ship, starfield rendering
- [x] CI: fmt, clippy -D warnings, tests on macos-14 + ubuntu (golden-hash cross-check),
      wasm32 `cargo check` for sim+render, cargo-deny license audit
- [x] ~~Push to private GitHub repo~~ — deferred: local-only git for now (owner's call).
      Repo is tagged `v0.1.0-m0`; push when a remote is wanted.
- [x] Eyeball pass on starfield (§11.1) — three structural defects found and fixed:
      infinite-tail falloff (cell quilt), scalar Jacobian (size gradient),
      missing anisotropy (elliptical stars). Owner signed off on the result.

## M1 — Orbit

Each task: (files) → tests to write FIRST → acceptance.

1. **Camera-relative math module** (`render/src/camera.rs`)
   Tests first: unit test that world-space f64 positions at offsets up to 1e12 m
   produce camera-relative f32 with error < 1e-3 m near the camera; golden-image
   jitter test scaffold (feature `render-tests`): render two frames 1 sim-tick apart
   at offset 1e9 m, SSIM > 0.995 on static scene.
   Accept: no visible jitter at extreme offsets.

2. ~~**Input → Controls mapping**~~ ✅ DONE (`app/src/input.rs`)
   Tests first: sim-side clamp tests already exist; add mapping unit tests
   (key state → Controls vector, all components within [-1,1], no NaN).
   Keyboard: WASD+RF translation, arrows+QE rotation; gamepad optional (gilrs, MIT/Apache — verify).
   Accept: hand-flyable ship, controls feel weighty (max accel/torque from ShipParams, no instant stops).

3. ~~**Flight assist toggle**~~ ✅ DONE — implemented inside `sim::step` rather than a
   separate `assist.rs`: it is ~8 lines that belong in the rotation integrator, and
   splitting it would have separated the damping from the arithmetic it must not
   perturb. Tests live in `sim/tests/assist.rs`.
   Tests first: with assist on and zero input, angular velocity decays to < 1e-3 rad/s
   within 3 s; without, it persists exactly (conservation).
   Rotational damping only (Souls-weight: no magic translation brakes).
   Accept: assist on/off switchable, deterministic (hash tests still green).

4. **Planet globe pass** (`shaders/planet.wgsl`, `render/src/planet.rs`)
   Tests first: WGSL validation picks it up automatically; unit test sphere
   ray-intersection helper (analytic impostor: fullscreen-quad or bounding-quad ray
   vs sphere in camera-relative space); golden image of globe at 3 distances.
   Continent mask: small (≤ 256 KB) hand-authored equirect texture + shader noise
   detail (P2 asset budget). Day/night from sun dir; night side city-lights emissive
   (mask-driven). Lane A only.
   Accept: Earth readable as Earth from orbit; terminator crosses landmarks.

5. **Sun + tonemap pass unification** (`render/src/post.rs`)
   Tests first: golden image; assert no banding artifacts (dither on).
   Single directional sun; exposure adapts between space/day hemisphere (simple, no
   auto-exposure history buffer — P1).
   Accept: consistent look space→dayside.

6. **HUD v0** (`shaders/hud.wgsl`, `render/src/hud.rs`)
   Tests first: WGSL validation; layout unit tests (screen-space anchoring math).
   Altitude, speed (surface + orbital), prograde/retrograde markers. Vector-crisp
   (SDF lines/text or instanced quads), MSAA-friendly, no textures.
   Accept: flyable with instruments alone.

7. **ViewProvider trait review** (§5.3) (`render/src/view.rs`)
   Tests first: FlatView unit tests (view matrix, projection, resize).
   Write the trait + FlatView impl + a written half-page ADR in docs/adr/0001-viewprovider.md
   arguing it can host OpenXR stereo (2 views, per-eye projection, shared world) without
   breaking callers. This is the M1 gate — do not close M1 without this review.
   Accept: ADR merged; render loop consumes only the trait.

8. **Scale slider (debug)** (§11.5) (`app/src/debug.rs`)
   Tests first: changing WorldScale preserves invariant tests at 3 sample scales.
   Accept: 1:50 / 1:100 / 1:200 switchable at runtime for playtesting.

## Universe / FTL lane (SPEC §6.7 — design agreed, not yet scheduled)
Near-field star instancing (position-hashed cells, real parallax) · band
promotion/demotion by angular size · deterministic `content_at(cell)` seed
function shared by sim and render · FTL travel model + the "arrive at the star
you aimed at" test · star -> system -> body resolution ladder.
Note: near-field parallax is invisible at orbital speed, so it demos only
alongside FTL or a near body. Sequence accordingly.

## Backlog seeds (M2+, unordered — do not start)
Atmosphere LUT bake (Lane B compute + Lane A fragment fallback) · cube-sphere terrain
chunking · entry-fx pass · graphics.toml + tiers UI · wasm entry + canvas glue ·
save format (postcard? verify license) · audio bed (kira? verify license) ·
OpenXR spike plan (M4) · golden-image infra hardening
