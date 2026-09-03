# FARFALL editions — one codebase, two ways to fly it

FARFALL ships as **two editions of the same game**, built from **one repository
and one workspace**. There is no fork, no "web port" and no second sim: the
deterministic simulation, every render pass, every shader, the cockpit, the
menus and the settings keys are the same Rust, compiled twice.

| | **NATIVE** | **WEB** |
|---|---|---|
| What it is | A single-player executable | The GitHub Pages build — *the page is the game* — and the home of multiplayer |
| Platforms | Windows, macOS, Linux (x86-64 and arm64) | Any WebGPU browser: Chrome / Edge 113+, Safari 26+, Meta Quest Browser |
| Players | One | One today; **N players** per `docs/PLAN-MULTIPLAYER.md` |
| VR | **Native OpenXR** (SteamVR on Windows) — branch `fable/vr`, SPEC §5.3 | **WebXR** — Quest standalone in its own browser, or Index / Quest Link through SteamVR + Chrome |
| Stick | HOTAS out of the box (T.Flight HOTAS 4 stock, any stick through the SETUP WIZARD) via winmm on Windows | The same HOTAS through the browser Gamepad API, the same wizard |
| Settings and world file | `~/.farfall/settings.cfg`, `~/.farfall/world.cfg`, `~/.farfall/huds/*.fhud` (USERPROFILE on Windows when HOME is unset) | The same keys and the same file format, kept in `localStorage` |
| Build | `cargo run --release -p farfall-app` | `./web/build.sh` (wasm-bindgen, optional wasm-opt) → `web/dist` |
| Deploy | Copy the exe; nothing else to install | Every push to the deployed branch publishes through `.github/workflows/pages.yml` |

Jay Jay's framing, which this document holds to: *"the same game and completely
cross-platform."* A feature that works in one edition and not the other is a bug,
not a difference in kind; the differences below are the shells around the game,
never the game.

## What is shared (everything that matters)

| Layer | Where | Notes |
|---|---|---|
| Simulation | `crates/sim` (`farfall-sim`) | Pure Rust, f64, fixed 1/120 s step, no GPU/window deps. Bit-identical on every platform *and in wasm*: transcendentals only through `libm`, no HashMap iteration, seeded PRNG only. The golden-hash CI gate (macOS-arm64 + linux-x86-64, verified on Windows) is what makes cross-play between editions possible at all. |
| Renderer | `crates/render` (`farfall-render`) | wgpu passes, cameras, quality tiers. Knows nothing about winit, the browser, or XR beyond the seam below. |
| Shaders | `shaders/*.wgsl` | All WGSL, composed with `common.wgsl`, statically validated by `cargo test` (naga). No image assets in either edition — *every visual thing is a shader*, which is also why the web build is one wasm file and one page. |
| Game logic and glue | `crates/app/src/lib.rs` and its modules | The app crate is a **library** both shells link: cockpit, dials, DESIGN mode, mimics, arms, belt, map, menu, landing, EVA, helicopters, the STICK wizard, save/resume. |
| Audio | `crates/audio` | Hull sounds, hoops, plume — the same synthesis in both. |
| Settings, HUD files, world files | `crates/app/src/settings.rs`, `hud_file.rs`, `save.rs` | Same keys, same `key = value` and `.fhud` formats. A `.fhud` saved natively loads in the browser and vice versa; a `world.cfg` seals under the same FNV-1a hash in both. |
| Input model | `crates/app/src/input.rs`, `stick.rs` | Named controls, every key rebindable, the stick as a named-control map. Only the OS-side poll differs (below). |
| Tests | `cargo test --workspace` | One gate for both editions. `cargo check --workspace --target wasm32-unknown-unknown` is part of CI so the web lane can never rot behind native. |

## What differs (the shells)

### 1. The shell: window and frame loop

- **NATIVE**: `crates/app/src/main.rs` — a winit window (borderless fullscreen, `FARFALL_WINDOWED=1` for a window), the OS event loop drives `Game::tick` and `redraw`. Bench and capture knobs (`FARFALL_BENCH=*`, `FARFALL_CAPTURE`, `FARFALL_WINDOW_POS`) are native-only tooling.
- **WEB**: `crates/app/src/web.rs` — the browser shell. `web/index.html` is the page, `requestAnimationFrame` drives the same `Game` through wasm-bindgen; the canvas is the swapchain; render scale governs itself to hold 60 fps at the screen's full resolution (AUTO SCALE on the GFX page) while the HUD and dials stay native-resolution.

### 2. Input: where the stick comes from

- **NATIVE**: the stick is read through winmm (`joyGetPosEx`, `windows-sys`) on Windows; a HOTAS plugged in mid-session is found within the second. Keyboard and mouse through winit.
- **WEB**: the same stick through the browser **Gamepad API**, keyboard and mouse through DOM events. The STICK page, its 37-step SETUP WIZARD and the keyboard-free menu piloting (`docs/HOTAS.md`) are identical code above that line.

