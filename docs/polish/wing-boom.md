# wing-boom — the wing guns carried on the airframe (branch `fable/wing-boom`, 2026-08-31)

Jay Jay's call on the flag raised in [cockpit-arms.md](cockpit-arms.md): the
wing hardpoints should not read as pods floating beside the canopy on a bare
lug — they move back onto the wing, carried on a proper boom under it.

## What changed

- **The places** — `bay.rs::Hardpoint::pos()` (the one transform table) moves
  WING L/R from `(±2.6, -0.35, -0.6)` to `(±2.6, -1.0, 0.9)`: down to just
  under the wing plane (y -0.92) and aft to the nose of the new boom.
  `arms::WING_L/WING_R` (the firing table — player, mimics, miners) moved with
  it; nose and belly untouched.
- **The booms** — `common.wgsl::sd_fighter_exterior` grew a slender outrigger
  capsule under each wing (`x ±2.6`, z 0.95→3.9, rising into the wing's
  underside, r 0.14). It is in the *exterior*, not the mount, so every lane
  that draws the hull gets it with no new plumbing: the glass, the chase/jet,
  the bay hologram, mimics, miners, holo3PP, the map dart and the warp ghost —
  one silhouette everywhere. It sits inside the aft bounding box (front z 0.81
  > 0.8), so the cabin march's open-sky cost is untouched, and the mounts'
  own `MOUNT_BOUND_C/R` sphere trick is unchanged (the whole rail still fits
  the sphere at the new place).
- **No new veto needed** — the cockpit sky early-out only fires for rays with
  `exit.y > -0.05`; every sight line to the boom (y ≈ -1) points down, so it
  marches and the budget holds (perf below).

## What still holds

- The muzzle stays meaningfully forward: the rail's barrel tip sits ~4.7 m,
  the slug spawn point ~2.1 m, ahead of the wing's leading edge (z 3.04 at
  that station) — nothing fires through the airframe.
- Convergence (`CONVERGE_M` 300 m) and the sight pips read the same table;
  the pip shift from the new place is <0.01 NDC (see wb-glass-wl-rail's
  leader and pip, wb-arms' tracers).
- `every_pass_reads_the_same_fit_table` (bay), the cabin/jet/hologram lane
  pins and the sight test updated to the new coordinates; `h.pos().z < 2.0`
  ("forward of the engines") still true at 0.9.

## Captures (farfall-captures/wing-boom/, 800x600 4xMSAA, ~353-359 fps — the boom is free)

| capture | shows |
|---|---|
| wb-before-yawneg | **BEFORE** (base ff954c9): the rail a floating pod beside the canopy |
| wb-glass-wl-rail | after, same head (-95,-15): the rail slung low abeam off WING L |
| wb-glass-wl-boom | head -130,-30: the money shot — breech on the boom, the boom running back into the wing |
| wb-glass-wl-cannon | the twin cannon on the same carry |
| wb-bay | the SHIP bay hologram agreeing: RAIL under WING L, panel matching |
| wb-chase | dead astern: the fit under the wings |
| wb-chase-quarter | high quarter: the rail riding the wing |
| wb-mimic | a mimic wearing the same silhouette (exterior booms for free) |
| wb-arms | tracers and muzzle flash from the new muzzles, pips right |

## Post-merge verification (2026-09-01, `ae573ff` = this work + fable/polish 8765bbb)

The merge with polish (HOTAS: cabin uniforms grew the stick beside the
hardpoints) was clean — no conflicts, full workspace test suite green,
sim golden hash untouched (the hardpoint tables live in the app crate;
`sim::state_hash` covers `WorldState` only). Re-captured on the merged
build (wbm-*, 1600x1200 except the first four at 800x600):

| capture | shows |
|---|---|
| wbm-bay | SHIP bay hologram: cannon slung on the boom under WING L/R, FIT panel matching |
| wbm-holo / wbm-holo-big | holo3PP miniature over the dash wearing the mounts at the wing stations |
| wbm-mimic / wbm-mimic-big | a hailing mimic 40 m out with the same under-wing carry |
| wbm-chase | dead astern: the fit under the wings |
| wbm-chase-quarter | high quarter (head -25,-18): both wing mounts riding the booms |
| wbm-glass-boom | over the shoulder (head -130,-30): the breech carried on the boom into the wing |

## Decisions

- Boom in the exterior rather than per-mount geometry: the boom is airframe
  (there even when the hardpoint is empty, like a real pylon station), and
  putting it in `sd_mount` would have doubled it into the nose/belly mounts
  or needed a kind split. The bare-lug kind-0 pylon still draws on the boom's
  nose for an empty wing slot.
- `(±2.6, -1.0, 0.9)` and not further aft: far enough back that the boom
  carries it, far enough forward that the gun still shows through the side
  glass from the seat (the wing itself is aft of the canopy and out of view).
