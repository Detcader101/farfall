# SHIP lane — the holographic ship bay

Branch `ship` (worktree `~/space-game-ship`), forked from `weapons`. Never merged
by this lane; the owner merges. A first-person holographic bay for looking at,
arming and (later) refitting the pilot's own ship, opened on its own key onto a
screen-fixed, cursor-dragged panel with a side card — the 3D map's pattern.

## Done
1. `ship-holo` — hologram.wgsl: the fighter's own SDF seen from outside as a
   translucent cyan hologram (scanlines, edge glow, slow yaw), orbited with the
   mouse like the map camera; colour / scanline density / size / spin / anchor
   are settings keys with menu rows.
2. `ship-hardpoints` — the four slots (nose, wing L, wing R, belly) as pips on
   the hologram; the side card lists each slot's mount, left/right cycles it,
   saved as `ship.hardpoint.<n>`; arms reads it on load.

## Next chunks (one commit each, through the gate)
3. Trim: hull trim hue/saturation on the hologram (already in the shader's
   lanes) and on the real ship's jet/cabin passes — `ship.trim.hue/sat`.
4. Loadout weight: mounts change mass and inertia in the sim (a rail is heavy);
   the card shows MASS and the turn rate change.
5. Ammo/heat per hardpoint on the card, live in flight; empty slots greyed.
6. More mounts: missile rack, mining laser, sensor pod (arms enum first).
7. Callsign + hull number painted on the hologram and the jet (settings key).
8. Bay key opens from the map too (a tab on the pane), one panel family.
9. Sound: the bay's hum and the mount click on cycling (no chimes).
10. Ledger sweep: every entry above `complete` with an e2e capture line.