### 3. The XR seam (SPEC §5.3)

The seam is a data type, not a backend: `VrEye` (an eye's orientation, seat and four frustum tangents) and `Option<VrView>` on `Game`. When it is `None` the app is exactly the flat game; when `Some`, `Game::pose()` and `Game::head()` become the headset's, and *nothing downstream changes*. The two editions populate it differently and get the stereo pair out differently:

- **WEB / WebXR**: `web/xr.js` owns the `XRSession` and hands `web.rs::xr_frame` both eyes' poses and tangents each browser frame. The game renders each eye, symmetric-FOV, into its half of the canvas; a small WebGL2 compositor in `xr.js` crops each eye's true asymmetric field back out into the headset's framebuffer. Controllers: left stick translates, right stick pitches and yaws, triggers boost and brake, grips roll, X chaos drive, Y warp stop, A flight computer, B menu. An **ENTER VR** button appears on the page when a headset is present.
- **NATIVE / OpenXR** (`fable/vr`, PR pending): `crates/app/src/xr.rs` stands up an OpenXR session whose Vulkan device is handed to wgpu through wgpu-hal, so the runtime approves the device before wgpu touches it. The same symmetric pair is rendered offscreen and `shaders/xrblit.wgsl` (`crates/render/src/blit_xr.rs`) crops each eye into its OpenXR swapchain image — the identical UV maths as the web compositor, as a pure tested Rust function. The left eye mirrors aspect-fit into the window. Chosen at start-up by the **VR HEADSET** setting or `FARFALL_VR=1`; falls back to flat, logging why, on any failure; never launches SteamVR itself. **VR RECENTRE** (HOME) re-seats yaw and position, never pitch or roll.

Everything above the seam — the sim, every world pass, the cabin, the sight — was written once. A VR-specific branch anywhere in gameplay code is a bug in the seam.

### 4. Netcode: the multiplayer edition (WEB)

Multiplayer is the web edition's reason to exist beyond "no install". The design lives in **`docs/PLAN-MULTIPLAYER.md`** (being written alongside this document; it is the authority on anything this section summarises). The shape it follows is SPEC §11's direction:

- **The sim is the only authority** (SPEC §5.2). Multiplayer changes "input" into "inputs from N players" and nothing else changes shape. State is already serialisable and hashable; the golden hash *is* the anti-desync contract.
- **Client/server state**: each peer runs the full deterministic sim in lockstep on the agreed inputs; there is no server-side physics to disagree with. A host peer owns the session's clock, membership and input ordering — the "server" is a role a client plays, not a separate build.
- **Host peer-to-peer**: peers exchange inputs directly (WebRTC data channels in the browser) with the host as the tie-breaker. Two clients lock-stepping for ten minutes with identical hashes is the experiment that settles it.
- **Regional relay — ShedNet hosts Europe**: a rendezvous/relay node per region for signalling, NAT traversal fallback and late-join snapshots. The family's ShedNet estate (Proxmox, the same estate that runs the Tekken bot) hosts the Europe relay; other regions are a matter of running the same small service elsewhere.
- **Artefacts first**: the shareable `.fhud` HUD file is already the first player-to-player object; ship fits and flight recordings follow the same path before live netcode is on by default.

The native edition stays single-player by design: no relay dependency, no session, nothing that can fail without a network. Should the netcode later prove worth carrying natively, the sim's authority rule means the same lockstep code compiles there too — the choice is a product decision, not an architectural one.

## Build and run, side by side

```sh
# Both editions, one gate
cargo test --workspace
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
cargo check --workspace --target wasm32-unknown-unknown

# NATIVE — single-player exe (binary is `farfall`; Esc opens the menu)
cargo run --release -p farfall-app
FARFALL_VR=1 cargo run --release -p farfall-app     # native OpenXR, fable/vr, SteamVR already running

# WEB — the Pages build, WebXR and (per the plan) multiplayer
./web/build.sh                          # wasm + JS glue + page into web/dist (wasm-bindgen-cli; wasm-opt if present)
python3 -m http.server -d web/dist      # http://localhost:8000 — ENTER VR appears with a headset
```

The live page: **https://detcader101.github.io/farfall/** — built by `.github/workflows/pages.yml` on every push to the deployed branch. Merging into that branch *is* a deploy.

## Rules that keep it one game

1. A feature lands in both editions or it is not complete — `features.yaml` ledgers the e2e for each where they differ (WebXR's stereo pair has a headless check; native OpenXR needs a worn pass).
2. Anything behind `cfg(target_arch = "wasm32")` or the XR seam is *shell*: windowing, input polling, storage, compositing, transport. If it touches how the ship flies, it belongs in shared code.
3. The web lane is in CI (`cargo check --target wasm32-unknown-unknown`); a native-only dependency that breaks it is a build failure, not a follow-up.
4. Settings keys, `.fhud` and `world.cfg` are the interchange formats between editions and between players. Change them the way SPEC changes: format first, then code, and never drop a key the other edition writes.
