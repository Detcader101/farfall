# fable/postfx — the HDR picture, the stars, the drive outside the glass, the shake

Branch `fable/postfx`, worktree `farfall-wt/postfx`, from `fable/polish` (ea42028).
Captures in `C:\Users\jayja\farfall-captures\postfx\` (baseline in `..\baseline\`).

## What changed, for the player

- **The picture is HDR.** Everything outside the glass renders into a float
  target and writes real radiance; one post pass does bloom, exposure, a filmic
  curve, a hair of chromatic fringing at the rim of the glass, and a dither to
  8 bits. The Sun, the muzzle flash, the tracers, the engine nozzles and the
  brightest stars bloom; a lit hull, the dash and the nebula never do.
  Highlights roll into white through their own hue (AgX) instead of clipping to
  flat white/cyan (compare `baseline/arms-1.png` → `postfx/arms-1.png`).
- **GFX page rows:** BLOOM (0–200%), EXPOSURE (−2..+2 EV in quarter stops),
  TONEMAP (OFF / SOFT / AGX), FRINGE (0–200%). Keys `graphics.bloom`,
  `graphics.exposure`, `graphics.tonemap`, `graphics.fringe`.
- **Stars are points.** Sub-pixel gaussian cores (σ ≈ 0.6 px, was 1.1) and a
  steep magnitude law — most stars a fiftieth to a third of white, a few per
  screen past white (those bloom), the rare one blazing — with the tint by
  temperature strongest on the bright ones. The "snow" of `baseline/cockpit-1.png`
  is gone; the nebula carries the sky (`postfx/cockpit-1.png`, `nebula-1.png`,
  `p2880-belt-1.png`).
- **The drive is outside the ship.** The wormhole's mirror-sphere flip and
  inversion, the chaos field's liquid refraction and ripples, the speed streaks
  and their chromatic split, the cool rim — all of it is now done to the world
  *before* the cabin, the dials, the holo3PP, the map, the bay and the text are
  drawn. Through a hyper run the dash and the gauges are crisp and steady
  (`postfx/hyper-1.png` vs `baseline/hyper-1.png`, where they were smeared).
- **Less shake.** `cam.drive-shake` stock 40% → 12%; the helmet camera's sway
  and roll per g, tremor and gun kick at about a third of what they were, its
  cap 0.20 → 0.12 rad. CAMERA SHAKE and DRIVE SHAKE on the CABIN page still go
  to 200%. The gauges share the dash's head frame (`Game::head()`), so they
  cannot move relative to it.

## How it is built

```
scene pass   world passes (starfield … ghost)  →  MSAA Rgba16Float  →  resolve → world HDR
post chain   prefilter (½ res: 13-tap, soft knee at 1.15 radiance, partial Karis, α = log lum)
             down ×5 (13-tap)  ·  adapt (1×1: mean log lum, eased τ = 1.4 s)  ·  up ×5 (9-tap tent, ONE+ONE)
ship pass    post main draw (the drive's distortion → fetch world + bloom → exposure × drift
             → tonemap → dither) into the MSAA 8-bit ship target, THEN cabin, horizon, dials,
             guide, sight, holo3PP  →  resolve → scene colour
