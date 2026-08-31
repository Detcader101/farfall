# The carrier: dock the jet, walk to the bridge

*Plan, 2026-08-30. The ledger (features.yaml) tracks each chunk; this is the shape.*

## Goal

A second ship in the world — a carrier — with a landing deck the jet can dock on.
Docked, the pilot leaves the seat and walks, first person, through the hangar and
the corridor to the bridge. Same doctrines as everything else: the carrier is drawn
by shader (SDF, ray-marched, no assets), every part ships with a setting and a menu
row, every chunk has tests through the gate and a scene capture looked at, and it
works from the link and in VR (the walker in VR is head look plus stick).

## Chunks

### C1 — The carrier is there
- A body in the world: ~600 m, in a low orbit of the planet (sim-side position from
  a Kepler orbit like the Moon's, deterministic; `WorldParams::carrier`).
- Drawn on the GPU: a new `carrier.wgsl` pass (SDF hull — a long spine, a flight
  deck open at the stern with a lit mouth, superstructure with the bridge glazing
  forward-top) marched like the mimic ships but bounded by a big sphere; specks at
  distance like the far belt; lit by the Sun; nav lights.
- On the MAP and in the WORMHOLE DRIVE panel as a destination (`CARRIER`), so ENGAGE
  drops the jet a few km astern of the deck, matched in velocity, like Uranus does
  for the belt. The readout names it (`CARRIER 4.2 KM`).
- Settings: `world.carrier` (on/off), `world.carrier-orbit-km`. Menu rows on GFX/CABIN.
- Tests: shader validation; sim placement determinism (position at t is a pure
  function); a scene capture from the approach (`FARFALL_BENCH_CARRIER=1`).

### C2 — Docking
- The deck is a landing plane in the carrier's frame, moving with it. `landing.rs`
  gets a second target: the deck (its plane, its edges, its approach corridor).
  LANDING mode's hoops and touchdown prediction work against it; the deck's edge
  lights turn green/amber/red with the predicted touchdown, as the ground does.
- Contact: the sim's contact model gains a moving plane; at rest on it, the jet is
  DOCKED — its state is carried in the carrier's frame (position and velocity
  follow the carrier exactly; no drift, no numerical creep), the readout says
  `DOCKED`, controls are inert except LAUNCH (a Named bind, default `B`... chosen
  so it does not collide with the bay) which pushes off the deck.
- Settings: `dock.assist` (the computer flies the last 200 m if on), the deck's
  hoop spacing. Tests: contact with a moving plane conserves the relative state;
  a docked jet stays put over a long sim run (golden hash); a docking capture.

### C3 — Leaving the seat: the walker
- A first-person mode, entered only when DOCKED (LEAVE SEAT on the readout / a
  Named bind): the camera leaves the cockpit and becomes a walker in the carrier's
  frame — eye height 1.7 m, WASD walk, mouse (or headset) look, gravity along the
  deck's down (the carrier has a spin section or just a chosen "down"; we choose).
- The hangar interior as an SDF twin: floor, walls, gantries, lights, the parked jet
  (the existing fighter SDF, from outside — this is where the SHIP bay's diagram
  came from, now real). Collision on the CPU against a Rust copy of the SDF (the
  cabin already keeps a CPU twin of its shader, the same discipline).
- The cockpit passes stand down; the HUD reduces to the readout (where you are,
  what the carrier is doing). In VR the head is the head; the stick walks.
- Settings: walk speed, look sensitivity (reuse), FOV (reuse), `walker.head-bob`.
  Tests: walker cannot leave the hangar through a wall (CPU SDF); stairs/ramps
  climbable; capture of the hangar with the jet parked.

### C4 — The way to the bridge
- Corridor from the hangar to the bridge: doors that open on approach, a lift or
  stairs, the bridge — a room forward-top with glazing that looks out on the real
  world (the starfield/planet/Sun passes drawn through the window mask, so the
  outside is the same sky the jet flies in).
- The bridge's holograms reuse the instrument passes (the map as a table hologram,
  the readout on a console) — every element on the bridge is one we already draw.
- Tests: the route is walkable end to end (CPU SDF path check); the window shows
  the sky (capture from the bridge, planet in the glass).

### C5 — The bridge does something
- Take the helm: sit at the bridge chair and the carrier itself flies — same sim,
  a heavier body, slower limits, the map and the wormhole drive usable from here.
  Leave the chair, walk back, board the jet, LAUNCH.
- This closes the loop: jet → deck → walk → bridge → fly the carrier → walk → jet.

## Order and use of agents

C1 and C2 first — they are pure extensions of systems that exist (bodies, landing,
contact, warp destinations) and make the carrier a place. C3 is the new system (a
walker with SDF collision); C4 is content on that system; C5 is wiring.

Agents sparingly: one for the carrier hull SDF (C1, a self-contained shader and
its CPU twin with tests), one for the walker collision core (C3), each handed the
doctrines and the ledger rule. Everything else is done in the main line so the
game stays one thing.
