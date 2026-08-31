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
**Left Shift** boosts, **H** (hold) is the CHAOS DRIVE, fuelled by entropy — the wormhole drive half-engaged: a field that moves space, not the ship, straight past the relativity wall. Its speed climbs with its charge (ENTROPY on the readout) up to 0.4 c — the Sun in a dozen seconds — and it is a skill game: past a limit you never quite know, the drive slips and the whole wormhole drops you into an unstable orbit of some random body going some random way; from halfway there the flight shakes, a little, then a lot. Let go and you are launched at whatever speed you had. **V** is WARP STOP: all speed and spin taken out at once, the ship warped in place, and the quantum after-image of your own jet slides on ahead down the old vector and fades, **Space** air-brakes (and kills any spin, harder than Z), **Z** (hold) is the emergency gyro — it kills any tumble on a fixed time constant, whatever the torque limits say (DESPIN on the keys page), **X** toggles the flight computer, **T** toggles the predicted path, **K** opens DESIGN mode (the guide comes up and the mouse is the pointer: hover a dial, **-**/**=** size it, **,**/**.** tilt it toward or away from you (a DIAL set into the dash leans on its own axis in a housing, so it reads from where you sit; a hologram foreshortens), **Tab** cycles its style, **F** its fade, **Backspace** resets it, click-drag moves it — the text readout too, which is a glass element like a dial and may go anywhere one may — **K** saves and leaves; each dial's own settings are kept as `ui.<dial>.size/style/fade/tilt`). Every style is open to every dial: WARTHOG (the default — the A-10's steam gauge: a black face plate on the dash in a machined bezel, white markings, a tapered white pointer, the warning arc painted red; the gyro a real ball standing out of the dash, blue over brown), TRON hologram, JET (a hologram over a thin ring; the gyro's is the same real ball — a globe of the world's frame, painted through like a true attitude ball) or DIAL — a period instrument, black face plate on the dash, ivory markings, a cream needle, red for the warning arc. Nothing is ever hollowed into the dash for an instrument: no wells, no bowls, no sockets — the instrument is what shows, with at most a thin rim. Gauges stay lit by default (GAUGES → FADE to have them fade by relevance). **G** toggles LANDING mode (the hoops close up to the LANDING HOOPS spacing, grow into gates, and turn green/amber/red with the predicted touchdown — soft, firm, hard — while the readout counts it down), **right mouse button** (hold) or **L** (lock) looks around with the mouse or trackpad — the head, not the nose: the ship never sees it — and while looking, **left mouse button** on a dial (or the text readout) picks it up: turn your head and it comes with you, release to drop it anywhere on the glass (the menu's GAUGES page shows it as DRAGGED; cycling its slot lets go) —
**C** cycles atmospheres, **[** / **]** set render scale, **M** opens the map with its WORMHOLE DRIVE panel (drag to turn, wheel or -/= to zoom; Enter engages), **J** fires the wormhole drive at the MAP page's plan (destination and safe distance, Enter there engages it too), **Esc** opens the menu (the pause panels sit on the screen and follow your head; drag them with the mouse) — graphics, key bindings and the cockpit layout, all saved to `~/.farfall/settings.cfg` — and quitting lives there.

Everything is ship-relative at any attitude: "forward" is always where the nose
points. With the flight computer **on** (default) the ship also *goes* where it
points — velocity is steered toward the nose within a fixed slice of the engine,
so it handles like a VTOL: hold the horizon and the orbit holds, pitch down and
you descend. Switch it **off** and the ship is a pure ballistic body: attitude
and velocity are independent, and orbits are conserved exactly.

**The drive's look**: the wormhole and the Chaos Drive show as a liquid, vortical refraction of the view with fine ripples, chromatic splitting, radial speed streaks and a cool rim — the lens barely widens; the picture does the talking — and the stars themselves draw out into streaks, the old Star Trek way. The drive's voice is a sub that swells under a saturated hum.

**The Sun**: a saturated dot from the planet, a surface up close — limb darkening, granulation, sunspots drifting with its rotation, prominences on the limb and the odd coronal mass ejection — with a lens flare in the glass (starburst, streak, rainbow ghosts; LENS FLARE on the GFX page).

**Asteroid belt**: Uranus' ring is the belt — set the drive to URANUS and ENGAGE, and it drops you straight into the belt (1.6 to 2 radii out, in the plane of Uranus' tilted spin) in a perfect orbit of it — going round with the rocks at exactly their speed, so they hang still about you; it is simply there, part of the same world. Rocks from five to three hundred metres go round with the ring, drift, knock each other about and knock the ship (the shield ripples, the hull grits); they are ray-traced on the GPU, craggy and Sun-lit. Seen from a distance the ring is not a sheet but the same population — the far rocks are drawn as lit specks from the belt's own hash, so what you fly into is what you saw.