present      blit (plain upscale) → map, bay hologram, HUD text, pointer at native res
```

- `crates/render/src/lib.rs` `SceneTarget`: two layers, one sample count —
  `world_attachment()` (Rgba16Float) and `colour_attachment()` (swapchain
  format, as before, so captures/readback are untouched). `WORLD_FORMAT`.
- `crates/render/src/post.rs` + `shaders/post.wgsl`: `PostPass` owns the chain
  (six levels from half the scene, so its cost follows the render-scale
  governor), the 1×1 adapted-luminance ping-pong, and the main draw.
  `begin_ship_pass()` encodes the chain and returns the ship pass with the
  world already painted, so `lib.rs` only splits its one scene pass in two.
- `shaders/common.wgsl`: `radiance(col, exposure)` for the world passes (linear,
  clamped at 4096 for the half-float resolve); `tonemap()` kept for the cabin,
  which draws after the post pass and curves itself.
- `shaders/blit.wgsl` is a plain upscale now; the drive's look moved out of it.
- Exposure: the setting × a drift `clamp((0.12 / L_geo)^0.35, 0.70, 1.30)` on
  the frame's geometric-mean luminance — a starry sky is lifted a third of a
  stop at most, a sunlit planet held back half — landing at once on the first
  frame so benches are reproducible.
- AgX: Wrensch's minimal fit (inset, log2 over 16.5 stops, 6th-order sigmoid,
  outset, 2.2 power back to linear for the sRGB framebuffer) with a mild
  punch (contrast 1.12, saturation 1.18).
- Fringe: the world fetched at ±0.0035·fringe·glass² uv along the radial, glass
  = smoothstep(0.55, 1.45, rim radius) — nothing at the centre, a hair at the
  rim, added to the drive's own split.
- New bench knobs: `FARFALL_SKIP=post` (one fetch, nothing done) and
  `FARFALL_SKIP=bloom` (no chain); `FARFALL_BENCH_SIZE=w,h` makes a window of
  exactly that many pixels, bigger than the display if need be — how the
  2880×1800 numbers below were taken on a 1080p desk.

## Verification

Windowed 800×600 captures, all looked at: `cockpit`, `hyper`, `nebula`, `arms`,
`alt500`, `chase`, `thrust`; full-size `p1080-*` and `p2880-*` for the perf set.

Perf (`FARFALL_VSYNC=off`, 6 s, RTX 2080 Ti, desk shared with a running game so
the 1% lows are noisy; frame averages are the signal):

| scene | 1920×1080 | 2880×1800 |
|---|---|---|
| default (12 km, nose on the planet) | 1.82 ms (549 fps) | 4.09 ms (245 fps) |
| FARFALL_SKIP=post | 1.76 ms | 3.38 ms |
| FARFALL_SKIP=bloom | 1.70 ms | 4.41 ms (noise) |
| alt 500 m | 1.75 ms | 4.59 ms |
| nebula | 1.95 ms | 4.73 ms |
| hyper | 3.25 ms | 4.80 ms |
| belt (FARFALL_SPAWN=belt) | 4.27 ms | 3.97 ms |

A second, interleaved A/B at 2880×1800 (`ab-*.log`) landed at 5.3 / 5.3 / 4.0 ms
(base / skip post / skip bloom) with 40–65 ms worst frames from the other game on
the desk — so the post pass's own cost at 2880×1800 is inside the run-to-run
noise (< ~0.7 ms); it wants a quiet desk or GPU timestamps for a firm number.
Baseline default at 1080p was 1.50 ms: the HDR target (a second resolve, 16F
bandwidth) plus the post pass cost ~0.3 ms at 1080p; at 2880×1800 the post pass
itself measures ~0.7 ms by its absence. Worst scene 4.8 ms against the 16.7 ms
floor — 3.5× headroom.

Gate: `cargo test --workspace`, fmt, clippy `-D warnings`, wasm check — green.

## Decisions

- **Ship layer stays 8-bit and MSAA.** The cabin/dial pipelines keep the
  swapchain format and sample count, so nothing about how they look changed,
  `FARFALL_CAPTURE` readback is unchanged, and the only extra GPU work is one
  resolve at scene scale. The alternative (dials in the present pass) would
  have cost them MSAA.
- **Holograms don't bloom.** The holo3PP and the SHIP bay hologram sit on the
  ship side of the post pass by design (they must not warp with the drive), so
  their glow stays analytic in their own shaders. If a bloomed hologram is
  wanted, it needs its own small chain — not done.
- **TONEMAP has three states, not two.** OFF is an honest clip; SOFT is the old
  `1 − e^−x` for A/B; AGX is stock.
- **BLOOM 0% still runs the chain** because the exposure meter rides in it;
  `FARFALL_SKIP=bloom` is the profiling switch.
- **Star density unchanged** (55% of cells); the fix was the footprint and the
  law, not the count.
- The `FARFALL_SCENE_TESTS=1` golden-image scenes will differ from their
  references (the whole picture changed); regenerate them after the merge.

## What is left

- The planet's daytime sky and the 500 m ground are still low-contrast haze —
  that is the planet/atmosphere pass, not the post; it reads better under AgX
  but wants its own pass (perf/particles agents).
- Engine plumes: the nozzle glow writes radiance 2.4–3 and blooms under
  thrust; the plume body itself is the particles agent's.
- A bench window over the display occasionally takes a stray keypress from
  whatever Jay Jay is playing (one run captured on the P key and lost its
  readout to a click) — re-run if a capture looks impossible.
