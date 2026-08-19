# Shader-Driven Open-Source Space RPG — Technical Plan v0.1

Author: Opus 5 research pass, 2026-08-19. Every license and API claim below was
verified against crates.io, docs.rs, or the upstream spec/issue on this date.
Sources are listed at the bottom.

---

## 1. The constraint that shapes everything

You asked for: shaders, high framerate, web + wasm, Mac + Windows + VR (Quest / Index).
There is one hard fact that decides the architecture:

**WebXR cannot use WebGPU in production yet.**

| Fact | Status as of Aug 2026 |
|---|---|
| WebGPU on desktop browsers | Shipping by default: Chrome/Edge 113+, Safari 26+, Chrome Android 151+. Firefox still platform-gated. |
| WebXR/WebGPU Binding spec (`XRGPUBinding`) | Immersive Web **Editor's Draft, 15 June 2026** — not a Candidate Recommendation. |
| Chrome support | Behind two `chrome://flags` — *WebXR Projection Layers* + *WebXR/WebGPU Bindings*. Canary since 135. Dev testing on Windows + Android only. |
| Meta Quest Browser | Experimental WebGPU + WebXR depth projection added **21 Apr 2026**. Not production. |
| `web-sys` XR bindings | Has `XrSession`, `XrFrame`, `XrViewerPose`, `XrWebGlLayer`… but **no `XrGpuBinding`, no `XrProjectionLayer`, no `XrGpuSubImage`**. Confirmed against docs.rs today. |
| `wgpu` 30.0 | Has `ExternalTexture` — but it is video-oriented (`ExternalTextureTransferFunction`, YUV). It is **not** a way to wrap an XR compositor's `GPUTexture`. |
| `wgpu` + WebXR upstream | Issue #8329 ("document how to integrate wgpu with a webxr app", Oct 2025) is open, labelled *help required*, no maintainer answer. |
| `bevy_webxr` | Placeholder crate. Functionality not filled in. |
| `bevy_mod_openxr` 0.6.0 | Real, maintained (updated 2026-06-20), MIT/Apache-2.0 — but **native OpenXR only**, not the browser. |

### What this means concretely

