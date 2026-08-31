# fable/warp — the eight-second liquid wormhole jump

Branch `fable/warp`, worktree `farfall-wt/warp`, from `fable/polish` (74347db+).
Captures in `C:\Users\jayja\farfall-captures\warp\`.

## What changed, for the player

Jay Jay's ask: *"I need the warp to take longer, like an 8 second long
animation of the world around your ship getting stretched warped, inverted,
pulled warped inside out and liquid/fluid into the new destination — with all
gpu shaders."*

The jump (J / map ENGAGE) is now four phases flowing into each other over
eight seconds, every effect a shader on the world image alone — the dash,
dials and text stay crisp throughout:

- **CHARGE (2 s)** — space starts to stretch radially: the stars draw out
  into threads away from the nose, a liquid shimmer builds at the rim, the
  drive's cold glow climbs. The lens itself only eases 10% wider — the
  stretch does the talking.
- **PULL (2 s)** — the picture warps harder: the star threads reach full
  length, the chromatic split grows, and the view folds in toward the nose
  with a slow turn (a new fold term in `post.wgsl`, hardest partway out).
- **FLIP (1.5 s)** — the old mirror-sphere inversion made long and liquid:
  the world turns through itself, fisheye through colour-negative with the
  liquid field at full, and the sim's one jump fires exactly at the peak,
  when the view is fully inside out. A hyper SLIP still enters here directly.
- **REFORM (2.5 s)** — the destination pours in as fluid: settling rings run
  out from the eye, the new sky un-warps through liquid swirls and chromatic
  fringes that ease to stillness, the glow dies.

**WARP LENGTH** on the DRIVE page (key `warp.length`, 50–200%, stock 100% =
8 s, shown as `100% (8S)`) scales the whole sequence; a menu edit mid-jump is
ignored so a running sequence keeps its shape. The CHAOS drive (H) keeps its
continuous look and shares the same lanes; WARP STOP's ghost is untouched.

## How it is built

- `crates/app/src/warp.rs`: the phase machine grew a `Pull` phase and a
  `length` scale; `Look` grew `stretch` / `pull` / `reform` lanes. Every
  phase's look is value-continuous into the next (a test walks the whole
  sequence asserting no field steps more than 0.1 per 10 ms).
- `crates/render/src/post.rs` + `shaders/post.wgsl`: `PostUniforms` gained a
  `drive` vec4 (stretch, pull, reform — all clamped, all zero when idle).
  The shader adds the PULL fold (an inward radial bend with a slow rotation),
  the REFORM settling rings (`sin(r·42 − t·6.5)·e^(−1.6r)` displacement) and
  a reform-driven boost of the existing liquid field, split and rim wash.
- `stretch` is also fed app-side into the existing speed lane
  (`speed_look()` maxes it in), so the starfield's own streak taps draw the
  star threads — the postfx agent's Star-Trek streaks, reused as vocabulary.
- Shake untouched (`shake.rs` / `look.rs` / the warp jostle are the design
  agent's); the sequence reads correctly with shake at 0.

## Verification (all captures looked at)

`FARFALL_BENCH_WARP=s` stages the sequence; captures at 1, 3, 4.5, 6, 7.5 s:

- `seq-s1` charge threads + rim shimmer; `seq-s3` full pull, split visible;
  `seq-s45` fully inside-out (inverted sky, dark well, caustic shards);
  `seq-s6` the new sky pouring in through fringed liquid; `seq-s75` nearly
  still. `spin-1..4` (SECONDS=8, SPIN=4): the head turning through the
  sequence — spin-2/3 land mid-PULL/FLIP and stay view-coherent.
- `seq-after` (9.5 s in): the sequence's own contribution is gone — the
  residual star-trails there are the arrival scene itself (a 9.5 km/s solar
  orbit through dust), identical before and after this change.
- Perf: `perf-full` — 2880×1800 4×MSAA, vsync off, the whole sequence in
  frame: **5.83 ms avg (171 fps)**, mid-sequence readout 156 fps; worst
  frames are shared-desk noise. The 60 fps floor has ~3× headroom.
- Gate: `cargo test --workspace` (all green, 154 in farfall-app incl. 3 new
  phase-machine tests + 1 post-lane test), fmt `--check`, clippy
  `-D warnings`, wasm check — all green. Sim untouched.

## Decisions

- Four *visual* stages, four machine phases: `Pull` is a real phase (the
  monotonic-order test wants honest states), and the named test
  `the_sequence_jumps_exactly_once_at_the_flips_peak` was retimed
  (`CHARGE_S + PULL_S + FLIP_S*0.5`), never duplicated.
- The FOV peak stays 1.35 (the old test's "a touch wider, no more" bound):
  the eight seconds come from the picture, not the lens.
- `warp.length` scales durations, not dt, so pausing/clamping behaviour is
  unchanged; NaN → stock; set is refused mid-sequence.

## What is left

- Sound: the swell now rides the 4 s charge ramp automatically (it reads
  `Look::charge`), but a bespoke four-phase score (a PULL groan, a REFORM
  wash) would sell it harder — audio agent's lane.
- The arrival scene's own dust streaks at a fast orbital arrival read a
  little like the sequence continuing; if it bothers Jay Jay it is the dust
  pass's speed response, not the warp's.
