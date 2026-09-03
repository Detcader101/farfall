# FARFALL polish pass — plan and art direction (2026-08-31)

Owner of the brief: Jay Jay. Executed by Claude Fable 5 across parallel agents,
each in its own git worktree/branch, merged into **`fable/polish`** (the
integration branch; worktree at `C:\Users\jayja\farfall-wt\polish`).

## The ask, verbatim in spirit

> Gather everything, benchmark the current state, then make every piece more
> efficient and cleaner / higher definition. Fill out every feature; make sure the
> HUD isn't cutting off or displaying wrong, and that the menus include every
> feature and keybind — the things that would turn off a new player who doesn't
> know FARFALL. Make it nice to look at, to market it eventually. All GPU shaders.
> Ideal: a modern Star Trek / Star Wars feel with many fine particle and liquid
> effects that read as HDR and surreal; detailed HUD and world elements that don't
> clash — a detailed scene that is easy to pick out yet full of content.

## Art direction (the taste rules every agent works to)

1. **HDR first.** The scene target becomes a float (RGBA16F) HDR buffer; every
   emitter (Sun, stars, engine plumes, holograms, tracers, drive field) writes
   real radiance > 1.0; one post pass does bloom (physically soft, wide, never a
   blob), exposure (fixed per-scene with a slow auto-exposure drift), a filmic
   tonemap (AgX-style: highlights roll into white through their hue, never clip
   to flat), and a hair of chromatic fringing at the glass rim only. Dither to 8-bit.
2. **Fine, not loud.** Particles are small, numerous, and *slow* in the dark:
   drifting dust motes lit by the cabin, ice crystals in the sunlight, a thin
   plasma skin on the plume. Nothing in space swirls like smoke. Liquid effects
   belong to the drives and the shield (refraction, caustic sheen, chromatic
   rims), and to the holograms' interference shimmer.
3. **The HUD is glass, not paper.** Thin lines, high-contrast cyan/ivory on
   dark, amber only for warnings, red only for danger. A proper readable font
   (5×7 minimum, anti-aliased by SDF or supersample) for every readout and menu.
   Layout math stays in Rust under test; shaders only answer "is this pixel lit".
4. **Nothing cut off, ever.** Every text element is measured against its panel
   in a test. Every menu row fits, every tab is visible, every bind shows its
   full key name, every page scrolls with a visible scrollbar and count.
5. **A new player can find everything.** A first-run CONTROLS card (dismiss
   with any key; re-open with F1 / from the menu), a HELP page in the menu
   that lists every control by group with a one-line "what it does", and the
   menu itself laid out so a stranger can read it at 800×600 and at 2880×1800.
6. **60 fps floor at 2880×1800 4×MSAA** holds. Every new pass reports its cost
   in the frame stats; anything over its budget governs itself down like the
   cabin does. Measured with `FARFALL_VSYNC=off FARFALL_BENCH_FULL=1`.
7. **Still a shader game**: no image assets, no fonts on disk. Determinism of
   the sim is untouched — render-only work.

## Workstreams (one branch each, from `fable/polish`)

| Branch | Worktree | Scope |
|---|---|---|
| `fable/hud-menu` | `farfall-wt/hud-menu` | Font upgrade, readout layout, menu panel (all tabs, scrolling, full key names, HELP page, first-run card), every bind and setting present |
| `fable/postfx` | `farfall-wt/postfx` | HDR scene target, bloom, exposure, AgX tonemap, glass fringing, dither; GFX rows for each |
| `fable/particles` | `farfall-wt/particles` | Space dust / motes / ice crystal pass, plume plasma skin, drive & shield liquid upgrades |
| `fable/perf` | `farfall-wt/perf` | Per-pass profiling, the expensive passes made cheaper without visible loss |

Merge order: perf → postfx → particles → hud-menu (the HUD touches the most
of `lib.rs`; it merges last onto a stable base). Each branch commits often with
descriptive messages; each leaves `docs/polish/<branch>.md` describing what it
did, how it verified it (capture names), and what is left.

## Verification

- `C:\Users\jayja\farfall-captures\bench.sh <exe> <outdir> <name> [ENV=VAL...]`
  runs the Windows exe from WSL with env passed through, moves the capture PNGs
  into `<outdir>/<name>-N.png`, and prints the perf line. Baseline set in
  `farfall-captures/baseline/` (see `BASELINE.md` beside it).
- Gate: `cargo.exe test --workspace`, `cargo.exe fmt --all --check`,
  `cargo.exe clippy --workspace --all-targets -- -D warnings`,
  `cargo.exe check --workspace --target wasm32-unknown-unknown`.