Rust → wasm → WebXR is possible **today** only via WebGL2: you drive the session
with `web-sys` and render into `XRWebGLLayer`'s opaque framebuffer using `glow`
or raw GL calls. Note the known papercut — `XRWebGLLayer.framebuffer` returns
`None` through `web-sys` because opaque framebuffers report null while still
being bindable (wasm-bindgen issue #2864); you need a small JS shim.

The WebGPU-in-VR path exists but is pioneer territory: `renzora/wgpu-webxr-webgpu`
demonstrates wgpu rendering directly into XR compositor textures with no WebGL and
no texture copies, using a small wgpu fork that exposes the underlying `GPUDevice`
and wraps external `GPUTexture` objects, plus hand-written `#[wasm_bindgen]` externs
for `XRGPUBinding` / `XRProjectionLayer` / `XRGPUSubImage`. **Caveat: I could not
fetch this repo directly (404 on both the repo root and raw README) — it may have
been renamed or removed. Its license is unverified. Treat it as a technique
reference to reproduce, not a dependency to add.**

---

## 2. Recommended architecture: one core, three front-ends

Do not pick a single rendering target. Pick a **core that does not know what a
renderer is**, and put a thin seam where the platform differs.

```
        ┌──────────────────────────────────────────────────────┐
        │  sim-core  (Rust, no_std-friendly, zero GPU deps)     │
        │  orbital mechanics · faction state · economy · save   │
        │  deterministic, fixed-timestep, fully unit-testable   │
        └───────────────────────┬──────────────────────────────┘
                                │  (plain data snapshots)
        ┌───────────────────────┴──────────────────────────────┐
        │  render-core  (wgpu + WGSL, platform-agnostic)        │
        │  one shader library, one frame graph, one material set │
        └──┬──────────────────┬─────────────────────┬───────────┘
           │                  │                     │
    ┌──────┴──────┐   ┌───────┴────────┐   ┌────────┴─────────┐
    │ native-flat │   │  native-xr     │   │  web             │
    │ Mac+Windows │   │  Windows       │   │  wasm            │
    │ Metal/DX12/ │   │  OpenXR →      │   │  WebGPU (flat)   │
    │ Vulkan      │   │  Index, Quest  │   │  WebGL2 (WebXR)  │
    │ full compute│   │  Link, Steam   │   │  XRGPUBinding    │
    │             │   │  Frame         │   │  when it lands   │
    └─────────────┘   └────────────────┘   └──────────────────┘
```

**Why this split and not "just the web":**

- macOS has no OpenXR runtime. VR on Mac is not a target anyone can hit natively —
  Mac is a *flat* platform for this project. Say so publicly and early.
- Valve Index and Steam Frame are best served by native OpenXR (`openxr` crate 0.21.1,
  MIT/Apache-2.0). Going through the browser costs you compute shaders and gains
  you nothing on a PC that already has SteamVR.
- Quest standalone can be reached two ways — a native Android build via OpenXR,
  or the browser. Ship the browser build first (zero-install is your distribution
  advantage), keep native as the performance escape hatch.
- The web build is where "click a link and you're in the game" lives. That is
  worth more to an open-source project than 20% more shader budget.

**The seam that matters:** define one trait, e.g. `XrBackend`, with
`views() -> [ViewProjection; N]`, `begin_frame()`, `submit(view, target)`. Native
OpenXR, WebXR/WebGL2, and future WebXR/WebGPU are three implementations. Everything
above the seam is written once. Get this trait right in week one and the three-target
story stays cheap forever.

---

## 3. Verified dependency shortlist

You asked specifically for MIT. Almost nothing serious in Rust is MIT-only — the
ecosystem norm is `MIT OR Apache-2.0`, which is *more* permissive than MIT alone
(you may take the MIT branch). Flagged below where that is not true.

| Component | Pick | License (verified) | Note |
|---|---|---|---|
| GPU abstraction | `wgpu` 30.0.0 | MIT OR Apache-2.0 | ✅ Native + WebGPU + WebGL2 from one codebase |
| Shader IR / translation | `naga` 30.0.0 | MIT OR Apache-2.0 | ✅ WGSL → SPIR-V/MSL/HLSL/GLSL |
| ECS | `bevy_ecs` 0.19.1 (standalone) or `hecs` 0.11.1 | MIT OR Apache-2.0 | ✅ `bevy_ecs` usable without the full engine |
| Full engine (if wanted) | `bevy` 0.19.1 | MIT OR Apache-2.0 | ✅ Batteries-included; costs you frame-graph control |
| Native VR | `openxr` 0.21.1 / `bevy_mod_openxr` 0.6.0 | MIT OR Apache-2.0 | ✅ Index, Quest Link, Steam Frame |
| Web bindings | `web-sys` 0.3.104 | MIT OR Apache-2.0 | ✅ WebXR present; XRGPU types absent |
| WebGL2 in Rust | `glow` 0.18.0 | MIT OR Apache-2.0 OR Zlib | ✅ For the WebXR-today path |
| Windowing | `winit` 0.31 | Apache-2.0 | ⚠️ Apache-only, not MIT |
| Physics | `rapier3d` 0.35.2 | Apache-2.0 | ⚠️ Apache-only. Jolt (MIT, C++) is the MIT alternative but adds a C++/wasm build |
| Narrative scripting | Yarn Spinner (`yarnspinner` 0.9.0) | MIT OR Apache-2.0 | ✅ Native Rust port, actively released (Aug 2026) |
| Narrative alt | ink (inkle) / `bladeink` 2.0.0 | ink: MIT · bladeink: Apache-2.0 | ⚠️ The Rust runtime is Apache-only |
| Multiplayer (later) | Colyseus | MIT | ✅ Node.js, authoritative rooms, binary delta sync |
| Star catalogue | AT-HYG / HYG v4 | **CC-BY-SA 4.0** | ⚠️ Share-alike **on the data**. Doesn't infect your MIT code, but you must redistribute the data file under BY-SA |
| Atmosphere reference | `Dimev/atmosphere-shader`, `sinnwrig/URP-Atmosphere` | MIT | ✅ Good study material; port, don't vendor |

### Do NOT copy code from these
Naev, Endless Sky, Pioneer, Oolite, Vega Strike are **GPL**. They are excellent
*design* references — read them, play them, take mechanics ideas — but a single
copied function makes your MIT release non-distributable. Keep a hard rule: nobody
opens their source while writing yours. Orbiter (MIT) is the one you *can* borrow
orbital-mechanics code from.

If you want a fully permissive star map, generate one procedurally from a seeded
PRNG instead of shipping AT-HYG. Real catalogues buy you authenticity in a ~100 ly
bubble; beyond that everything is invented anyway.

---

## 4. "Made from shaders" — making it actually run at 90 Hz

The romantic version of this idea (raymarch everything) dies in VR. Budget: 90 Hz
stereo means **~9–11 ms for both eyes**, and Quest-class hardware is a phone. A
full-screen SDF raymarch per eye will not fit. The version that works:

1. **Distance-banded representation.** Stars → GPU-instanced point sprites with an
   analytic spectral-class-to-colour function (millions, one draw call). Distant
   planets → analytic sphere impostors shaded in the fragment shader. Near planets →
   raymarched/tessellated terrain. Cockpit and ships → actual meshes. The band a
   body sits in is a function of angular diameter, evaluated on the CPU per frame.
2. **Atmospheres via precomputed LUTs, not per-frame raymarching.** Bruneton-style
   transmittance + scattering tables computed once in a compute shader, sampled in
   two texture fetches. This is the single biggest win available to you.
3. **Volumetrics at quarter resolution + temporal reprojection.** Nebulae and dust
   raymarch into a ¼-res buffer, reprojected with the previous frame's motion
   vectors, upsampled bilaterally. Never at native stereo resolution.
4. **Floating-origin, f64 on the CPU, f32 on the GPU.** A galaxy in f32 world space
   jitters catastrophically. The sim keeps `f64` (or fixed-point) absolute positions;
   the renderer receives camera-relative `f32`. Non-negotiable, and painful to
   retrofit — do it on day one.
5. **Foveation.** Quest supports Fixed Foveated Rendering; Godot-class engines expose
   Quad View. With wgpu you drive this yourself via OpenXR foveation extensions on
   native. Assume you will need it.
6. **One shader source of truth.** Write WGSL. `naga` translates to MSL/SPIR-V/HLSL
   for native and GLSL for the WebGL2/WebXR path. Do not maintain a second GLSL set
   by hand — feature-gate instead (`#ifdef`-equivalent via naga preprocessing or
   shader permutation keys), and keep a "WebGL2 subset" lint: no compute, no storage
   buffers, limited texture units.

---

## 5. Choice-driven RP — the design question that actually decides scope

Elite Dangerous and EVE achieve "choices matter" by completely different means, and
conflating them is the classic way this genre of project dies:

- **EVE**: choices matter because *other players* are the consequence engine. This
  requires a persistent authoritative server, an economy with real scarcity, and a
  population. Population is the hard part, and it is not a technical problem.
- **Elite Dangerous**: choices matter through the **Background Simulation** — faction
  influence values per system, mutated by aggregate player action, driving states
  (boom, famine, war, lockdown) that change what the world offers you.

**Recommendation: build the Elite-style BGS first, single-player, deterministic,
offline.** It is tractable for a small team, it is testable (you can fast-forward
1000 ticks in CI and assert the economy didn't collapse), and it produces "my choices
changed the world" without needing a single other human online. Design the tick
function so it *could* later run server-side for shared state — that is a deployment
change, not a rewrite.

Then layer narrative on top: Yarn Spinner (MIT/Apache) for authored branching content,
with the BGS state exposed to it as variables, and story outcomes writing back into
faction influence. Authored narrative reacting to simulated state, and mutating it,
is where "roleplay-esque, choices matter" actually lives — neither system alone gets
you there.

**Scope discipline:** the failure mode is a beautiful renderer with no game. Define
one 20-minute vertical slice — one star system, three factions, one story arc with
three real endings, flyable in flat and in VR — and refuse everything outside it
until it ships.

---

## 6. Risk register — spikes to run before writing game code

Run these as throwaway prototypes. Each has a kill/confirm criterion.

| # | Risk | Spike | Kill criterion |
|---|---|---|---|
| R1 | **Rust→wasm→WebXR is not viable** | Minimal `web-sys` + `glow` immersive-vr session, render a triangle in stereo on Quest 3 browser and on Index via SteamVR/Chrome | Can't hit stable 72 Hz on Quest with a trivial scene → web VR is not the primary VR target; native OpenXR becomes primary |
| R2 | **wgpu can't reach the XR framebuffer** | Try wgpu WebGL2 backend into `XRWebGLLayer`'s opaque FBO; if blocked, fall back to raw `glow` for the XR path only | If wgpu can't do it and a fork is required → decide explicitly: fork, or run a separate `glow` renderer for web-VR |
| R3 | **Shader budget doesn't fit stereo** | Port one atmosphere LUT + one raymarched nebula, measure GPU ms/eye on Quest 3 and on a mid GPU | >8 ms/frame stereo at target res → cut volumetrics to impostors before building content on them |
| R4 | **f32 precision** | Fly 10^12 m from origin, check for jitter | Visible jitter → floating origin was implemented wrong; fix before anything else |
| R5 | **Wasm bundle size / load time** | Measure release wasm + assets, gzip/brotli | >20 MB or >5 s to playable → strip, split, stream |
| R6 | **XRGPUBinding timeline** | Reproduce the wgpu-fork WebGPU-XR technique behind a feature flag | Doesn't work in Chrome Canary with both flags → shelve; revisit when the spec reaches CR |

R1 and R3 together decide whether this project is "a web game that also runs in VR"
or "a native VR game that also has a web build". **Do not write the plan's milestone 2
until those two spikes have answers.**

---

## 7. Milestones

- **M0 — Spikes (2–3 weeks).** R1–R5 above. Output: a one-page verdict on renderer
  topology. Nothing shipped.
- **M1 — Skeleton.** Workspace with `sim-core` / `render-core` / three front-end crates.
  The `XrBackend` trait. Floating origin. One star, one planet, one ship, flyable
  flat on Mac + Windows + web. CI building all three targets.
- **M2 — Looks like the game.** Atmosphere LUTs, instanced starfield, nebula volumetrics,
  cockpit. VR on whichever path M0 chose. Frame-time budget enforced in CI.
- **M3 — Is a game.** BGS tick, three factions, economy, Yarn Spinner dialogue,
  save/load, the 20-minute vertical slice with three endings.
- **M4 — Open source release.** MIT (dual MIT/Apache-2.0 recommended for Rust-ecosystem
  compatibility — it's what wgpu, bevy, and the whole dependency tree use). CONTRIBUTING,
  a licence audit of the full tree via `cargo-deny`, and an honest README about
  which platforms actually work.

---

## Sources

- WebXR/WebGPU Binding Module L1 (Editor's Draft, 15 Jun 2026) — https://immersive-web.github.io/WebXR-WebGPU-Binding/
- Toji, "Experimenting with WebGPU in WebXR" — https://toji.dev/2025/03/03/experimenting-with-webgpu-in-webxr.html
- Meta Horizon OS web release notes — https://developers.meta.com/horizon/release-notes/web/
- wgpu #8329, "document how to integrate wgpu with a webxr app" — https://github.com/gfx-rs/wgpu/issues/8329
- wasm-bindgen #2864, opaque framebuffers in WebXR — https://github.com/rustwasm/wasm-bindgen/issues/2864
- three.js #30806 (WebGPU in WebXR) and #32538 (multiview) — https://github.com/mrdoob/three.js/issues/30806
- caniuse WebGPU — https://caniuse.com/webgpu
- crates.io / docs.rs license + API verification, 2026-08-19
- AT-HYG database (CC-BY-SA 4.0) — https://www.astronexus.com/projects/at-hyg
- Meta WebXR performance workflow — https://developers.meta.com/horizon/documentation/web/webxr-perf-workflow/
