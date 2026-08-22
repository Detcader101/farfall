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
**Left Shift** boosts, **Space** air-brakes, **X** toggles the flight computer, **T** toggles the predicted path, **K** opens DESIGN mode (the guide comes up and the mouse is the pointer: hover a dial, **-**/**=** size it, **Tab** cycles its style, **F** its fade, **Backspace** resets it, click-drag moves it, **K** saves and leaves — each dial's own settings are kept as `ui.<dial>.size/style/fade`), **G** toggles LANDING mode (the hoops close up to the LANDING HOOPS spacing, grow into gates, and turn green/amber/red with the predicted touchdown — soft, firm, hard — while the readout counts it down), **right mouse button** (hold) or **L** (lock) looks around with the mouse or trackpad — the head, not the nose: the ship never sees it — and while looking, **left mouse button** on a dial picks it up: turn your head and it comes with you, release to drop it anywhere on the glass (the menu's COCKPIT page shows it as DRAGGED; cycling its slot lets go) —
**C** cycles atmospheres, **[** / **]** set render scale, **M** opens the map with its WORMHOLE DRIVE panel (drag to turn, wheel or -/= to zoom; Enter engages), **J** fires the wormhole drive at the MAP page's plan (destination and safe distance, Enter there engages it too), **Esc** opens the menu — graphics, key bindings and the cockpit layout, all saved to `~/.farfall/settings.cfg` — and quitting lives there.

Everything is ship-relative at any attitude: "forward" is always where the nose
points. With the flight computer **on** (default) the ship also *goes* where it
points — velocity is steered toward the nose within a fixed slice of the engine,
so it handles like a VTOL: hold the horizon and the orbit holds, pitch down and
you descend. Switch it **off** and the ship is a pure ballistic body: attitude
and velocity are independent, and orbits are conserved exactly.

**Cockpit**: every instrument — speedo, altimeter, gyro, G meter, head-up horizon and its pitch ladder, predicted path and its hoops (and their sound), readout — can be moved between slots on the glass or switched off from the menu's COCKPIT page, and a SAFE EDGE pulls everything in from the rim. The cockpit is a solid fighter: a fuselage with the cabin carved out of it (the hull has a wall), the nose ahead and the swept wings seen through the glass, canopy arches and rails, a metal dash and side consoles — and under every dial a lit socket with a beam standing up to the hologram. CABIN FRAME / GLOW / METAL shape it (COCKPIT page); CABIN DETAIL (GRAPHICS page) is the fraction of the scene it is marched at. Turn your head and it goes round you. GAUGE STYLE picks TRON (holograms on the glass over lit sockets), JET (spherical wells hollowed into the dash with bezels, the dials under glass inside) or DIAL (real instruments set flush into the dash: each face is drawn in the dash's own plane, foreshortened with the view, in a bezelled well — drawn after the cabin, so nothing fights for depth); GAUGES picks FADE (by relevance) or STAY; GUIDE rules the glass and marks every dial's anchor and pick-up reach for laying the cockpit out; FOV lives on the GRAPHICS page and only changes the view — the glass is laid out at a fixed 70° reference, so a dial never moves when the FOV does; the GAUGES page of the menu holds the cockpit-wide style and fade, every instrument's slot, and a DIAL block to size, style and fade one dial at a time; dials may be dropped past the locked view's rim, where only a turned head sees them. Every dial re-ranges in 1-2-5 decades with a ×m Ek multiplier beside it and a three-digit readout that goes scientific past 999 — nothing caps, at any speed, height or load. HOOP SIZE scales the path's hoops; the MAP page's 3D map (drag to turn, wheel or -/= to zoom; poles drop every body to the Moon's orbital plane, the ship is a dart in its true attitude) has BODY RINGS (0–6) and GRID settings; they pass around the ship and fade red astern, so a look back shows the path just flown.

**Runtime knobs** (env vars, no rebuild needed; they override the settings file):

| Variable | Default | Purpose |
|---|---|---|
| `FARFALL_WINDOWED=1` | off | Start windowed instead of borderless fullscreen |
| `FARFALL_MSAA=1\|2\|4\|8` | 4 | MSAA sample count |
| `FARFALL_VSYNC=off` | on | Uncap the frame rate to see real headroom |
| `FARFALL_GPU_SYNC=1` | off | Profiling: block on GPU completion so timings measure the GPU, not submission |
| `FARFALL_SKIP=starfield,plasma` | none | Profiling: leave named passes out (`starfield`, `bodies`, `planet`, `plasma`, `trajectory`, `gauge`, `hud`, `blit`) so each one's cost shows up as its absence |
| `FARFALL_BENCH=1` | off | Freeze the sim at spawn (`FARFALL_BENCH_ALT`, `FARFALL_BENCH_SECONDS`) for comparable measurements; exits by itself. `FARFALL_BENCH_POS=x,y,z` parks the ship anywhere (with `FARFALL_BENCH_VEL` and `FARFALL_BENCH_LOOK`) to capture a scene; `FARFALL_BENCH_MAP=1` opens the 3D map, `FARFALL_BENCH_SPIN=n` turns the head a full circle and captures n frames, `FARFALL_BENCH_FULL=1` benches at the display's real size, `FARFALL_BENCH_THRUST=m,p,y,r` forces the throttle and RCS, `FARFALL_CAPTURE=final` captures the presented frame |

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
