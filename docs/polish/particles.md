# particles — fine particle and liquid effects (branch `fable/particles`)

What this branch did, how it was verified, what is left. Captures live in
`C:\Users\jayja\farfall-captures\particles\` (800×600 bench frames; `*-crop.png`
are ffmpeg zooms of the same frames).

## Why the baseline showed nothing

- **Ghost and strikes were expired before the capture.** `FARFALL_BENCH_GHOST` and
  `FARFALL_BENCH_STRIKES` staged their moments against a capture at t = 1 s, but the
  bench captures halfway through `FARFALL_BENCH_SECONDS` (2 s with bench.sh), past the
  1.8 s ghost life and with the 5 m/s ripples already round the back of the shell.
  `bench_capture_s()` in `crates/app/src/lib.rs` now ages both against the real
  capture time. Knob docs updated.
- **No chase-view plume.** `jet_uniforms` used the live `effort` (zero in a frozen
  bench) and ignored `FARFALL_BENCH_THRUST`; jet.wgsl only painted a glow ring on the
  nozzle lip. Both fixed: a shared `Game::thrust_look()` feeds the cabin and the chase
  view, and the jet pass draws real plumes.
- **The shield's honeycomb never drew.** `honeycomb()` normalised the cell chart so
  no point ever reached a hex edge (edge distance always ≥ 0.5 of a cell), and the
  hex SDF was flat-topped against a pointy-topped lattice. Fixed (apothem = cell/2,
  axes swapped). This was latent in the live game too.

## What a player will notice

1. **Space dust** (`crates/render/src/dust.rs`, `shaders/dust.wgsl`, `DUST` row on
   GRAPHICS, `graphics.dust`): motes on a 40 m world lattice, 7×7×7 cells × 12, one
   instanced draw. Sun-lit ice glints (twinkle + forward-scatter halo), grit in the
   belt, streaks one frame long from the eye's velocity **relative to the local
   circular orbit** (`dust::drift`) so a coasting ship's motes hang still and thrust
   streams them. Density: 10 % floor, the ring (`Belt::ring_density`, 1 inside,
   fading over 5 km), a planet's air (√(ρ/ρ0)), ×0 under the hyper field. Hidden
   behind the nearest body. Cabin motes (40) drift between the head and the dash in
   its cyan light, drawn after the cabin composite. Captures: `dustonly-1`
   (rest, sparse), `dustonly-belt-1` (dense), `dustonly-fast-1` and `dust-fast-1`
   (3 km/s streaks), `belt-1`.
2. **Engine plumes** (`jet.wgsl`, `cabin_blit.wgsl`, `holo.wgsl`): white-hot core with
   shock diamonds, translucent blue-violet rippling skin, amber nozzle mouths, blue
   wash on the nacelle tails and (first person) on the hull aft; RCS puffs flicker;
   hull has seams, Fresnel and the planet's light on the belly. Captures: `thrust-1`,
   `thrust-crop`, `thrust-rcs-1`, `aft-thrust-1`, `holo-thrust-1`/`holo-crop`.
3. **After-image** (`ghost.wgsl`): chromatic rim fringe, caustics flowing down its
   length, starts nearer (10 + 42 m). Capture: `ghost-1`, `ghost-crop`.
4. **Shield** (`shield.wgsl`): a wave with a white crest, caustic-lit honeycomb behind
   it; under hyper a liquid sheen and violet graze. Captures: `strikes-1`,
   `strikes-alone-1` (sky passes skipped), `hyper-1`. The last honeycomb fix
   (orientation) built and validated but its capture was blocked by the bench HALT —
   see "Left".
5. **Holograms**: interference shimmer on the holo3PP and the SHIP bay
   (`bay-1`). **Belt**: rock-hit bursts throw fine glowing grit (`arms-1`,
   `arms-crop`); the ring's far specks glint as facets turn (`belt-1`).

## Cost (FARFALL_BENCH_FULL=1 FARFALL_VSYNC=off, the display is 1920×1080 here,
4×MSAA, interleaved A/B, noisy because a game was running)

| scene | with | without | pass |
|---|---|---|---|
| reference orbit, dust 10 % | 1.52 ms | 1.51 ms | dust ≈ 0.01–0.02 ms |
| in the belt, dust 100 % | 1.53 / 2.57 ms | 1.38 / 2.23 ms | dust ≈ 0.15 ms (instance-bound, not fill-bound: expect ≈ 0.2 ms at 2880×1800) |
| chase + full thrust + RCS | 1.26 / 1.56 ms | 1.28 / 1.31 ms | jet plumes ≈ 0.1–0.25 ms |
| 3 strikes + ghost | 1.30 ms | 1.19 ms | shield + ghost ≈ 0.1 ms |

Logs: `perf-*.log` in the captures dir.

## Decisions

- Dust rests in the local circular orbit, not the inertial frame: otherwise every
  orbit is a 7 km/s blizzard. The ring arrival is exactly that orbit, so belt motes
  hang still there, as the rocks do.
- Dust is drawn before the jet pass (the ship hides motes behind it; motes in front
  of the hull are lost — rare, small) and cabin motes after the cabin composite.
- Additive passes (dust, ghost, shield) write radiance without a tonemap, crests
  and cores > 1; jet/cabin keep their existing `tonemap()` so the postfx merge is
  a mechanical strip. Values were chosen to read on the current LDR target too.
- Settings row lives on the GRAPHICS page next to LENS FLARE (25 % steps, OFF at 0)
  — the hud-menu agent owns the layout; only the row was added.
- The bench's capture time is read from `FARFALL_BENCH_SECONDS` in `bench_capture_s()`
  rather than threaded through `Config`, to keep the `lib.rs` diff small.

## Left

- Bench captures were HALTED (locks/HALT) before the final shield honeycomb capture
  and the hyper recapture; `strikes-alone-crop.png` on disk is from the build
  *before* the axis fix (shows the star-lattice bug). Re-run when lifted:
  `FARFALL_BENCH_STRIKES=3 FARFALL_SKIP=starfield,nebula,bodies,planet` and
  `FARFALL_BENCH_HYPER=1`.
- The stars are still soft blobs, so at 10 % the motes are hard to tell from them
  until the starfield/postfx work lands; in `dustonly-*` (sky skipped) they read.
- No per-pass GPU timestamps exist; costs are by absence (FARFALL_SKIP). A real
  2880×1800 number needs that display.
- Belt dust could co-rotate with the ring's drift (a few m/s) rather than the pure
  circular orbit; not done, the difference is a slow creep.
