# Polish pass — gauges (`fable/gauges`, 2026-08-31)

Scope: the instruments on the dash — `crates/render/src/{gauge,attitude,cabin}.rs`,
`shaders/{gauge,gvec,gyro,horizon,trajectory,cockpit,common}.wgsl`, the style default in
`crates/app/src/settings.rs`, two draw-order lines in `crates/app/src/lib.rs`.

## What a player will notice

1. **WARTHOG is the default.** A fresh `settings.cfg` (or none) gives the A-10 cluster:
   black face plates on the dash in machined bezels, white markings, a tapered white
   pointer with a boss, the warning arc painted red on the scale, and the gyro a real
   ball — blue over brown with a white horizon, the wings outlined dark so they read on
   either half. TRON / JET / DIAL still cycle on the GAUGES page and per dial.
2. **No cavities, in any style.** The cabin marcher hollows nothing for an instrument
   any more: the TRON recess and lit socket ring, the JET spherical bowl and the DIAL
   well are gone. A DIAL/WARTHOG face is a plate 8 mm proud of the dash inside a thin
   bezel; the JET/WARTHOG gyro is a sphere standing out of the metal (centre 4 cm under
   the surface) with a thin seam ring, painted only above the surface; a JET dial floats
   over a thin flush ring; a TRON hologram over bare metal with its beam. Seen at an
   angle (`FARFALL_BENCH_HEAD=30,-15`) the dash is solid metal with instruments on it.
3. **Crisper instruments.** Every stroke on the arc gauges, the G vector and the gyro is
   a constant-width line with one pixel of anti-aliasing; ticks are arc-length strokes at
   true multiples of the scale (they used to sit half a step off, so no major sat under
   the mach bars); the readout digits are heavier; the `×m Ek` multiplier is printed on
   the face under the hub (arc gauges) so it never lands on the bezel; the G vector is
   drawn at 0.82 on the dash so its fore/aft bar sits on the plate. Steam gauges have no
   halo light at all (printed markings); holograms keep theirs, with accents a little
   over 1.0 for the bloom.
4. **Pitch ladder and hoops.** Bars 9° wide, 30°/60° heavier, end ticks toward the
   horizon, the pitch in degrees at both ends, azimuth conformal (× cos elev). The
   camera-space handedness was wrong (`cross(fwd, up)` points left) so the numerals came
   out mirrored — fixed. The horizon pass is now drawn *before* the cabin so the dash
   occludes the ladder instead of the ladder scribbling over the dials. Landing hoops are
   a one-pixel-plus core at 1.8 radiance with a soft skirt instead of fat pink bands.

## Why the plates looked absent at first (a trap for the next agent)

The dash slab is `sd_round_box(…, 0.04)`: the rounding *inflates* the box, so the metal's
top is 4 cm above the nominal `DASH_C` plane. Everything seated on the plane was buried.
`DASH_SURFACE` (cockpit.wgsl) = `DASH_SURFACE_M` (cabin.rs) = `DIAL_DASH_SURFACE`
(common.wgsl) = 0.04 is now the seat for every instrument, and `Placement::in_dash` /
`Placement::ball` measure from it. Keep the three in step.

Also: `CabinUniforms` capped the socket count at 4 while the shader has 6 pads, so the
fifth dial (the G vector, last in `Instrument::ALL`'s slotted order) never got a plate.
Now `min(6)`.

## Decisions

- **Warthog gyro = the real ball** (was a 2D disc with a brownish tint). Jay Jay asked
  for a ball with no bowl; the A-10's ADI is a ball; one path for both JET and WARTHOG.
  `GyroUniforms::ball` codes a WARTHOG ball as `d.x = 3` because the up vector takes the
  lane the flag rode in (`d.y`) — the old shader was reading `up.x` as the flag.
- **A tilted DIAL is lifted, not sunk.** With no well to cut through, a leaned face's
  near edge would go into the metal; instead the whole instrument rises along the dash
  normal by `R·|sin tilt|` on a housing. Pinned by the cabin test.
- **Warning arc painted, not tinted, on steam gauges.** A printed dial does not change
  colour; the needle entering the red arc is the warning, and the readout goes red.
  Holograms keep the tint.
- **No numerals on the arc scales.** The 1-2-5 ranging makes the majors 113 m/s, 2.5 km,
  1.67 g — nothing a printed numeral would help. The readout is the number; the arc is
  the trend.
- **Horizon drawn under the cabin.** It is at infinity, so the dash should hide it. The
  trajectory already drew that way; this makes the two agree.
- **TRON keeps its beam** but loses the socket recess and ring ("lit sockets under
  holograms must not be visible"); the beam is light in the air, not a cavity.

## Verification (all in `C:\Users\jayja\farfall-captures\gauges\`)

- `r3-default-1.png` — WARTHOG cluster on the dash; `zoom/r3-cluster.png` (2×) shows the
  plates, bezels, tapered pointer, red arcs, `×2` legend, crisp digits.
- `r3-head-1.png` — `FARFALL_BENCH_HEAD=30,-15`: plates and ball seen at an angle, no
  hollow anywhere.
- `r3-tron-1.png`, `r3-jet-1.png`, `r3-dial-1.png` — the other three styles, all cavity-free.
- `r3-g-1.png` — `FARFALL_BENCH_G=1,0.5,2`: G vector dot and 2.29 readout, G meter.
- `r3-landing-1.png` + `zoom/r3-hoops.png` — hoops and the numbered ladder at 3000 m.
- `r3-spin-1..4.png` — the horizon line is occluded by the hull when looking aft/sideways.
- `r3-full` — `FARFALL_BENCH_FULL=1 FARFALL_VSYNC=off`, 1920×1080 4×MSAA: 562 fps avg,
  1% low 270, frame avg 1.78 ms (baseline 1.50 ms; the ladder numerals and plates cost
  ~0.3 ms at 1080p — far inside the 60 fps floor).
- `r4-default-1.png`, `r4-head-1.png` — after the six-socket fix: the G vector has its
  plate and bezel too.

Gate: `cargo test --workspace`, `fmt --check`, `clippy -D warnings`, wasm check — green.
New tests: `cabin.rs::the_ball_stands_proud_of_the_dash_with_no_bowl`, the plate-height
and leaned-edge asserts in `cabin.rs::the_head_turns_the_rays_not_the_cabin`,
`settings.rs::warthog_is_the_default_and_the_menu_still_cycles_every_style`.

## Left for later

- The cabin is marched at `cockpit.res` of the scene; at 800×600 the bezels show the
  march's stair-steps. That is the cabin's resolution, not the instruments'.
- The hoops go pale on a blown-out sky (landing at 3000 m) — they carry radiance > 1 for
  the postfx agent's exposure/tonemap to sort out.
- The DIAL-style gyro (ivory) is still the 2D disc on its plate; only JET/WARTHOG get the
  sphere. Could be a ball too if Jay Jay wants one everywhere.
- The A-10 layout's extra instruments (VSI, AoA, RPM, heading) from the warthog chunk-2
  plan are untouched.