**Shield**: the ship has a force field — a shell a few metres out, invisible until high-speed debris hits it; every strike sends a ripple of blue holographic light spreading evenly from the point of impact across the shell, the field's honeycomb showing through around it; nothing at rest, a hit every couple of seconds at a kilometre a second, a patter when fast, and under the hyper drive the whole shell ablates (SHIELD on the CABIN page).

**Sky**: low down the day sky is the air's colour — blue here, amber or green on the other atmospheres — and hides the stars; it thins with height and is black by the top of the air (SKY on the GFX page).

**Hull sounds** (CABIN page, HULL SOUNDS): space is silent, but the ship is not — under speed the frame groans and knocks (never hisses — there is no air), grains of rock hit the hull with a gritty scratch (the faster you go the more you meet; pebbles thud), and each hoop is a heartbeat — two soft thumps — that quietens at speed.

**Holo3PP**: press N and a real 3D hologram lights over an emitter in the dash: your own ship in miniature at its true attitude — the same fighter SDF, translucent cyan, nozzles glowing with the engines — with the velocity vector as a rod, the nearest body as an amber wire globe at its true bearing and angular size (the ground rises under the little ship as you come in to land), and the Sun's bearing as a bead. Third person without ever leaving first person: it is an object in the cabin, with parallax as you turn your head. Drag it along the dash, size it with HOLO SIZE (GFX page). Y swaps the whole screen to the raw chase camera (CAMERA on the GFX page) — the dev/bench reference the hologram is measured against.

**Cockpit**: every instrument — speedo, altimeter, gyro, G meter, G vector (a cross-plot of the felt load: a line from the hub to a dot for sideways and up/down, a bar for fore/aft, ranging like the rest), head-up horizon and its pitch ladder, predicted path and its hoops (and their sound), readout — can be moved between slots on the glass or switched off from the menu's GAUGES page. The cockpit is a solid fighter: a fuselage with the cabin carved out of it (the hull has a wall), the nose ahead and the swept wings seen through the glass, canopy arches and rails, a metal dash and side consoles — and the instruments sitting ON the dash — face plates in thin bezels, the gyro's ball standing proud — never hollowed into it. CABIN FRAME / GLOW / METAL shape it (CABIN page); CABIN DETAIL (GFX page) is the fraction of the scene it is marched at, and FPS FLOOR (default 60) governs the size it is re-marched at while the head turns, so a turn never drops under the floor. Turn your head and it goes round you. GAUGE STYLE picks WARTHOG (the default: steam gauges on the dash, the ball gyro), TRON (holograms on the glass, a beam of light up to each), JET (holograms over thin rings on the dash, the ball gyro) or DIAL (period instruments on the dash: each face is drawn in the dash's own plane, foreshortened with the view, inside a thin bezel — drawn after the cabin, so nothing fights for depth); GAUGES picks FADE (by relevance) or STAY; GUIDE rules the glass and marks every dial's anchor and pick-up reach for laying the cockpit out; FOV lives on the GRAPHICS page and only changes the view — the glass is laid out at a fixed 70° reference, so a dial never moves when the FOV does; the GAUGES page of the menu holds the cockpit-wide style and fade, every instrument's slot, and a DIAL block to size, style and fade one dial at a time; dials may be dropped past the locked view's rim, where only a turned head sees them. Every dial re-ranges in 1-2-5 decades with a ×m Ek multiplier beside it and a three-digit readout that goes scientific past 999 — nothing caps, at any speed, height or load. HOOP SIZE scales the path's hoops; the MAP page's 3D map (drag to turn, wheel or -/= to zoom; poles drop every body to the Moon's orbital plane, the ship is a dart in its true attitude) has BODY RINGS (0–6) and GRID settings; they pass around the ship and fade red astern, so a look back shows the path just flown.

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

## Play in the browser (and in VR)

**https://detcader101.github.io/farfall/** — the same game as a WebGPU page: nothing to
install, no account. Chrome / Edge 113+, Safari 26+, or the Meta Quest Browser. Click to
fly (the world's render scale governs itself to hold 60 fps at your screen's full resolution —
AUTO SCALE on the GFX page; the HUD and dials are always native); in a headset an **ENTER VR** button appears (WebXR — Quest standalone in its browser,
or Index / Quest Link through SteamVR + Chrome on a PC). Controllers: left stick translates,
right stick pitches and yaws, triggers boost and brake, grips roll, X chaos drive, Y warp
stop, A flight computer, B menu.

```sh
./web/build.sh                      # wasm + page into web/dist (needs wasm-bindgen-cli)
python3 -m http.server -d web/dist  # then open http://localhost:8000
```

The app crate is a library (`crates/app/src/lib.rs`) that the native binary and the wasm
module share; `crates/app/src/web.rs` is the browser shell and the WebXR frame entry,
`web/xr.js` the session, compositor and controllers. Pushes to `web` (and `main`) deploy
through `.github/workflows/pages.yml`.

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
