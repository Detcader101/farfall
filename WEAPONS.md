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
