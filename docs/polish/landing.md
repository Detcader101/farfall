# landing — LANDED state, DISEMBARK hook, a better landing (2026-08-31)

Branch `fable/landing`, worktree `farfall-wt/landing`. Jay Jay's ask: *"make the
landing better, have a feature in the landing to stop it on the ground for
eventual disembark."*

## What was done

### 1. The ship lands (sim — `crates/sim`)

Ground contact used to be restitution + friction only: the ship skidded, slid,
and was kicked into the ground and pushed back out every tick, so it never
truly came to rest. Now `ShipState` carries a `Ground` state:

- `Flight` — clear of every body.
- `Down { body, clean }` — in contact. `clean` is the verdict on the touchdown
  that put it there, judged on the tick it happened: descent under
  `TOUCHDOWN_INTO_MPS` (12 m/s) and the ship's own up within 15° of the surface
  normal. Kept until lift-off.
- `Landed { body, up }` — parked on the gear. Entered from a clean contact once
  the ship slides under 2 m/s with the throttle at idle: the hull is levelled
  (the gear takes the last of the tilt), spin is zeroed, and from then on the
  position is *recomputed* every tick from the body's centre and the stored
  normal (`centre + up × (R + 2.5 m)`), the velocity is the body's own step
  velocity. Exact — a landed ship on the Moon is carried round the planet bit
  for bit (`a_landed_ship_rides_the_ground_exactly_as_the_planet_turns`, tested
  on the Moon because the planet itself is still in this frame).
- Release: any thrust past `RELEASE_THRUST` (0.05), boost, or the hyper field.
  The stick does not (torque on the gear is the gear's). With the throttle up a
  ship never re-settles, so the main engine rolls it off rather than re-parking
  it every tick (that was a real bug caught by `throttle_releases_a_landed_ship`).
- A hard or tilted touchdown is `Down { clean: false }`: it skids as it always
  did and never settles until it lifts off and re-lands. The crash it ought to
  be stays deferred (features.yaml `crash`); it is not faked into a landing.
- Contact now happens at `R + GEAR_HEIGHT_M` (2.5 m) on every body, so the
  pilot's eye is never at ground level and settling moves nothing.

**Golden hash: unchanged** (`0xe8f76101b8054115`). The scenario never touches
the ground, and `state_hash` eats nothing for `Flight`; `Down`/`Landed` add a
tag, the body and the anchor (`the_ground_state_is_in_the_hash`). Two existing
tests that pinned the ship at the bare surface now expect the stance.

### 2. DISEMBARK (`I`, `control.disembark`)

A `Named` bind, on the KEYS page like every other, default `I` (the one letter
the dash did not already answer to; Enter is reserved for the menu). It answers
on the readout for four seconds: `DISEMBARK  NOT YET` when landed, `NOT LANDED`
when down, `LAND FIRST` in flight; the LANDED readout names the key. SPEC §9
M5+ now says the walk-out is that milestone's.

### 3. A better landing (app + render)

- **Readout** (`crates/app/src/landing.rs::lines`): every line fits the 32-glyph
  panel in every state (`every_readout_line_fits_the_panel_in_every_state` —
  the baseline's line was clipped). Approach: `LAND SOFT IN  12S  ALT 340M` /
  `VS -4.2  ALONG 12  GEAR DOWN`, with the cue FLARE (descent past the gear's
  limit, ground within 20 s), LEVEL (tilted past 15°), GEAR DOWN (last 15 s).
  On the ground, mode or no mode: `LANDED ON MOON` / `SOFT  DOWN 1.8  ALONG 0.4
  M/S` / `I DISEMBARK`, or `DOWN ON PLANET  HARD` … `NOT LANDED  LIFT OFF,
  RE-LAND`, or `ROLLING ON PLANET  12 M/S`. The touchdown summary is recorded by
  the app from the sim's before/after on the tick the ground is met
  (`Record::judge`) — presentation memory, not world state.
- **Verdict** now agrees with the gear: HARD above 12 m/s (was 30), FIRM above
  6. The prediction contacts at the gear's height like the sim.
- **Landing pad**: a shader quad in the trajectory pass, flat in the surface's
  tangent plane at the predicted touchdown — outer ring, compass ticks, centre
  dot, and a ring that keeps closing on the centre; verdict colour at radiance
  3.0. 60 m radius at least, growing with distance (2% of range) so it never
  shrinks under about a degree. Setting `landing.pad` / LANDING PAD row.
- **Hoops converge on the pad**: in LANDING mode the hoop grid is phased onto
  the touchdown's path distance (`hoop_phase`, tested), so the last hoop sits on
  the pad and the rest stream toward the ship as it closes. Landing hoops are
  HDR-bright (radiance 2.2) for the postfx bloom.
- **LANDING ASSIST** (`landing.assist`, default on): in LANDING mode with a
  touchdown ahead, pitch/roll demand toward the surface normal, damped, blended
  per axis by the pilot's own input (full stick = no fight); yaw untouched.
  Tuned to ζ ≈ 0.8 on roll — the first cut at KD 0.8 was ζ ≈ 0.2 and still 3°
  out after six seconds; the test caught it.

### 4. Bench knobs

`FARFALL_BENCH_LANDED=1` (parked on the ground, LANDED, Sun up the sky, nose
along the ground) and `FARFALL_BENCH_DISEMBARK=1` (the key pressed at once).
Both in the lib.rs knob list and features.yaml.

## Captures (`farfall-captures/landing/`)

- `approach-1.png` — `FARFALL_BENCH_LAND=1 FARFALL_BENCH_ALT=300`: PENDING — benchmarks halted by Jay Jay (locks/HALT) when this was ready to capture
- `landed-1.png` — `FARFALL_BENCH_LANDED=1`: PENDING (same)
- `disembark-1.png` — `FARFALL_BENCH_LANDED=1 FARFALL_BENCH_DISEMBARK=1`: PENDING (same)

## Decisions

- Ground state in the sim, not the app: only the sim is authority on world
  state, and "stays put through the fixed-step integrator" is only true if the
  integrator itself holds it.
- `Flight` hashes to nothing so the golden could not move. A field that always
  hashed would have moved the constant for a non-physics reason, which CLAUDE.md
  forbids.
- Judged-at-touchdown, kept-until-lift-off: the simplest rule that keeps a hard
  hit from becoming a clean landing a tick later, without a crash model.
- Levelling snaps (≤ 15°) at the settle tick rather than easing — no extra sim
  state, and it reads as the gear compressing.
- The pad comes from the CPU prediction (any body) rather than the shader's own
  integration (planet only); the hoops follow the shader's path, phased to the
  CPU's path length. Off by metres at most; a hoop the shader finds past the
  ground is hidden, the pad marks the point regardless.
- Touchdown speeds for the summary are recorded by the app, not stored in the
  sim: they are memory for the readout, not world state.

## What is left

- No crash: a hard touchdown skids and says DOWN … HARD. When ship loss lands,
  `Down { clean: false }` is where it hooks in.
- The trajectory shader models the planet only; a Moon approach shows the pad
  (CPU) but no hoops along the true path.
- LANDING ASSIST verified by test and reasoning, not flown (the bench is
  frozen). Fly it: G, approach, take hands off pitch/roll, watch it level.
- The readout font/size is the hud-menu agent's; these lines are 32-column
  safe for whatever it ships.
