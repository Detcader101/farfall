# FARFALL *(working title)*

A shader-driven space game: **Starfox clarity at planetary scale.** Simple, readable
geometry; the GPU budget goes to real distances, real atmospheres, and a compact
Earth you can approach from orbit — eventually down to a cyberpunk city drawn by
shaders rather than assets. Quiet, weighty, souls-ish in tone.

**Status: M0 "Bedrock".** Deterministic sim core with a full invariant test suite,
a custom wgpu renderer with a procedural starfield at MSAA 4x, CI on two platforms,
and the living spec. Not yet a game — deliberately (see the milestones in
[SPEC.md](SPEC.md) §9).

## Read this first

| Doc | What it is |
|---|---|
| [SPEC.md](SPEC.md) | The living specification: pillars, architecture, doctrines, milestones. **Changes here before code.** |
| [TASKS.md](TASKS.md) | Current milestone broken into cold-startable, tests-first tasks. |
| [docs/RESEARCH-2026-08.md](docs/RESEARCH-2026-08.md) | The stack/license/WebXR research this project is built on (verified 2026-08-19). |

## Quickstart

```sh
# toolchain is pinned by rust-toolchain.toml; rustup handles it
cargo test --workspace                # physics, determinism, input, shaders, text
cargo run --release -p farfall-app    # fly it (Esc to quit)
```

**Controls** — WASD translate, R/F up/down, arrows pitch/yaw, Q/E roll,
**Left Shift** boosts, X toggles flight assist, Esc quits. All translation is
ship-relative at any attitude: "forward" is always where the nose points.

**Runtime knobs** (env vars, no rebuild needed):

| Variable | Default | Purpose |
|---|---|---|
| `FARFALL_WINDOWED=1` | off | Start windowed instead of borderless fullscreen |
| `FARFALL_MSAA=1\|2\|4\|8` | 4 | MSAA sample count |
| `FARFALL_VSYNC=off` | on | Uncap the frame rate to see real headroom |
| `FARFALL_GPU_SYNC=1` | off | Profiling: block on GPU completion so timings measure the GPU, not submission |

Frame stats appear on-screen and are summarised to the log every 5 s at
`RUST_LOG=info`. The number that matters is the **1% low**, not the average —
an average hides the stutter that a 90 Hz headset would not.

## Layout

```
crates/sim      deterministic headless simulation (no GPU/window deps)
crates/render   wgpu passes, cameras, quality tiers (no winit/sim deps)
crates/app      native shell: window, input, frame loop, wiring
shaders/        all WGSL — statically validated by `cargo test`
```

## Contributing (humans, AI-assisted, or neither)

The tests are the contract. Before touching physics, read `crates/sim/tests/`;
before touching a pass, read SPEC §6. Rules that will save you a review round:

- New sim behavior lands **with its invariant test in the same PR**.
- A changed golden hash means physics changed — explain it in the PR, own commit.
- Shaders: WGSL only, `override` constants for quality knobs, no new assets
  without a P2 justification (SPEC §2).
- No GPL/LGPL/AGPL code or ports — CI enforces the license allowlist (deny.toml).
  GPL space games are design references only; don't read their source while coding.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option. Contributions are accepted under the same terms.
