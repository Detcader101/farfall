# cockpit-arms — the glass sees the bay's fit (branch `fable/cockpit-arms`, 2026-08-31)

Owner asks: *"the cockpit should see the weapons you customised on the build
screen."*

## What was true before

The fit was real everywhere but on the airframe the pilot looks at. `arms.rs`
fired from whatever the bay mounted (`bay.rs::Hardpoint::pos()`, the muzzle
table), and the SHIP bay's hologram drew the mounts (`hologram.wgsl::sd_mount`)
— but the cockpit marched the bare hull (`sd_fighter_hull`), and the chase
view / holo3PP ship (`jet.wgsl`) marched the bare exterior. **Neither showed
the fit** — the brief's "the chase and holo3PP should already show fits" turned
out not to hold, so the jet lane was brought along too (it is the ships work's
own lane; holo.wgsl itself was not touched and did not need to be — the
holo3PP's ship is the jet pass).

## One source of truth

- **Places** were already one Rust table — `bay.rs::Hardpoint::pos()`. New
  `bay::fit_views(&mounts)` maps it plus each `Mount::kind()` into the render
  crate's `MountView`; the bay hologram fill, `CabinUniforms` and
  `JetUniforms` all read **only** this (`lib.rs` no longer hand-rolls the
  hologram's mount array). Test: `bay.rs::every_pass_reads_the_same_fit_table`.
- **Geometry** was one shader function in one pass — `sd_mount` moved from
  `hologram.wgsl` into `common.wgsl` (with `sd_ring_z` and the
  `MOUNT_BOUND_C/R` bounding sphere), so the bay, the glass and the chase
  march identical cannon/rail shapes. Kind 0 now draws the **bare pylon**
  (the carrier lug) instead of nothing: an empty hardpoint is a thing the
  pilot can see, in the bay and through the glass alike.

## The cockpit march (`cockpit.wgsl`)

- `Cockpit` uniforms grew `hp: array<vec4<f32>, 4>` (xyz place, w kind);
  `CabinUniforms` mirrors it (now 16×16 B, lane test updated).
- `sd_mounts(p)`: each mount joins the cabin march **only inside its
  hardpoint's bounding sphere**; outside it the sphere's distance is the safe
  lower bound, so the governor's budget for open sky is untouched.
- The sky early-out gained a mount veto: a ray passing a fitted hardpoint's
  bounding sphere (a wing gun past the brow, the nose rail) marches after all.
- A refit is a uniform change with an unmoved view → the cabin governor's
  sharp in-place strip redraw, never the moving-size blur
  (`cabin.rs::the_bay_fit_reaches_the_glass_and_redraws_in_place`).

## The chase view (`jet.wgsl`)

`sd_fitted` = exterior + the four mounts under the same bounds;
`JetUniforms::with_fit` carries the table (12×16 B, lanes pinned in
`jet.rs::jet_lanes_hold_their_places`). The holo3PP's little ship is this same
pass, so it agrees for free. `mimic.wgsl` is untouched: mimics/miners are
other ships with their own tier parts, not the bay's fit.

## Bench

`FARFALL_BENCH_FIT=n,l,r,b` (mount keys in hardpoint order nose, wing L,
wing R, belly; benchmark only) sets `settings.mounts` and `arms.mounts` so a
capture can wear any fit.

## Captures (farfall-captures/cockpit-arms/, all bench=True, 800x600 4xMSAA)

| capture | fit (nose, wing L, wing R, belly) | shows | perf |
|---|---|---|---|
| ca-glass-nose-1 | stock (rail, cannon, cannon, empty) | the nose mount over the brow from the seat | 355.3 fps |
| ca-glass-wl-cannon-1 | empty, cannon, cannon, empty | the cannon's breech and barrel off WING L through the left glass; the nose's bare pylon on the spine | 359.9 fps |
| ca-glass-wl-rail-1 | empty, rail, empty, empty | the rail on WING L — barrel, three coils, breech — through the left glass | 346.9 fps |
| ca-glass-wr-cannon-1 | empty, cannon, cannon, empty | the cannon off WING R through the right glass | 358.5 fps |
| ca-glass-wr-empty-1 | empty, rail, empty, empty | WING R's bare pylon: an empty hardpoint is a thing | 341.0 fps |
| ca-bay-fit-1 | empty, rail, empty, empty | the SHIP bay agreeing with the glass: WING L RAIL (coils on the hologram), WING R EMPTY, the panel reading the same | 324.7 fps |
| ca-chase-fit-1 | empty, rail, empty, empty | the chase view astern: the rail's breech off wing L, the bare pylon off wing R | 325.2 fps |

Perf sits where the cabin was before the fit joined it (~325-360 fps at the
bench window) — the bounding spheres keep sky rays cheap.

## Flag for Jay Jay (design)

`Hardpoint::pos()` is the **muzzle** table: the wing points sit at
(±2.6, −0.35, −0.6), ~3.6 m ahead of the wing's leading edge, so a wing mount
reads as a pod floating beside the canopy with only its lug — in the glass, in
the bay and in the chase alike (consistent, and honest to where the shots
leave). If Jay Jay wants them visually carried, the clean fix is an outrigger
boom from each wing hardpoint back to the wing in `sd_fighter_exterior` —
that touches the shared silhouette every lane draws (mimics too), so it is
left as his call rather than slipped into this pass.
