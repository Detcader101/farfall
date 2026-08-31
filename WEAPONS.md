# WEAPONS & TOOLS — design (branch `weapons`)

Owner intent (2026-08-24): shoot various weapons and tools from the ship, all as
GPU shaders; gritty, satisfying effects against asteroids, bodies and (later)
other ships; plus extra systems so it is not just point-and-shoot with the jet.

Note: SPEC.md §2 lists "no combat" as a non-goal. This branch supersedes that
for the weapons lane; SPEC.md gets a one-line amendment in the first chunk.

## Principles (inherited)
- Every visual is WGSL (`shaders/*.wgsl`, registered in `render/src/shaders.rs`,
  `InstrumentPass` flavour like `belt`/`ghost`/`shield`). No CPU-drawn anything.
- The sim's golden hash stays untouched: projectiles, damage and rock state live
  in the app (like `belt.rs`), impulses applied after `sim::step`.
- Every knob is a setting (`settings.rs` + `menu.rs` row + fits test) and a
  `~/.farfall/settings.cfg` key. New menu page: **ARMS**.
- One-shots in audio are counters in `Levels` (strike-voice pattern).
- Each chunk: tests first → gate → commit → ledger update → `FARFALL_BENCH_*`
  knob + scenes assertion for the e2e line.

## Architecture

```
app/src/arms/
  mod.rs        Arms { hardpoints, power, heat, ammo, projectiles, beams, tools }
  fire.rs       pure: aim solution, lead, spread, recoil impulse, heat/power step
  projectile.rs pure: f64 ballistic tracers, hit tests vs rocks / bodies / ships
  damage.rs     pure: rock damage table keyed on Rock.id -> fragments, body scars
  targeting.rs  pure: lock, lead marker, threat list
render/src/
  tracer.rs + shaders/tracer.wgsl    projectile streaks, muzzle flash, shockwaves
  beam.rs   + shaders/beam.wgsl      lance / cutter / tractor beams (volumetric)
  debris.rs + shaders/debris.wgsl    fragment cloud + ejecta, ray-traced like belt
  scar.rs   + shaders/scar.wgsl      decals on rocks/bodies: glowing crater, cooling
```

Rocks get mutable state via a side-table `HashMap<u64 /*Rock.id*/, RockState
{ hp, scars: [..], cracked }`; destroyed rocks spawn `Fragment`s (non-hash
rocks in the same `Belt` live set, so belt collisions/ray-tracing work unchanged).

## Weapons (each a `Weapon` enum arm with its own shader look)
1. **Autocannon** — kinetic slugs, muzzle flash, tracer streak with heat
   shimmer, ricochet sparks on glancing hits, recoil impulse on the ship.
2. **Railgun** — charge (BoomTrigger shape), rail glow, hypervelocity slug,
   plasma wake, through-and-through on small rocks; bass crack.
3. **Lance (beam)** — continuous cutting laser; heats the hit point (scar
   goes white→orange→dull red), slices rocks in two along the sweep.
4. **Flak** — proximity burst, shrapnel cloud, good vs fragments / ships.
5. **Torpedo** — slow guided; needs a lock; big gritty detonation with
   debris ring + shockwave refraction in `blit.wgsl` post.
6. **Mines** — dropped astern, drift on ship vel; ring-of-light trigger.

## Tools (non-lethal)
7. **Tractor / repulsor beam** — pulls or shoves rocks (impulse on rock vel).
8. **Mining cutter** — slow lance variant; chunks become collectible ore.
9. **Flare / illumination round** — lights the belt's dark side.
10. **Scanner pulse** — expanding sphere shader, tags rock composition.

## Extra systems (not point-and-shoot)
- **Hardpoints** — 4 slots (nose, wing L/R, belly), each mounted with a
  weapon; gimballed slots track the gaze in freelook (fire where you look).
  Mount assignment on the ARMS page; saved.
- **Power bus** — one reactor budget shared by drive / shield / arms; slider
  split (`arms.power`), firing while boosting browns out the shield ripple.
- **Heat** — per-weapon heat, overheat jams; radiators glow (shader on hull).
- **Ammo & magazines** — kinetic weapons need ammo; reload cycle sound.
- **Targeting** — lock (T-hold? see keys), lead marker on the glass computed
  from rock vel and slug speed, threat arrows at the glass edge.
- **Fire groups** — 1/2/3 select group; group = set of hardpoints, sequenced
  or salvo (`arms.group1.mode`).
- **Turret mode** — hold a key: freelook becomes a gimbal turret, ship holds
  attitude (assist), fire on click.
- **Damage model for ships** (multiplayer-ready): hull hp + subsystems
  (drive, shield, arms), pure and deterministic so it can be lockstepped
  later; the player's own ship feeds `shield::Impact` and, later, `crash`.

## Keys (currently free: B I N O U Y, digits)
- LMB fire (when not gaze-dragging), RMB stays freelook
- `1..6` select fire group / weapon, `N` next weapon, `Y` turret mode,
  `U` lock target, `B` drop mine, `I` toggle safeties. All rebindable.

## Chunks (each = tests + shader + setting + ledger + bench)
1. arms core: hardpoints, power/heat/ammo step, fire groups, ARMS page.
2. autocannon: tracer.wgsl (streak + flash + sparks), slug hit on rocks,
   RockState hp, `FARFALL_BENCH_ARMS=cannon`.
