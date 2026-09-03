# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

FARFALL: a shader-driven Rust/wgpu space sim. **SPEC.md is the living specification and changes there before code.** TASKS.md holds the current milestone's tasks; features.yaml is the honest feature ledger (a feature is `complete` only with tests through the gate, the workflow, and an eyeballed e2e capture or flight — update it per chunk). WEAPONS.md and SHIP.md spec their subsystems.

## Commands

```sh
cargo test --workspace                 # the whole gate: sim invariants, determinism, shader validation, text/UI
cargo test -p farfall-sim              # one crate
cargo test -p farfall-render sight     # one module's tests by name filter
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings   # CI gates, run before pushing
cargo run --release -p farfall-app     # fly it (binary is `farfall`, Esc menu to quit)
cargo check --workspace --target wasm32-unknown-unknown   # the web lane must never rot
```

- Toolchain is pinned by `rust-toolchain.toml` (exact version, deliberate — a bump is a reviewed event, not drift). rustup handles it.
- **Windows (this machine)**: builds need `C:\Users\jayja\mingw64\bin` on PATH (it is on the persistent user PATH). The rustup GNU toolchain's self-contained mingw lacks `as.exe`, so without it the `windows-*` crates die with "dlltool ... CreateProcess". The exe is `target\release\farfall.exe`; kill a running game before rebuilding (Windows locks the exe).
- Scene (golden-image) tests are opt-in: `FARFALL_SCENE_TESTS=1 cargo test -p farfall-app` — they run real bench captures, GPU required.
- Sim golden hash: if physics legitimately changed, regenerate with `cargo test -p farfall-sim print_golden -- --ignored --nocapture` and explain the change in the PR. A hash change that wasn't intended is a determinism leak — fix the leak, never the constant.
- Runtime knobs for verification: `FARFALL_WINDOWED=1`, `FARFALL_BENCH=1` (+ `FARFALL_BENCH_POS/LOOK/MIMIC/...`), `FARFALL_CAPTURE=final`, `RUST_LOG=info`. See README's table. E2e verification = a bench capture or flight actually looked at.

## Architecture

Three crates with a strict dependency rule — `app → {render, sim}`, `render` and `sim` import nothing of ours:

- `crates/sim` (farfall-sim): pure Rust, no GPU/window deps, compiles to wasm trivially. Fixed-step (1/120 s) symplectic Euler, planet-centred inertial frame, SI, f64. **The sim is the only authority on world state**; the renderer is a view driven by interpolated snapshots — gameplay state never lives there.
- `crates/render` (farfall-render): wgpu passes, cameras, quality tiers. Never imports sim; the app translates sim state into render-facing structs. The XR seam (`ViewProvider`) is the one trait where flat and VR differ.
- `crates/app` (farfall-app): winit shell, input, frame loop, sim↔render wiring; almost all gameplay-adjacent glue (cockpit layout, mimics, arms, belt, map, menu) lives here as modules.
- `shaders/`: all WGSL, composed with `common.wgsl`, registered in `crates/render/src/shaders.rs`, statically validated by `crates/render/tests/shader_validation.rs` (naga, no GPU needed). **Every visual thing is a shader — no image assets, ever.**

### Determinism (the cross-play insurance)

Bit-identical sim across platforms, enforced by a golden-hash CI gate (macOS-arm64 + linux-x86_64; verified identical on Windows x86-64 too). In sim code: transcendentals only through `libm` (never `std::f64::sin/...`), no HashMap iteration, no time/randomness except seeded PRNG. UI/glue layers (belt rocks, mimics) stay off the sim by deriving from hashes instead of stored state.

### Conventions that bite

- Body frame is right-handed **+X right, +Y up, −Z forward (the nose)** — sim and camera share it; `sim_directions` asserts all six. "+Z forward" is a silent mirror.
- Text/UI layout maths lives in plain Rust under test (e.g. `render/src/text.rs` bitmap, `render/src/sight.rs` projection); shaders only answer "is this pixel lit". The text bitmap silently clips out-of-bounds pixels — a line at row y needs `y + GLYPH_H` rows.
- Every applicable feature ships with a setting (menu row + `~/.farfall/settings.cfg` key) and every control is rebindable; panels are draggable and their anchors saved.
- Test names are behaviour specs (`a_mimic_off_the_glass_gets_an_edge_arrow_pointing_its_way`); a new sim feature lands with its invariant test in the same PR.
- 60 fps floor at 2880×1800; the cabin governs its own detail to hold it. No wind-like noise beds in space, no chimes.

### CI (.github/workflows/ci.yml)

fmt → clippy `-D warnings` → tests on macOS + Linux, wasm check, cargo-deny license audit (no copyleft in the tree, ever). Pages deploys the web build — the page is the game.
