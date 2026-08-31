# hotas — T.Flight HOTAS 4 support and the stick wizard

Branch `fable/hotas`, worktree `farfall-wt/hotas`. Scope: joystick/HOTAS input on
native Windows and the web, axes → controls with deadzone/invert/curve, every
named control bindable to a stick button, a step-by-step setup wizard, docs.

## What was done

- **`crates/app/src/stick.rs`** (new, ~1300 lines with tests): `Sample`
  (8 axes + 32-bit button mask, hat = bits 28–31), `Device` (USB ids → known
  name), `Flight` (PITCH YAW ROLL THROTTLE STRAFE LIFT), `AxisMap`, `StickMap`
  (the device map: axes, deadzone, curve, throttle zero, trigger, a button per
  `Named`; `stick.*` settings parse/render; `body_axes` → the sim's six body
  axes), `shape` (symmetric deadzone + power curve, never NaN), `StickItem`
  (the STICK page's rows), `Wizard` (steps, detection, keys, render), `Reader`
  (poll + edges) and two `platform` modules: winmm on Windows through
  `windows-sys` (already in the tree; feature `Win32_Media_Multimedia`), the
  Gamepad API on wasm through `web-sys`/`js-sys`. Other platforms: a stub.
- **`input.rs`**: `InputState::set_stick` — the stick's six axes add to the
  keys' smoothed axes (no ramp for a physical stick), clamped.
- **`settings.rs`**: `Settings.stick: StickMap`, the `stick.*` keys parsed and
  rendered.
- **`menu.rs`** (kept small for the hud-menu merge): `Page::Stick` ("STICK",
  third tab), `Item::Stick(StickItem)` delegating label/value/adjust to
  stick.rs, `MenuEvent::StickWizard`, `Menu::set_stick` / `rebinding` /
  `stick_button` (a stick button pressed on a PRESS KEY row binds itself), KEYS
  rows show the stick bind beside the key (`LSHIFT L1`, `UP PITCH`), and the
  header now fits six pages in 32 columns (the current page's brackets take
  the place of its spaces, and the GAUGES tab's short name is DIALS — GFX KEYS
  STICK CABIN DIALS ARMS is exactly 32 with the brackets).
- **From the Reforger tool, two more of its ideas** (the lead's note): once the
  stick is identified (`stick.layout`, set by the reader from the USB id) every
  page names the *physical* control — TRIGGER, L1, ROCKER, TWIST — not B0 /
  AXIS 4; and the wizard's summary walks the hardware for coverage:
  `COMPLETE: 21 OF 21 HAVE A JOB` or `… FREE: BASE L HAT-D`, so a control with
  no job is a visible hole. Not mirrored: a separate Audit (RESPONDS/DEAD) and
  Watch mode — the wizard's live bar and detection line do that job in-line.
- **`lib.rs`**: `mod stick`; `Game.{stick, wizard, stick_fire, stick_log}`;
  `App::poll_stick` (once a frame from `RedrawRequested`: axes into the input,
  trigger, button edges as their keys, or into the wizard); **the keyboard
  arm's body moved verbatim into `App::key_input`** so a stick button can be
  the key its control is bound to (one key path — this is the one non-local
  lib.rs change, a pure move plus a wizard check at its top); `render_menu`
  (wizard over the menu); `highlight_row` none under the wizard; the trigger
  ORs `stick_fire`; `FARFALL_BENCH_STICK=n`.
- **Docs**: `docs/HOTAS.md` (design, readers and why winmm, the default map
  as a table, settings keys, the wizard, adding another stick), `features.yaml`
  (`hotas-stick`, the `stick` settings group, the bench knob), README's
  controls paragraph.

## Decisions and why

- **winmm via `windows-sys`, not `gilrs`.** gilrs 0.11 (MIT/Apache-2.0) is WGI
  by default on Windows; WGI sees HID sticks on Windows 10+ but only for a
  focused window, in HID-descriptor axis order, and it brings the `windows`
  crate tree with it. winmm is what `hotas-reforger` measured this exact unit
  through, its X Y Z U V R order is the order the shipped default map is written
  in, `windows-sys` 0.61 is already in `Cargo.lock` (MIT OR Apache-2.0), and the
  binding is two functions. Zero new dependencies, cargo-deny untouched. Cost:
  Windows only natively (Linux/macOS get a stub and a documented road: gilrs
  behind the same `platform::{find, read}` pair).
- **A stick button is its key.** Buttons bind to `Named` controls and the app
  synthesises the bound key's press/release down `App::key_input`. No second
  table of what buttons do; the KEYS page is still the whole truth. The cost
  was moving the keyboard arm's body into a method (a pure move).
- **The wizard walks the stick, not the action list** (the Reforger tool's
  lesson): detection is "the axis that travelled furthest from where it rested
  when the step opened, at least half travel", re-evaluated every frame until
  ENTER, and the direction sets invert — pushing the wrong way and then the
  right way ends right. Buttons already held when a step opens don't count.
- **The wizard sits over the open menu** rather than being a new panel: it
  inherits the pause, the panel anchor, dragging and the text pass with no new
  lib.rs plumbing. `highlight_row` is suppressed under it.
- **The stick bypasses the key ramp.** A lever is its own ramp; smoothing it
  would add 130 ms of lag. Sum with the keys, clamp.
- **Throttle zero: centre by default.** The HOTAS 4 lever has no detent; a
  space sim wants reverse. BOTTOM is a row and a wizard step for a 0..1 lever.
- **Hat = four buttons** (bits 28–31) so it binds through the same path; the
  defaults put LOOK LOCK / RAIL / HOLD / CANNON on it.

## Verification

- `cargo.exe test -p farfall-app`: stick.rs 12 tests, input.rs 1, menu.rs 1
  (see the test log lines in the final report).
- Captures in `farfall-captures/hotas/`, all looked at; every one's `.log`
  carries `stick: found T.FLIGHT HOTAS 4 (044F:B67C, 6 axes, 12 buttons, a
  hat)` — the real stick, read by winmm at start of every run:
  - `stick-page-1.png` — `FARFALL_BENCH_MENU=2`: the STICK page (DEVICE
    T.FLIGHT HOTAS 4 (the real stick, found by winmm), STICK ON,
    SETUP WIZARD, the six axis rows by their physical names, DEADZONE 8%,
    CURVE 1.50, THROTTLE ZERO CENTRE, TRIGGER.
  - `wizard-pitch` — `FARFALL_BENCH_STICK=0`: step 1/37, PITCH: "PULL THE STICK
    BACK (NOSE UP)", DETECTED STICK Y +, the live bar.
  - `wizard-deadzone` — `FARFALL_BENCH_STICK=7`: the DEADZONE knob with the drift bar.
  - `wizard-trigger` — `FARFALL_BENCH_STICK=9`: TRIGGER: DETECTED TRIGGER.
  - `wizard-boost` — `FARFALL_BENCH_STICK=10`: BOOST: DETECTED L1, the key it stands in for.
  - `wizard-summary` — `FARFALL_BENCH_STICK=36`: the map, the trigger, and the coverage line.
  - `keys-page` — `FARFALL_BENCH_MENU=1`: KEYS with the stick binds beside the keys.
  - Post-merge, on the 48-column card: `stick-page-1.png` again (nine tabs,
    ROW 1/15, the LEVER BRAKE / LEVER JUMP rows, the description footer).
  - `landed-rest-1.png` + `.log` — `FARFALL_BENCH_LANDED=1` with the real
    stick at rest: the found-line and ZERO stick movement lines — the
    landing-strafe drift is gone (on a fresh plug-in the log would say
    `stick: axis 4 rests at full deflection - ignored until it moves`).
  - `cabin-stick-neutral-1.png` / `cabin-stick-demand-1.png` —
    `FARFALL_BENCH_HEAD=0,-35`, the latter with
    `FARFALL_BENCH_DEMAND=1,0.7,0.5,0.8`: the console column straight, then
    leaned back-and-right, the grip twisted, the lever slid forward.
  Every wizard page and every STICK/KEYS row is also measured against the
  32×16 panel in tests (`every_wizard_page_fits_the_panel`,
  `the_stick_page_and_its_wizard_are_in_the_menu`,
  `every_row_of_every_page_fits_the_panel`).
  What looking at the captures corrected: the known name lost its
  THRUSTMASTER so the DEVICE row and the FOUND: line fit; the driver's U
  axis (nothing on the unit moves it) no longer counts as a FREE control
  (`COMPLETE: 21 OF 21 HAVE A JOB`); the bar's `|`/`#` and the face
  buttons' `^` were not in the 3×5 font — now `:`/`*` and FACE L D R U.
  One KEYS capture came back with its first row in PRESS KEY (rebinding);
  a rerun with every stick edge logged showed no button edge and a clean
  page — not the stick; a one-off from the bench desktop's focus/click,
  not reproduced since.
- The real device: the stick is plugged in on this PC (Windows PnP shows
  `HID\VID_044F&PID_B67C`, PC mode) and a winmm probe from PowerShell read it as
  id 0, 6 axes, 12 buttons, a hat, all axes at rest (32767) — the same call
  `stick::platform::find/read` makes. The game's own line
  (`stick: found …` at start) shows in all the capture logs. Nobody was at
  the stick during the runs, so no movement line (`stick: pitch … ->
  thrust […] torque […]`, once a second while any axis passes 0.3) and no
  button-edge line (`stick: L1 down`) was produced — those are the first
  things to look for in the log on a real flight.

## What is left / open problems

- Not yet flown by hand: the default map's directions are asserted from the
  measured audit (`hotas-reforger/device-audit.json`) and unit tests, not from a
  flight. First thing to do at the stick: fly, and if anything is backwards, the
  wizard or the STICK page's ENTER (flip) fixes it in seconds.
- The web reader is compiled (wasm check) but not driven in a browser yet;
  Chrome orders a HOTAS 4's axes differently from winmm, so the browser needs a
  wizard run once (its map is in localStorage, separate from the native file).
- Linux/macOS: no reader (stub), documented.
- Merge notes for the orchestrator: lib.rs's keyboard arm moved into
  `App::key_input` (pure move); menu.rs additions are grouped and small
  (`Page::Stick`, `Item::Stick`, header fit); the base branch's
  `crates/render/src/sight.rs` is not fmt-clean (I reverted fmt's touch on it).

## The live flight (2026-08-31, Jay Jay at the stick)

Jay Jay flew the merged build with the real HOTAS 4 mid-pass
(`farfall-captures/live/hotas-live.log`, 408 stick lines). What it proved:
pitch, roll, yaw and their directions ride the stick full-range exactly as the
measured default map says. What it caught: **`strafe +1.00` at rest, for the
whole flight** — winmm reports an axis nothing has touched since plug-in at
full deflection, so the rocker's V axis read as a full strafe demand from the
first frame. Fixed with a calibration gate in `Reader::admit`: an axis that
has only ever read full rail contributes nothing until first seen inside
±0.95, then it is real for good (test
`an_unidentified_axis_resting_at_full_deflection_moves_nothing`; the log says
`stick: axis 4 rests at full deflection - ignored until it moves`). The
throttle inherits the same guard, which also means a lever parked at a rail
cannot slam the ship at spawn.

## The three follow-ons (Jay Jay, mid-pass)

1. **The cabin's stick answers the HOTAS.** The console's control column
   (already in cockpit.wgsl) now rides the live demand: the column leans
   with pitch/roll, the grip twists with yaw, the left lever slides with the
   throttle — from the summed demand (HOTAS or keys), through one new lane
   on the cabin uniforms (`CabinUniforms::with_stick`, quantised 0.05 so a
   settling ramp doesn't re-march the cabin every frame). Setting
   `cockpit.stick` (CABIN page row, described). **cockpit.wgsl touched only
   in the interior console block** — the stick/grip/throttle SDF lines
   inside `map()` (the region commented "The stick between the knees…"),
   plus one `stick: vec4` field appended to the `Cockpit` uniform struct
   after `eye` — for the cockpit-arms agent's merge.
2. **Lever hard back = air brake** (`stick.throttle-brake`, ON, STICK row +
   description): bottom ~5% of the lever's true travel holds the brake;
   centre-zero maps only; the calibration gate keeps an untouched railed
   lever out of it. Test `the_lever_hard_back_holds_the_air_brake`.
3. **Lever slam = 2 s chaos burst** (`stick.throttle-jump`, ON, STICK row +
   description): ≥70% of travel forward inside 250 ms ending past halfway
   holds the chaos drive for exactly two seconds (the same held-state as H;
   charge/entropy rules untouched); a smooth push can never fire it, and
   holding forward does not re-fire. Tests `a_smooth_throttle_push_never_jumps`,
   `a_slam_jumps_for_two_seconds_and_releases`. Both gestures are HELP
   entries under DRIVES (LEVER BACK / LEVER SLAM).

## The merge with fable/polish (74347db, eight branches in)

Done in this worktree, `fable/polish` merged INTO `fable/hotas`, per the lead.
What was reconciled:

- **The STICK page is the ninth tab** of the new 48-column card menu — GFX
  KEYS STICK CABIN DIALS ARMS MAP SHIP HELP (45 columns of header, fits).
  Every stick row gained a `describe()` sentence for the card's footer and a
  `keys()` claim for the settings ledger (`every_settings_key_has_a_menu_row`
  holds: `stick.*` keys are in `settings::KEYS`; a KEYS-page named row claims
  `control.<n>` *and* `stick.button.<n>`; the DEVICE row claims
  `stick.layout`, which the reader sets).
- **`App::key_input` carries fable/polish's whole keyboard arm** (the CONTROLS
  card close-on-any-key and F1, the new HOLO WIDER/CLOSER binds, DISEMBARK)
  with the wizard's hook after the card's — so a stick button still IS its
  key, and one bound to any of the new controls just works. The arm in
  `window_event` stays one line.
- **The wizard renders in the card's shape** — 48 columns, 16 lines, the
  5×7 font's `draw_line` — in the menu's place, inheriting the new panel.
  Three new named controls make it 37 steps.
- Their side kept: the trigger gained `stick_fire` back; the bench doc lists
  `FARFALL_BENCH_MENU` 0..8 (2 = STICK), `_STICK` and `_CARD` together;
  README's controls paragraph carries both the HOTAS sentence and the new
  WARTHOG gauge text; `edits_round_trip` now tweaks the stick block too.