3. debris: fragments + debris.wgsl, rock splitting, satisfying breakup.
4. scars: scar.wgsl decals on rocks and bodies, cooling curve.
5. railgun (charge) + lance (beam.wgsl) + audio voices.
6. targeting: lock, lead marker, threat arrows, turret mode.
7. flak, torpedo (guided + post shockwave), mines.
8. tools: tractor/repulsor, cutter + ore, flare, scanner pulse.
9. ship damage model + Impact feed (own ship), multiplayer seam.

## Other ships (polish pass, 2026-08-31 — branch `fable/ships`)

Owner asks, verbatim: *"Make mimic ships as big as me."* and *"Make ships that
spawn already, that are like mimics but mine asteroids to upgrade and get
bigger, eventually becoming more powerful with the materials."*

### Mimic size

A mimic is drawn from `sd_fighter_exterior` — the very SDF the cabin is carved
from and the chase camera shows — at 1:1. It IS our ship's size class. It reads
small because a hostile held 520 m off and a hailer 780 m: at 800×600 a 15 m
fighter at that range is a dozen pixels, while the chase camera sees our own
hull from 24 m. So:

- The mimic pass gets a **size lane** per hull (`MimicView.size`, metres per
  SDF unit): a mimic is 1.0 — exactly our fighter — and the collision sphere,
  hold radius and hit maths scale with it (`HULL_R_M * size`).
- **Encounter ranges** are the fix for what the eye sees: a hailer comes
  alongside at `HAIL_HOLD_M` = 90 m (canopy, beacon and nozzles readable), a
  hostile holds `HOLD_M` = 320 m weaving (its spread widened a touch so the
  closer range is not a free kill). Fire range unchanged.
- The pass grows to 12 lanes (4 mimics + 8 miners); the sight's marker array
  grows with it.

### Miners

App-side only (like mimics: hashes + app state; the sim's golden hash is
untouched). `crates/app/src/miner.rs`.

**Population.** When the belt is live (the ship is in Uranus' ring) and no
miners have been placed since the ring was last empty, `MINERS` (setting
`miners.count`, 0–8, 4 stock) ships are placed 900–2 600 m from the ship in
hashed directions, riding the ring's velocity, each with a hashed starting haul
(mostly nothing, a few already a tier up: a population that was here before
us). Dropped past 12 km. Never respawned until the ring has been left.

**Life (the growth state machine).**

```
Seeking  — pick a rock: the nearest live rock ≥ 8 m within 4 km that is not a
           mimic in a shroud, not another miner's claim, and NEVER the rock
           the pilot is holding station on (HOLD). None: drift, retry in 2 s.
Transit  — fly to a stand-off point (rock surface + 70 m × size) matching the
           rock's velocity, nose on it.
Mining   — the beam is on: ore drifts up it; the miner's haul grows at
           MINE_T_PER_S × (1 + 0.5 tier) × MINER GROWTH (miners.growth,
           0.25–4, 1 stock) t/s, by the rock's ore kind, until the rock has
           given its share (0.1 % of its mass, 2–80 t) — then it is wounded
           to breaking (fragments off it, the belt's own break) and the
           miner seeks again. The pilot taking HOLD on its rock, or the rock
           leaving the live set, sends it back to Seeking.
Hailing  — a neutral miner within 300 m of us hails once (a line on the
           readout, the relay knock) and goes on mining.
Attacking— shot at: a tier-0 miner runs (Leaving); a grown one comes about
           and fights with the mimic's guns scaled by tier (longer bursts,
           the rail at tier 3), holding 320 m.
Leaving  — full burn away, cleared past 8 km.
Wreck    — past its toughness: dark, tumbling; its whole haul goes into
           ours (× ORE YIELD) plus salvage.
```

**Tiers** (by haul crossing a threshold; a miner never loses a tier):

| tier | haul ≥ | size | toughness | shield | guns | hull detail |
|---|---|---|---|---|---|---|
| 0 | 0 t | 1.0 | 2.4 MJ | none | cannon, bursts of 3 | the fighter, ochre stripe |
| 1 | 40 t | 1.6 | 6 MJ | none | cannon, bursts of 5 | ore tanks under the wings |
| 2 | 160 t | 2.4 | 14 MJ | 40 % of a hit shed | cannon, bursts of 7 | + dorsal collector, bigger nacelles |
| 3 | 480 t | 3.4 | 32 MJ | 65 % shed | cannon 9 + the rail | + the drill boom off the nose |

**Look** (mimic.wgsl, same pass): the hull is the fighter SDF at `size` with
the tier's parts unioned on; an ochre working stripe instead of the hostile's
rust; engines a working amber-white; the beam a thin hot amber-white core with
a faint red halo and bright ore motes sliding up it toward the miner, ending in
a glow on the rock's face; a shield sheen on tiers 2–3 when hit. Far off, any
hull that would be under two pixels is a lit speck so the belt reads as
populated. The sight marks a miner with an ivory diamond (kind 3), red and
pulsing when hostile (kind 4); edge arrows as for mimics.

**Bench.** `FARFALL_BENCH_MINERS=tier[,mine|fight]`: a miner of that tier
ahead-left, mining a planted rock with the beam (default) or fighting (slugs
in the air, a hit on the shell).

**Interface for the HUD.** `Game::contacts()` → `Vec<(DVec3, u8)>` world
position + kind, mimics then miners, the same shape the sight's marks use.
