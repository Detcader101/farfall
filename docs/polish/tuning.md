# tuning — what the merge looks like, fixed (branch `fable/tuning`, 2026-08-31)

Scope: the devlog's critic list against the merged `fable/polish` (51eafe2). Every
capture named here is in `C:\Users\jayja\farfall-captures\tuning\` (800×600,
stamped); the devlog frames it answers are in `..\devlog\2026-08-31-1331\`.

## 1. The holo3PP sat over the two right-hand gauges

`HOLO_ANCHOR_DEFAULT` `(0.52, −0.46)` → `(0.81, 0.30)`, `holo.size` 0.24 → 0.18.
The hologram floats in the glass at the upper right, under the mini map and
outside the arch, clear of all five dials and of the forward view; over the
dash there is no gap between the dials to stand it in. A dragged anchor still
goes anywhere. `settings.rs::the_stock_hologram_sits_clear_of_the_dials_and_the_mini_map`
pins the geometry at 4:3 and 16:9 (it caught my own first guess: `HOLO_RADIUS_M`
is 0.7 m, not 0.5, so 0.22 reached the glass edge). The scene test
`the_holo3pp_stands_as_a_3d_hologram_over_the_dash` measures the new region.

Proof: `r4-holo-1.png`, `r4-alt12k-1.png` vs devlog `gfx-msaa8-1.png`.

## 2–3. "500 m is a whiteout", "12 km is a pale wash"

The 12 km wash was the air: `SUN_I` 26 was the Sun's irradiance in the air but
the ground was lit from an unrelated 1.5, so the haze along a horizontal
kilometre outshone the sunlit ground behind it several times over. Now the
ground and the deck are lit from `SUN_E = SUN_I / π` (a white Lambertian under
the same Sun), `SUN_I` = 5, EARTHLIKE `atmosphere_density` 0.45 → 0.30 (blue
optical depth per kilometre near Earth's on this small, short-scale-height
world), Mie `0.15 / g 0.72` → `0.06 / g 0.55`, the star cover follows the
dimmer sky (`over` 13 → 80). The SKY contract (`sky_column`, H_RAY) is untouched.

The 500 m whiteout was **not** the air. Bisected with `FARFALL_SKIP` and a new
`FARFALL_BENCH_CLOUDS=k` knob (`d2-*`, `d3-*`, `d4-*`, `d5-*`): with the planet
pass skipped the frame was still a grey-tan fog. It was the **entry plasma**:
`FARFALL_BENCH_ALT=500` parked the ship in the sim's circular orbit at 500 m —
787 m/s in sea-level air — and `thermal.wgsl` (spec'd to glow from ~400 m/s in
thick air) lit a 6 kK sheath over the whole canopy. That is the feature
working. The second layer was the cloud deck's underside, lit as if seen from
above. Fixes:

- `FARFALL_BENCH_ALT` under 8 km now flies at 250 m/s over a coast
  (`LOW_BENCH_*` in lib.rs; `FARFALL_BENCH_ALT_AT=lat,lon` picks the spot,
  stock 10°N 320°E found by `sc2-*`/`sc3-*` scouting — the orbit's own spot is
  open ocean). The stock 12 km scene is above the ceiling and unchanged.
- The deck's underside gets `1 − 0.8·density` of the Sun (a grey ceiling).
- `dust::drift` takes the air: in the air the motes rest with the ground, not
  with orbit (a 250 m/s cruise streamed them at 540 m/s).

Proof: `r4-low500-1.png` / `r5-low500-1.png` (dunes and hills with relief, a
horizon, a blue gradient), `r4-low500-side-1.png` (the coast: sand, turquoise
shallows, sea), `r4-alt3k-1.png` (land, coast, sea, the deck, graded blue),
`r4-alt3k-up-1.png` and `r5-low500-up-1.png` (the dome with the brightest stars
through it), `r4-alt12k-1.png` (deep graded blue, stars; devlog `gfx-msaa8-1`
was 140-ish grey-blue at the top, now 55/55/73), `r4-alt80k-1.png` (black
space), `d2-globe-1.png` (the globe from 336 km). Numbers (mean sRGB of the
top-of-sky band): devlog 12 km 150/170/200 → 55/55/73; 500 m 176/175/173 →
77/100/124 with the ground under it at 159/149/128.

## 4. Shield strikes filled the sky

`shield.wgsl`: the honeycomb is revealed in a ring 0.4–0.7 m behind the crest
and at the impact, fading as `exp(−d / (0.6 + 0.6·size))` from the strike and
faster with time than the wave; the crest and swell thin with distance; the
lattice lines 2.2 → 1.6. Under hyper the honeycomb term is 0.5 → 0.04 of the
stream: a wet sheen, no lattice.

Proof: `r4-strikes-1.png` (crest rings, a patch of cells round each hit, the
sky clear), `r4-hyper-1.png`, `r3-strikes5-1.png` (the scene test's five).

## 5. "A bright column of stars at the octahedral fold"

Two things were there. (a) `starfield.wgsl` measured a star's pixel footprint
from the screen derivatives of the grid coordinate, which is discontinuous at
the fold (x=0,z<0 and y=0,z<0 are the map's four edges): any pixel quad
straddling it got a garbage Jacobian and drew every neighbouring star at full
size. The footprint now comes from the derivatives of the view ray (smooth
everywhere), solved as a 2×2 Gram system, and the neighbourhood search wraps
through the fold's mirror (`oct_true_cell` / `oct_mirror_pos`), so the same
star is found from both sides at its true direction. `starfield.rs` mirrors
the wrap with `a_cell_past_the_maps_edge_is_the_mirror_of_its_neighbour_across_the_fold`.
(b) The column the devlog shows is **not stars**: it is Uranus' ring seen
edge-on from inside the belt (`bodies` pass — gone with `FARFALL_SKIP=bodies`,
still there with `FARFALL_SKIP=starfield`: `d6-belt-*`). In-plane rays run the
whole chord of the annulus, so at `RING_HAZE_FREE_M` 250 km the band saturated
into a hard-edged white column; at 700 km it is the soft band on the belt's
horizon the world doc describes (peak 77 → 43 sRGB, no plateau).

Proof: `r6-belt-1.png` vs devlog `place-belt-1.png`; `zoom-belt-col.png`.

## 6. Scene tests

There are no golden images — `crates/app/tests/scenes.rs` measures regions.
Nobody had run it on Windows: the harness set `TMPDIR` for the capture
directory but the exe's `temp_dir()` reads `TMP`/`TEMP` there, so every capture
landed in `AppData\Local\Temp` and all 18 tests failed at "no capture". The
harness now sets all three. Then 17/18 passed; the nebula test's "off is black
sky again" failed on two counts, both real: the baked Milky Way under the HDR
exposure read as a blue-grey fog over the whole sky with the nebula off
(`d7-neb-off-1.png`) — it is now a third as strong (`d8-neb-off-1.png`, a faint
band) — and the test's "lit" threshold (sRGB 0.08) sat under AgX's black floor
(empty space is ~38/35/43), so it counted the floor as gas; it measures above
0.22 now. The hologram's region moved with it; the ground test (POS at the
pole, ALT 0 — a snow field with the equirect's pole fan) passes with the new
sky (`r4-ground-1.png`, sky 97/115/130, ground lum 0.32 > 0.3 — marginal, and
the dark fans at the pole are the bake's singularity, not this branch's).
Run: `FARFALL_SCENE_TESTS=1 FARFALL_WINDOW_POS=-6000,-6000 WSLENV=FARFALL_SCENE_TESTS:FARFALL_WINDOW_POS cargo-q.sh test --release -p farfall-app --test scenes`
(the windows are born off-screen; 18 pass).

## Knobs added (documented in `crates/app/src/lib.rs`)

`FARFALL_BENCH_CLOUDS=k`, `FARFALL_BENCH_ALT_AT=lat,lon`; the ALT knob's
under-8-km behaviour.

## Left / for whoever owns them

- The plasma sheath at orbital speed in thick air fogs the whole glass (0.03
  floor of `veil` in `plasma.wgsl`). Right for an entry, but a dive to 500 m at
  700 m/s is a whiteout by design — `thermal`'s owner may want the veil term
  lower or the readability cap sooner.
- In-air dust is dense (`√(ρ/ρ0)·0.9`) and streaks at any airspeed — a fine
  rain over the sky at 250 m/s (`r5-alt3k-up-1.png`). Particles' call.
- The trajectory hoops sit in the middle of the low-altitude frames now that
  the path bends into the ground at 250 m/s — honest, but busy.
- The globe's limb glow from 336 km is thick (TOP = 0.42 R) — world's design.
- The deck from just above (3 km) is one bank across the view; from 500 m a
  grey ceiling. Coverage and bank size are the preset's.
