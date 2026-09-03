# ships — mimic size and the miners (branch `fable/ships`, 2026-08-31)

Owner asks: *"Make mimic ships as big as me."* and *"Make ships that spawn
already, that are like mimics but mine asteroids to upgrade and get bigger,
eventually becoming more powerful with the materials."* Then, mid-pass:
*"focus on making features simulated if you can too."*

## What I found about mimic size (and what I did)

A mimic was already our own fighter: `mimic.wgsl` marches `sd_fighter_exterior`
— the SDF the cabin is carved from (`cockpit.wgsl` → `sd_fighter_hull`) and
the chase camera draws (`jet.wgsl`) — with no scale factor anywhere. It read
small because of range: a hostile held 520 m off and a hailer 780 m, and a
15 m fighter at that range is a dozen pixels at 800×600, while the chase view
sees our own hull from 24 m. So:

- **Encounter ranges** (`mimic.rs`): a hailer comes alongside at
  `HAIL_HOLD_M` = 90 m — canopy, beacon and nozzles readable; a hostile holds
  `HOLD_M` = 320 m (was 520), spread widened 0.012 → 0.016 rad so the closer
  range isn't a free kill. Fire range unchanged.
- **A real size lane** in the pass (`MimicView.size`, metres per SDF unit):
  the hit sphere (`Mimics::hull_r_m()`), the hold radius and the shader all
  scale from one number. **MIMIC SIZE** (`mimics.size`, 50–300 %, 100 % stock
  = exactly our ship) on the ARMS page lets Jay Jay dial it by eye — his felt
  size is the one that matters and the geometry says 1:1 is honest, so the
  default stays 1:1 and the knob is his.
- The pass has **12 lanes** (4 mimics + 8 miners); the sight's marker array
  and `sight.wgsl` loop grew with it.
- Pre-existing bug fixed on the way: MIMICS / HOSTILITY / ORE YIELD changed on
  the ARMS page never reached `Mimics` until a SHIP-bay dropdown was touched
  (`lib.rs` only copied them in the bay handler). They are now applied every
  fixed step beside the arms' settings.

## Miners (new: `crates/app/src/miner.rs`)

Everything is simulated, app-side, deterministic (hash-placed, PRNG stepped
from `time_s`, never wall-clock), the golden hash untouched:

- **Population.** `MINERS` (`miners.count`, 0–8, 4 stock) placed from the
  nearest rock's cell hash the moment the belt goes live (900–2 600 m off,
  riding the ring, a few already a tier up); never again until the ring has
  been left. Dropped past 12 km.
- **State machine** `Seeking → Transit → Mining → (Seeking…)`, plus
  `Attacking`, `Leaving`, `Wreck`. Target choice: nearest live rock ≥ 8 m within
  4 km, not a shroud (`mimic::is_mimic`), not another miner's claim, and
  **never the rock the pilot is holding station on** — taking HOLD on a miner's
  rock makes it stand off (`MINER: YOUR ROCK. WE STAND OFF`).
