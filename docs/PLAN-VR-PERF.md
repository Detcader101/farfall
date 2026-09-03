# PLAN — VR at 144 Hz and high resolution (fable/vr-perf)

Jay Jay, 2026-09-03: "need 144 hz and high res so we need to get thinking creatively".

## Where we are (measured, fable/vr@972265e, RTX 2080 Ti, real SteamVR row on the desk)

| Setting | Pixels per frame (both eyes, after the hull) | Frame time | Budget at 144 Hz |
|---|---|---|---|
| SteamVR 150 % (2468×2740 per eye) | ~16 M | 30 ms | 6.9 ms |
| SteamVR 100 % (~2016×2240 per eye) | ~10.6 M | ~20 ms (est.) | 6.9 ms |

Flat bench at 800×600 runs ~2.9 ms, so the cost is ~1.7 ms per megapixel plus ~2 ms fixed.
A ~4× speedup is needed. Uniform downscaling reaches it only at ~1100×1100 per eye, which is
not "high res". The pixels have to go where the lenses resolve them, and nothing may be rendered
twice that both eyes see identically.

## The work, in order (each item a commit with its own bench row)

1. **Measure first.** GPU timestamp queries (`TIMESTAMP_QUERY` when the adapter offers it) around
   every pass in VR bench mode; the perf line gains `pass_ms={starfield:…,planet:…,cabin:…,post:…}`
   per eye. No optimisation is chosen before this row exists.
2. **Far world once per frame.** Starfield, nebula, planet, bodies, sun/flare are at infinity:
   render them once per frame into a single wide "far" target and sample it per eye with that
   eye's rotation only (no translation). Cabin, dials, hands, mimics, near belt rocks, dust stay
   per eye. Expected ~1.5–2× on the world share. Test: the far target is drawn once per frame
   with two eyes; each eye's sample uses its own orientation.
3. **Foveated density.** Render the per-eye targets through a radial UV remap (full density in the
   central ~40°, ~¼ density at the rim, smooth in between); the crop/composite pass inverts the
   remap. `vr.foveation` setting 0–1 (0 = off). Expected ~2× on per-pixel passes. Test: the
   remap and its inverse round-trip; density at the centre is 1.0, at the rim the chosen floor.
4. **Direct asymmetric projection.** Render each eye's true frustum instead of the symmetric hull
   (the hull is 1.14–1.19× wider than needed): the camera takes four tangents, the crop becomes
   identity. ~15 %. Test: hull factor logs 1.000.
5. **Per-eye cabin refinement cache.** `view_moved` now counts an eye switch as motion (correct),
   which means the cabin re-marches every frame in VR; keep one cache per eye so both settle.
6. **Rate targets.** After 1–5: `bench-matrix-vr.sh real quick` at 144 Hz with SteamVR at 100 %
   and 150 %; the devlog page's "holds" column is the acceptance. Fallbacks that stay in the
   menu: 120 Hz, VR RENDER SCALE, AUTO SCALE (floor = display rate).

## Not doing

Motion smoothing / reprojection tricks (SteamVR already does them at half rate; they are not
144 Hz). Lower-precision shading in the cabin. Any change to the sim.