- **The cut is real.** Each step the beam wounds the rock through
  `Belt::strike` against the belt's own toughness, so the rock cracks as it is
  worked (the guns' wounds add to the same tally) and breaks into fragments as
  the last of its share (0.1 % of its mass, 2–80 t) goes. The haul is tonnes by
  `mimic::Ore` of the rock's seed at `0.35 t/s × (1 + tier/2) × MINER GROWTH`
  (`miners.growth`, 25–400 %).
- **Tiers** are thresholds on that haul (40 / 160 / 480 t; a miner never comes
  down): size 1.0 / 1.6 / 2.4 / 3.4 × our fighter, toughness 2.4 / 6 / 14 / 32
  MJ, shield sheds 0 / 0 / 40 / 65 % of a hit (a cyan sheen on the hull),
  bursts of 3 / 5 / 7 / 9, the rail at tier 3. Hull parts by tier in
  `mimic.wgsl::sd_parts`: ore tanks under the wings; dorsal collector and
  fatter nacelles; the drill boom with its ring.
- **Consequences.** Neutral until shot at (a neutral one within 300 m hails
  once — the relay knock, a line on the readout). Shot: tier 0 runs, a grown
  one comes about and fights with the mimic's gun code scaled by tier; its
  slugs ride `Mimics.slugs`, so they hit our hull through the existing
  shield-strike / HULL path. A wreck gives its **whole haul** to ours ×
  ORE YIELD plus salvage: `WRECK: 200 T HAUL TAKEN`. HOLD (O) locks a miner
  (`HOLD MINER 120 M`).
- **Look.** Same pass: ochre working stripe, grime, working amber-white
  engines, a lamp at the beam's root; the beam a thin hot core + faint red
  halo + ore motes sliding toward the ship + a glow where it bites, occluded
  by rocks; far off any hull under ~2 px is a lit speck so the ring reads as
  worked. Sight marks: ivory diamond / edge arrow (kind 3), red pulsing when
  hostile (kind 4).

### Interface for the HUD / hologram agent

`Game::contacts(t) -> Vec<(DVec3, u8)>` — world position + kind (0 hail,
1 hostile, 2 wreck, 3 miner, 4 hostile miner), mimics out of their shrouds
first, then miners. The sight's marks are built from exactly this list;
the hologram's enemy-bearing marks can be too.

## Verification

Captures in `farfall-captures/ships/` (all `FARFALL_SPAWN=belt`, 800×600):

- `miner0-mine-1.png`, `miner1-mine-1.png`, `miner2-mine-1.png`,
  `miner3-mine-1.png` — each tier at work: the hull with the tier's parts,
  the beam to the planted rock's face with its bite, the far speck with its
  marker, the hail line on the readout.
- `miner0-fight-1.png` … `miner3-fight-1.png` — each tier come about, its
  slugs in the air, the shield sheen on tiers 2–3, a hit on the shell.
- `mimic-hail-1.png` — a mimic hailing, unchanged in the 12-lane pass.

Note on the first round of captures: two of them showed `HOLD ROCK 0 M` /
`HOLD MINER 0 M` and the holo3PP — the log says `hold: locked` and `holo3PP
ON`: key presses leaking into the bench window while Jay Jay was playing.
Not a bug; re-captured.

Perf (1920×1080 4×MSAA, vsync off, 8 s, `FARFALL_SPAWN=belt`):
`perf-miners-full` (a tier-3 miner mining ahead + a far one, 12-lane pass,
beam and specks) 2.35 ms avg frame / 425 fps; `perf-belt-full` (no miners)
1.79 ms / 560 fps. The miners cost ~0.5 ms at 1080p — under 1.5 ms at
2880×1800, well inside the 60 fps floor. The per-pixel cost is 12 bounding
sphere tests plus a march only where a hull is; the beam and speck maths run
only for lanes that have them and only test the rocks where they glow.

Gate: `cargo.exe fmt --all --check`, `clippy --workspace --all-targets -D
warnings`, `cargo.exe test --workspace`, `cargo.exe check --workspace
--target wasm32-unknown-unknown` — all green at the final commit (the last
run is quoted in the final report).

Tests added (behaviour-named, `crates/app/src/miner.rs`):
`a_miner_grows_through_its_tiers_as_its_haul_crosses_the_thresholds`,
`a_miner_seeks_a_rock_flies_to_it_mines_it_with_the_beam_and_seeks_again_when_it_is_spent`,
`a_miner_picks_the_nearest_rock_worth_mining_and_not_a_claim_or_a_shroud`,
`a_miner_never_mines_the_rock_the_pilot_is_holding_station_on`,
`a_shot_small_miner_runs_and_a_grown_one_fights_back_harder_until_it_is_a_wreck_that_drops_its_haul`,
`the_population_is_placed_once_in_the_ring_and_again_after_leaving_it`;
`render/src/mimic.rs::mimic_lanes_hold_their_places` covers the new lanes.

## Decisions and why

- **1:1 stays the default mimic size** — the geometry is honest and the brief
  said "same size class"; the ranges are what fix the picture; the knob exists
  for taste.
- **Miners break rocks rather than shrink them** — rocks are hashed, not
  stored; shrinking would need a side table the belt shader reads. Breaking
  reuses the belt's fragment machinery and is visible.
- **Miner slugs, hails and lines ride the mimics' structs** — one shield path,
  one tracer path, one readout line, no new plumbing in `lib.rs`.
- **Population per visit, no respawn** — deterministic and cheap; a ring you
  clear stays cleared until you leave it.

## What is left

- A miner scene test in `crates/app/tests/scenes.rs` (opt-in golden captures).
- Miners do not trade, dock or go home; a hail is one line.
- The shield sheen is a hull rim effect, not a bubble; a proper shield shell
  shader for the big tiers would be a nice follow-up.
- No sound of its own for the beam (no chimes; a low structure-borne hum
  when close would fit the sound rules).
